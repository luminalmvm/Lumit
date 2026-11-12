//! The point track.

use super::*;

/// Two textured squares on a flat background, each sliding its **own** way.
/// Deliberately not [`PlaneShot`]: the whole claim of a point track is that
/// the two things it follows need not be on one surface, and a fixture where
/// they were would prove nothing the planar test does not already.
struct PatchShot;

impl PatchShot {
    const FRAMES: usize = 12;
    /// Half a patch's side.
    const HALF: f64 = 26.0;
    const P1: [f64; 2] = [110.0, 90.0];
    const P2: [f64; 2] = [290.0, 200.0];

    /// Where each patch's centre is on frame `n` — two unrelated slides.
    fn centres(n: usize) -> [[f64; 2]; 2] {
        let d = n as f64;
        [
            [Self::P1[0] + 1.8 * d, Self::P1[1] + 0.4 * d],
            [Self::P2[0] - 1.1 * d, Self::P2[1] + 1.6 * d],
        ]
    }
}

impl LumaFrames for PatchShot {
    fn info(&self) -> (usize, u32, u32, f64) {
        (Self::FRAMES, W as u32, H as u32, FPS)
    }

    fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
        if n >= Self::FRAMES {
            return None;
        }
        let now = Self::centres(n);
        let home = Self::centres(0);
        let mut out = vec![0.35f32; W * H];
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f64, y as f64);
                for (c, r) in now.iter().zip(home) {
                    if (fx - c[0]).abs() <= Self::HALF && (fy - c[1]).abs() <= Self::HALF {
                        // Sampled in the patch's own frame-0 coordinates, so
                        // each carries its texture along with it.
                        out[y * W + x] = texture(fx - c[0] + r[0], fy - c[1] + r[1]);
                    }
                }
            }
        }
        Some(out)
    }
}

fn point_job(track: Uuid, tag: &str, two: bool) -> Job {
    let home = PatchShot::centres(0);
    let regions = PointRegions {
        points: home,
        two,
        radius: 40.0,
    };
    let settings = AnalysisSettings {
        follow: if two {
            lumit_core::fx::effects::planar_track::FOLLOW_TWO_POINTS
        } else {
            lumit_core::fx::effects::planar_track::FOLLOW_ONE_POINT
        },
        ..AnalysisSettings::default()
    };
    let masks = MaskTrack::default().within_all(regions.outlines());
    Job {
        media: track,
        key: Some(AnalysisKey::new(&fingerprint(tag), settings, &masks)),
        settings,
        kind: JobKind::Points { regions },
        masks,
        open: Box::new(|| Some(Box::new(PatchShot) as Box<dyn LumaFrames>)),
        analyse: true,
        range: None,
    }
}

/// The whole point path, end to end: a real analysis of two independently
/// moving patches, the reported box against the similarity the render
/// actually applied, and the answer reaching the store.
#[test]
fn a_two_point_analysis_follows_both_patches_and_reaches_the_store() {
    let _serial = serially();
    let dir = tempfile::tempdir().unwrap();
    with_cache(dir.path());

    let effect = Uuid::now_v7();
    let cancel = AtomicBool::new(false);
    let (out, _) = run_here_answer(point_job(effect, "points", true), &cancel);
    let (fps, clip_frames, answer) = out.expect("two textured patches are followable");
    assert_eq!(fps, FPS);
    assert_eq!(clip_frames, PatchShot::FRAMES);
    let Answer::Planar(track) = answer else {
        panic!("a point job answers with a track");
    };
    assert_eq!(
        track.frames.len(),
        PatchShot::FRAMES,
        "every frame was followed"
    );
    assert_eq!(track.reanchors, 0, "nothing thinned over twelve frames");

    // The truth is the similarity taking the two reference centres to where
    // the fixture actually put them — an exact formula, not a measurement.
    let home = PatchShot::centres(0);
    let mut worst = 0.0f64;
    for f in &track.frames {
        let now = PatchShot::centres(f.frame as usize);
        let (ax, ay) = (home[1][0] - home[0][0], home[1][1] - home[0][1]);
        let (bx, by) = (now[1][0] - now[0][0], now[1][1] - now[0][1]);
        let d = ax * ax + ay * ay;
        let (wr, wi) = ((bx * ax + by * ay) / d, (by * ax - bx * ay) / d);
        for (got, corner) in f.corners.iter().zip(track.reference_quad) {
            let (ex, ey) = (corner[0] - home[0][0], corner[1] - home[0][1]);
            let want = [now[0][0] + wr * ex - wi * ey, now[0][1] + wi * ex + wr * ey];
            worst = worst.max((got[0] - want[0]).hypot(got[1] - want[1]));
        }
    }
    assert!(worst < 2.0, "worst corner error {worst} px");

    // And into the store, under the effect instance, readable the way the
    // Corner pin and the transform keys read it.
    publish_planar(effect, fps, clip_frames, *track);
    let range = Store.planar_range(effect).expect("filed under the effect");
    assert_eq!(range.first_frame, 0);
    assert_eq!(
        range.last_frame,
        PatchShot::FRAMES as i64 - 1,
        "the whole clip was followed"
    );
    assert!(Store.planar_corners(effect, 5).is_some());
    clear();
}
