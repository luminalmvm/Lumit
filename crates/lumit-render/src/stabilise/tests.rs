//! The analysis, end to end, on a clip with a shake written down.

use super::*;
use lumit_core::stabilise::frame::{Edges, Framing};
use lumit_core::stabilise::motion::apply;
use lumit_core::stabilise::path::Model;
use lumit_core::stabilise::plan::{build, PlanRows, Smoothness};

/// A picture with corners everywhere: overlapping blobs of different sizes and
/// brightnesses on a soft ramp, the same at every frame, seen through a camera
/// that slides and turns by the frame's shake.
struct Shaken {
    frames: usize,
    w: u32,
    h: u32,
    /// Per frame: the picture's slide in x and y and its turn in radians about
    /// the centre, as the frame shows it.
    shake: Vec<[f64; 3]>,
}

fn hash(i: u32) -> f64 {
    let mut z = u64::from(i).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (z >> 11) as f64 / (1u64 << 53) as f64
}

impl Shaken {
    fn scene(x: f64, y: f64) -> f32 {
        let mut v = 0.3 + 0.2 * (x / 400.0) + 0.1 * (y / 300.0);
        for i in 0..90u32 {
            let (bx, by) = (hash(3 * i) * 520.0 - 40.0, hash(3 * i + 1) * 400.0 - 40.0);
            let r = 3.0 + hash(3 * i + 2) * 9.0;
            let d2 = (x - bx).powi(2) + (y - by).powi(2);
            v += 0.35 * (if i % 2 == 0 { 1.0 } else { -1.0 }) * (-d2 / (r * r)).exp();
        }
        v.clamp(0.0, 1.0) as f32
    }

    /// Where frame `n` shows the scene point that frame `n`'s pixel `p` is.
    fn source_of(&self, n: usize, p: [f64; 2]) -> [f64; 2] {
        let [dx, dy, a] = self.shake[n];
        let (cx, cy) = (f64::from(self.w) / 2.0, f64::from(self.h) / 2.0);
        // The frame shows the scene slid by (dx, dy) and turned by a: pixel p
        // shows the scene point that slide and turn took to p.
        let (qx, qy) = (p[0] - cx - dx, p[1] - cy - dy);
        let (c, s) = (a.cos(), a.sin());
        [cx + c * qx + s * qy, cy - s * qx + c * qy]
    }

    /// Where frame `n` shows the scene point frame 0 shows at pixel `p`.
    fn truth(&self, n: usize, p: [f64; 2]) -> [f64; 2] {
        let scene = self.source_of(0, p);
        let [dx, dy, a] = self.shake[n];
        let (cx, cy) = (f64::from(self.w) / 2.0, f64::from(self.h) / 2.0);
        let (qx, qy) = (scene[0] - cx, scene[1] - cy);
        let (c, s) = (a.cos(), a.sin());
        [cx + c * qx - s * qy + dx, cy + s * qx + c * qy + dy]
    }
}

impl LumaFrames for Shaken {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.w, self.h, 30.0)
    }

    fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
        let mut out = Vec::with_capacity((self.w * self.h) as usize);
        for y in 0..self.h {
            for x in 0..self.w {
                let s = self.source_of(n, [f64::from(x) + 0.5, f64::from(y) + 0.5]);
                out.push(Self::scene(s[0], s[1]));
            }
        }
        (n < self.frames).then_some(out)
    }
}

fn shaken(frames: usize) -> Shaken {
    let shake = (0..frames)
        .map(|n| {
            let t = n as f64;
            [
                6.0 * (t * 0.9).sin() + 3.0 * (t * 2.3).cos(),
                4.0 * (t * 1.3).cos() - 2.0 * (t * 3.1).sin(),
                0.01 * (t * 0.7).sin(),
            ]
        })
        .collect();
    Shaken {
        frames,
        w: 480,
        h: 320,
        shake,
    }
}

fn job(frames: usize) -> (Job, Shaken) {
    let clip = shaken(frames);
    let copy = Shaken {
        frames,
        w: clip.w,
        h: clip.h,
        shake: clip.shake.clone(),
    };
    let job = Job {
        effect: Uuid::nil(),
        source: Uuid::nil(),
        key: [0; 32],
        on_disk: false,
        settings: AnalysisSettings::default(),
        masks: MaskTrack::default(),
        open: Box::new(move || Some(Box::new(copy) as Box<dyn LumaFrames>)),
        fresh: true,
        range: None,
        rate: None,
        rate_scale: 1.0,
    };
    (job, clip)
}

/// Locked to the first frame, every frame's map puts each pixel where the
/// written-down shake moved it, to a third of a pixel, for the turn-aware fits.
#[test]
fn no_motion_undoes_a_written_down_shake() {
    let (job, clip) = job(40);
    let cancel = Arc::new(AtomicBool::new(false));
    let (found, whole) = analyse(job, &cancel, &|_| {}).unwrap();
    assert!(whole, "every frame was read");
    assert_eq!(found.frames, 40);
    assert!(found.steps.iter().all(|s| s.trusted() && !s.cut));
    for model in [Model::Similarity, Model::Perspective] {
        let rows = PlanRows {
            model,
            lock: Some(0),
            smoothing: Smoothness::Even(0.0),
            keep_scale: false,
            framing: Framing {
                edges: Edges::Show,
                max_scale: 1.5,
                margin: 0.0,
                smooth_more: 1.0,
                fps: 30.0,
            },
            shown: (0, 39),
            dropped: Vec::new(),
        };
        let plan = build(&found, 1, &rows);
        let mut worst = 0.0f64;
        for n in 0..40 {
            for p in [
                [120.0, 80.0],
                [360.0, 80.0],
                [240.0, 160.0],
                [120.0, 240.0],
                [360.0, 240.0],
            ] {
                let got = apply(&plan.maps[n], p).unwrap();
                let want = clip.truth(n, p);
                worst = worst.max((got[0] - want[0]).hypot(got[1] - want[1]));
            }
        }
        assert!(worst < 0.33, "{model:?}: worst {worst} px");
    }
}

/// Cancel stops a run between frames and files nothing.
#[test]
fn cancel_stops_an_analysis_between_frames() {
    let (job, _) = job(60);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let result = analyse(job, &cancel, &move |step| {
        if let Progress::Tracking { done, .. } = step {
            if done >= 8 {
                flag.store(true, Ordering::Relaxed);
            }
        }
    });
    assert_eq!(result.err(), Some(AnalysisError::Cancelled));
}

/// A sidecar that parses but does not hold together is read as no file: one
/// cut short, and one with no frames, which a plan made anyway draws nothing
/// from rather than panicking.
#[test]
fn a_truncated_or_empty_sidecar_is_read_as_none() {
    let (job, _) = job(12);
    let cancel = Arc::new(AtomicBool::new(false));
    let (good, _) = analyse(job, &cancel, &|_| {}).unwrap();
    let dir = std::env::temp_dir().join(format!("lumit-stabilise-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = [7u8; 32];
    write_sidecar(&dir, &key, &good);
    assert_eq!(read_sidecar(&dir, &key).as_ref(), Some(&good));
    let mut cut = good.clone();
    cut.steps.truncate(3);
    write_sidecar(&dir, &key, &cut);
    assert!(read_sidecar(&dir, &key).is_none(), "cut short");
    let mut empty = good.clone();
    empty.frames = 0;
    empty.steps.clear();
    empty.runs.clear();
    write_sidecar(&dir, &key, &empty);
    assert!(read_sidecar(&dir, &key).is_none(), "no frames");
    let _ = std::fs::remove_dir_all(&dir);
    let rows = PlanRows {
        model: Model::Perspective,
        lock: Some(5),
        smoothing: Smoothness::Even(0.0),
        keep_scale: false,
        framing: Framing {
            edges: Edges::CropAndScale,
            max_scale: 1.5,
            margin: 0.0,
            smooth_more: 1.0,
            fps: 30.0,
        },
        shown: (0, 0),
        dropped: Vec::new(),
    };
    assert_eq!(build(&empty, 1, &rows).frames, 0);
}
