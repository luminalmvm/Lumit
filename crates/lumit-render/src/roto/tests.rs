//! The propagation job, the `roto/` tier and the store, asserted against a
//! **synthetic shot with a known matte** (docs/impl/roto.md §10) — a bright disc
//! translating over textured ground, rendered by the test so every claim is an
//! assertion rather than a look.
//!
//! No encoder and no asset: the frames arrive through [`RotoFrames`], which is
//! the seam `LumaFrames` already is on the tracking side.

use super::*;
use lumit_core::roto::{RotoStroke as DocStroke, RotoStrokeKind};

/// The shot: a 96×72 frame, spatially smooth texture, a bright disc that moves
/// four pixels a frame. Smooth texture on purpose — γ prices *every* colour
/// step, so per-pixel noise would make the walk crawl (docs/impl/roto.md §12).
struct Disc {
    frames: usize,
    width: u32,
    height: u32,
}

const W: u32 = 96;
const H: u32 = 72;
const R: f32 = 14.0;

impl Disc {
    fn new(frames: usize) -> Self {
        Disc {
            frames,
            width: W,
            height: H,
        }
    }

    /// Where the disc's centre is on frame `n`.
    fn centre(n: i64) -> (f32, f32) {
        (24.0 + 4.0 * n as f32, 36.0)
    }

    /// The matte the test knows to be right.
    fn truth(n: i64) -> Vec<bool> {
        let (cx, cy) = Self::centre(n);
        (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .map(|(x, y)| {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                dx * dx + dy * dy <= R * R
            })
            .collect()
    }
}

impl RotoFrames for Disc {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.width, self.height, 24.0)
    }

    fn rgba(&mut self, n: usize) -> Option<Vec<u8>> {
        if n >= self.frames {
            return None;
        }
        let inside = Disc::truth(n as i64);
        let mut out = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                // A slow ramp both ways: texture the flow can lock on to, with
                // no step anywhere for the geodesic walk to trip over.
                let ground = 40 + ((x / 8 + y / 8) % 3) as u8 * 6;
                let (r, g, b) = if inside[(y * W + x) as usize] {
                    (235u8, 225u8, 210u8)
                } else {
                    (ground, ground + 4, ground + 8)
                };
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Some(out)
    }
}

fn stroke(frame: i64, kind: RotoStrokeKind, from: (f32, f32), to: (f32, f32)) -> DocStroke {
    DocStroke {
        id: uuid::Uuid::now_v7(),
        points: vec![from, to],
        radius: 2.0,
        kind,
        frame,
    }
}

/// A block that cuts the disc out on frame `base`: one stroke through it, and
/// the border ring answering for the background.
fn block_at(base: i64) -> RotoBlock {
    let (cx, cy) = Disc::centre(base);
    RotoBlock {
        base_frame: Some(base),
        strokes: vec![stroke(
            base,
            RotoStrokeKind::Foreground,
            (cx - 6.0, cy),
            (cx + 6.0, cy),
        )],
    }
}

fn job(block: RotoBlock, frames: usize) -> RotoJob {
    RotoJob {
        instance: uuid::Uuid::now_v7(),
        key: None,
        settings: RotoSettings::default(),
        block,
        open: Box::new(move || Some(Box::new(Disc::new(frames)) as Box<dyn RotoFrames>)),
        propagate: true,
    }
}

/// Intersection over union of a stored matte against the analytic disc.
fn iou(run: &RotoRun, frame: i64) -> f64 {
    let truth = Disc::truth(frame);
    let plane = run.matte(frame).expect("a matte for this frame");
    let (mut inter, mut union) = (0usize, 0usize);
    for (i, &t) in truth.iter().enumerate() {
        let got = plane.get(i).copied().unwrap_or(0) > 127;
        if t && got {
            inter += 1;
        }
        if t || got {
            union += 1;
        }
    }
    inter as f64 / union.max(1) as f64
}

/// Tests share one process-wide cache override and one running slot, so they
/// take a lock rather than racing each other — `crate::track`'s arrangement.
fn serially() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// §10 item 1, in its cross-crate form: the base frame's own solve, through the
/// real job's decode-and-convert path rather than the crate's test harness.
#[test]
fn the_base_frame_is_cut_from_its_own_strokes() {
    let _guard = serially();
    set_test_cache_dir(None);
    let (run, cancelled) = propagate(job(block_at(0), 1), &never(), &|_| {}).expect("a run");
    assert!(!cancelled);
    assert_eq!((run.first_frame, run.last_frame), (0, 0));
    assert!(iou(&run, 0) >= 0.95, "base IoU {}", iou(&run, 0));
}

/// §10 item 2: the disc translating, strokes on the base only, both directions
/// from a base in the middle.
#[test]
fn the_matte_is_carried_both_ways_from_the_base() {
    let _guard = serially();
    set_test_cache_dir(None);
    let frames = 9;
    let (run, _) = propagate(job(block_at(4), frames), &never(), &|_| {}).expect("a run");
    assert_eq!((run.first_frame, run.last_frame), (0, 8));
    for f in 0..frames as i64 {
        let got = iou(&run, f);
        assert!(got >= 0.85, "frame {f} IoU {got}");
    }
}

/// §10 item 7's first half, and §5's honesty rule: a frame outside the
/// propagated span has **no matte at all**, so the effect passes through rather
/// than holding a neighbour's answer.
#[test]
fn outside_the_span_there_is_no_matte_to_hold() {
    let _guard = serially();
    set_test_cache_dir(None);
    let (run, _) = propagate(job(block_at(0), 3), &never(), &|_| {}).expect("a run");
    assert!(run.matte(0).is_some());
    assert!(run.matte(2).is_some());
    assert!(run.matte(3).is_none(), "a frame past the span has no matte");
    assert!(run.matte(-1).is_none());
}

/// §10 item 4: a correction leaves the frames between it and the base
/// **byte-identical**, and re-solving copies them rather than solving them —
/// asserted by counting solves, never by timing.
#[test]
fn a_correction_reuses_the_prefix_it_did_not_touch() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    set_test_cache_dir(Some(dir.path().to_path_buf()));

    let fingerprint = lumit_core::model::Fingerprint {
        size: 4096,
        head_tail_hash: "roto-test".into(),
        mtime_secs: 0,
    };
    let frames = 8;
    let first = block_at(0);
    let key = RotoKey::new(&fingerprint, &first, RotoSettings::default());
    let mut j = job(first.clone(), frames);
    j.key = Some(key);
    let (run, _) = propagate(j, &never(), &|_| {}).expect("a run");
    write_sidecar(dir.path(), key, &run);
    let before: Vec<Vec<u8>> = (0..5)
        .map(|f| run.matte(f).expect("a matte").to_vec())
        .collect();

    // A correction at frame 5: frames 0..4 cannot depend on it.
    let mut second = first.clone();
    let (cx, cy) = Disc::centre(5);
    second.strokes.push(stroke(
        5,
        RotoStrokeKind::Foreground,
        (cx - 2.0, cy - 2.0),
        (cx + 2.0, cy + 2.0),
    ));
    let key2 = RotoKey::new(&fingerprint, &second, RotoSettings::default());
    assert_ne!(
        key.file_name(),
        key2.file_name(),
        "a new table is a new run"
    );
    let mut j2 = job(second, frames);
    j2.key = Some(key2);
    let last = std::sync::Mutex::new(Progress::Queued);
    let (run2, _) = propagate(j2, &never(), &|p| {
        if let Ok(mut held) = last.lock() {
            *held = p;
        }
    })
    .expect("a run");
    let last = last.into_inner().expect("the reporter never panicked");

    // Six frames were copied: the base and 1..4 forward, which the correction
    // cannot reach, and nothing else. Counting, not timing (§5).
    let Progress::Solving { reused, .. } = last else {
        panic!("the run never reported progress");
    };
    assert_eq!(
        reused, 5,
        "exactly the frames before the correction are copied"
    );

    for (f, want) in before.iter().enumerate() {
        let got = run2.matte(f as i64).expect("a matte");
        assert_eq!(
            &got[..],
            &want[..],
            "frame {f} moved, and nothing it depends on changed"
        );
    }
}

/// §10 item 6: the sidecar's whole contract — round trip, a rebuild identical
/// to the hit, a wrong key refused, a newer version refused, a deleted file
/// rebuilt to the identical bytes.
#[test]
fn the_sidecar_round_trips_and_refuses_what_it_cannot_vouch_for() {
    let _guard = serially();
    let dir = tempfile::tempdir().expect("a temp dir");
    set_test_cache_dir(Some(dir.path().to_path_buf()));

    let fingerprint = lumit_core::model::Fingerprint {
        size: 512,
        head_tail_hash: "roto-sidecar".into(),
        mtime_secs: 0,
    };
    let block = block_at(0);
    let key = RotoKey::new(&fingerprint, &block, RotoSettings::default());
    let mut j = job(block.clone(), 4);
    j.key = Some(key);
    let (run, _) = propagate(j, &never(), &|_| {}).expect("a run");
    let bytes = encode(key, &run).expect("encodes");

    // Round trip.
    let back = decode(&bytes, Some(key)).expect("decodes");
    assert_eq!(back.frames, run.records);

    // A wrong key is refused rather than believed.
    let other = RotoKey::new(&fingerprint, &block_at(1), RotoSettings::default());
    assert!(decode(&bytes, Some(other)).is_none());

    // A version from the future is refused before the body is parsed.
    let mut newer = bytes.clone();
    newer[7] = FORMAT_VERSION.saturating_add(1) as u8;
    assert!(decode(&newer, Some(key)).is_none());

    // A file that is not one of ours never reaches a deserialiser.
    let mut alien = bytes.clone();
    alien[0] = b'X';
    assert!(decode(&alien, Some(key)).is_none());

    // Delete-safe, and the rebuild is byte-identical to what was deleted: the
    // whole determinism claim, asserted rather than assumed (§8).
    write_sidecar(dir.path(), key, &run);
    let path = dir.path().join(key.file_name());
    assert!(path.exists());
    std::fs::remove_file(&path).expect("removes");
    assert!(read_sidecar(dir.path(), key).is_none());
    let mut again = job(block, 4);
    again.key = Some(key);
    let (rebuilt, _) = propagate(again, &never(), &|_| {}).expect("a run");
    assert_eq!(
        encode(key, &rebuilt).expect("encodes"),
        bytes,
        "a rebuild is byte-identical to the file it replaces"
    );
}

/// §10 item 8, and §6's fifth step: a cancel **finalises rather than discards**.
/// The frames already solved are kept, correctly named, and the span says how
/// far it got.
#[test]
fn a_cancel_keeps_the_prefix_it_finished() {
    let _guard = serially();
    set_test_cache_dir(None);
    // Raised from the start: the base frame is solved before the loop reads the
    // flag, so the run keeps exactly that one frame and stops.
    let flag = AtomicBool::new(true);
    let (run, cancelled) = propagate(job(block_at(2), 8), &flag, &|_| {}).expect("a run");
    assert!(cancelled);
    assert_eq!((run.first_frame, run.last_frame), (2, 2));
    assert!(run.matte(2).is_some(), "the finished frame was kept");
    assert!(run.matte(3).is_none(), "nothing was invented past it");
    assert!(run.is_partial());
}

/// §10 item 8's refusals, each produced and each named.
#[test]
fn every_refusal_has_a_name_and_none_is_a_fault() {
    let _guard = serially();
    set_test_cache_dir(None);

    // No base frame: refused before a thread is spawned.
    let mut j = job(RotoBlock::default(), 2);
    j.key = Some(RotoKey::new(
        &lumit_core::model::Fingerprint {
            size: 1,
            head_tail_hash: "x".into(),
            mtime_secs: 0,
        },
        &RotoBlock::default(),
        RotoSettings::default(),
    ));
    assert_eq!(
        request(j),
        Requested::Refused(RotoFailure::NoBaseFrame),
        "Propagate before any stroke is a refusal, not a guess"
    );

    // Offline: no fingerprint, so nothing to key a cache with.
    assert_eq!(
        request(job(block_at(0), 2)),
        Requested::Refused(RotoFailure::Offline)
    );

    // Unreadable: the frames would not open.
    let mut j = job(block_at(0), 2);
    j.open = Box::new(|| None);
    assert_eq!(
        propagate(j, &never(), &|_| {}).unwrap_err(),
        RotoFailure::Unreadable
    );
}

/// The store's own contract: a published run answers by frame, the warm cache
/// hands back the same plane, and the per-frame read stays inside the 1 ms bound
/// the render path is budgeted at (§7, docs/13).
#[test]
fn the_store_answers_one_frame_quickly_and_forgets_on_clear() {
    let _guard = serially();
    set_test_cache_dir(None);
    let instance = uuid::Uuid::now_v7();
    let chain = [7u8; 32];
    let plane: Vec<u8> = (0..(W * H)).map(|i| (i % 251) as u8).collect();
    let run = run_from_planes(W, H, 24.0, 40, &[(3, chain, plane.clone())]).expect("a run");
    publish(instance, run);

    assert_eq!(span(instance), Some((3, 3)));
    assert_eq!(stored_chain(instance, 3), Some(chain));
    let (w, h, got) = matte(instance, 3).expect("a matte");
    assert_eq!((w, h), (W, H));
    assert_eq!(&got[..], &plane[..], "the plane round-trips through LZ4");
    assert!(matte(instance, 4).is_none());

    // Budgeted, not measured for a headline: a hundred reads of one frame, the
    // shape the frame walk makes, well inside a millisecond each.
    let started = std::time::Instant::now();
    for _ in 0..100 {
        assert!(matte(instance, 3).is_some());
    }
    let each = started.elapsed().as_secs_f64() * 1000.0 / 100.0;
    assert!(each < 1.0, "a store read took {each:.3} ms, budget is 1 ms");

    clear();
    assert!(propagated(instance).is_none());
}

/// §10 item 9, `--ignored`: the per-frame propagation cost at the note's own
/// target, printed rather than gated until the numbers are real.
#[test]
#[ignore = "perf measurement, not a gate (docs/impl/roto.md §7)"]
fn propagation_cost_per_frame() {
    let _guard = serially();
    set_test_cache_dir(None);
    let frames = 12;
    let started = std::time::Instant::now();
    let (run, _) = propagate(job(block_at(0), frames), &never(), &|_| {}).expect("a run");
    let ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!(
        "roto propagate: {ms:.1} ms/frame at {}×{} ({} frames); the §7 target is 60 ms at 1080p",
        run.width, run.height, frames
    );
}
