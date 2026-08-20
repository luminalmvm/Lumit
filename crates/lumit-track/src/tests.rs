//! Phase-1 test plan (docs/impl/tracking.md §5): synthetic first, real second.
//!
//! Everything here is generated in-test from a deterministic procedural
//! texture — no assets, no fixtures, no random seeds — so the ground truth is
//! an exact formula rather than an eyeballed number, and the tests mean the same
//! thing on every machine.

use super::*;
use lumit_core::mask::{flatten_path, Mask, MASK_PATH_TOLERANCE_PX};

// --- The synthetic world ---------------------------------------------------

/// A deterministic integer hash in 0..1. Splitmix-style finaliser over the two
/// lattice coordinates: no state, no seed, identical everywhere.
fn hash2(ix: i64, iy: i64) -> f64 {
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    ((h >> 11) as f64) / ((1u64 << 53) as f64)
}

/// Smooth value noise on a unit lattice.
fn noise(x: f64, y: f64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let (ix, iy) = (x0 as i64, y0 as i64);
    let a = hash2(ix, iy);
    let b = hash2(ix + 1, iy);
    let c = hash2(ix, iy + 1);
    let d = hash2(ix + 1, iy + 1);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// The procedural texture the synthetic sequences are painted with: three
/// octaves of value noise, rich enough in corners for Shi–Tomasi and smooth
/// enough for a gradient-based solve.
fn texture(x: f64, y: f64) -> f32 {
    let v = 0.20
        + 0.34 * noise(x / 17.0, y / 17.0)
        + 0.22 * noise(x / 7.0, y / 7.0)
        + 0.14 * noise(x / 3.0, y / 3.0);
    v.clamp(0.0, 1.0) as f32
}

/// An affine camera motion about a centre: a point that sat at `p` on frame 0
/// sits at [`Motion::fwd`]`(p)` on this frame.
#[derive(Clone, Copy)]
struct Motion {
    m: [[f64; 2]; 2],
    t: [f64; 2],
    c: [f64; 2],
}

impl Motion {
    fn identity(c: [f64; 2]) -> Self {
        Motion {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: [0.0, 0.0],
            c,
        }
    }

    fn translate(c: [f64; 2], dx: f64, dy: f64) -> Self {
        Motion {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: [dx, dy],
            c,
        }
    }

    fn rotate(c: [f64; 2], radians: f64) -> Self {
        let (s, k) = (radians.sin(), radians.cos());
        Motion {
            m: [[k, -s], [s, k]],
            t: [0.0, 0.0],
            c,
        }
    }

    fn scale(c: [f64; 2], s: f64) -> Self {
        Motion {
            m: [[s, 0.0], [0.0, s]],
            t: [0.0, 0.0],
            c,
        }
    }

    fn shear(c: [f64; 2], k: f64) -> Self {
        Motion {
            m: [[1.0, k], [0.35 * k, 1.0 - 0.5 * k]],
            t: [0.6, -0.4],
            c,
        }
    }

    fn fwd(&self, p: [f64; 2]) -> [f64; 2] {
        let (x, y) = (p[0] - self.c[0], p[1] - self.c[1]);
        [
            self.c[0] + self.t[0] + self.m[0][0] * x + self.m[0][1] * y,
            self.c[1] + self.t[1] + self.m[1][0] * x + self.m[1][1] * y,
        ]
    }

    fn inv(&self, p: [f64; 2]) -> [f64; 2] {
        let det = self.m[0][0] * self.m[1][1] - self.m[0][1] * self.m[1][0];
        let (x, y) = (p[0] - self.c[0] - self.t[0], p[1] - self.c[1] - self.t[1]);
        [
            self.c[0] + (self.m[1][1] * x - self.m[0][1] * y) / det,
            self.c[1] + (-self.m[1][0] * x + self.m[0][0] * y) / det,
        ]
    }
}

const W: usize = 320;
const H: usize = 180;
/// The textured quad, in frame-0 coordinates. Everything outside it is flat, so
/// no feature is ever born there — the quad is the whole trackable world.
const QUAD: (f64, f64, f64, f64) = (34.0, 22.0, 286.0, 158.0);

fn centre() -> [f64; 2] {
    [W as f64 / 2.0, H as f64 / 2.0]
}

/// Render one frame of "a textured quad under `motion`", optionally with a
/// solid occluder rectangle painted over it in *image* coordinates.
fn render(motion: &Motion, occluder: Option<(f64, f64, f64, f64)>) -> Vec<f32> {
    let mut out = vec![0.35f32; W * H];
    for y in 0..H {
        for x in 0..W {
            let p = motion.inv([x as f64, y as f64]);
            if p[0] >= QUAD.0 && p[0] <= QUAD.2 && p[1] >= QUAD.1 && p[1] <= QUAD.3 {
                out[y * W + x] = texture(p[0], p[1]);
            }
            if let Some((ox0, oy0, ox1, oy1)) = occluder {
                let (fx, fy) = (x as f64, y as f64);
                if fx >= ox0 && fx <= ox1 && fy >= oy0 && fy <= oy1 {
                    // Not flat: a flat occluder would be rejected by NCC's
                    // zero-variance rule for the wrong reason. This one has real
                    // detail that simply is not the detail underneath it.
                    out[y * W + x] = 0.15 + 0.5 * texture(fx * 2.0 + 900.0, fy * 2.0 - 700.0);
                }
            }
        }
    }
    out
}

/// Run a whole sequence of motions and hand back the track set.
fn run(motions: &[Motion], settings: TrackSettings, masks: Vec<ExclusionMask>) -> TrackSet {
    let mut tracker = Tracker::new(settings).with_masks(masks);
    for (i, m) in motions.iter().enumerate() {
        let f = render(m, None);
        let plane = FramePlane::new(&f, W, H).unwrap();
        tracker.push(i as i64, plane, None).unwrap();
    }
    tracker.finish()
}

/// Per-point error of every *followed* track point against the analytic ground
/// truth: a track born at `p` on frame `f0` must sit at `M_f(M_f0⁻¹(p))` on
/// frame `f`.
///
/// The birth point itself is deliberately excluded. Its error is exactly zero by
/// construction — `M_f0(M_f0⁻¹(p)) = p` for any `p` at all — so counting it
/// measures the tracker's arithmetic not at all, and enough of them drown the
/// samples that do. That is not a hypothetical: a tracker so broken that every
/// track dies after one frame still produces a set of one-point tracks whose
/// errors are all zero, and every accuracy threshold below would pass on it.
/// With the birth points dropped, such a set yields no samples and
/// [`median`]/[`quantile`] fail loudly instead.
fn errors(set: &TrackSet, motions: &[Motion]) -> Vec<f64> {
    let mut out = Vec::new();
    for t in set.tracks() {
        let Some(first) = t.points.first() else {
            continue;
        };
        let origin = motions[first.frame as usize].inv([first.x, first.y]);
        for p in t.points.iter().skip(1) {
            let want = motions[p.frame as usize].fwd(origin);
            out.push((p.x - want[0]).hypot(p.y - want[1]));
        }
    }
    out
}

fn median(mut v: Vec<f64>) -> f64 {
    assert!(!v.is_empty(), "no samples to take a median of");
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn quantile(mut v: Vec<f64>, q: f64) -> f64 {
    assert!(!v.is_empty(), "no samples to take a quantile of");
    v.sort_by(f64::total_cmp);
    v[(((v.len() - 1) as f64) * q).round() as usize]
}

/// A closed rectangular mask path in px@comp — the K-408 carriage the tracker
/// reads its exclusion regions through.
fn rect_mask(x0: f64, y0: f64, x1: f64, y1: f64, inverted: bool) -> ExclusionMask {
    let mut mask = Mask::rectangle(x0, y0, x1 - x0, y1 - y0);
    mask.inverted = inverted;
    ExclusionMask::from_mask(&mask, 0.0, 1.0)
}

// --- Detection and the track substrate -------------------------------------

#[test]
fn features_are_spread_across_the_buckets_and_ordered_deterministically() {
    let set = run(
        &[Motion::identity(centre())],
        TrackSettings::default(),
        vec![],
    );
    let tracks = set.tracks();
    assert!(
        tracks.len() > 60,
        "the textured quad should carry plenty of features, got {}",
        tracks.len()
    );
    // Ids ascend in detection order, and detection order is bucket row-major.
    let mut last_bucket = 0usize;
    for (i, t) in tracks.iter().enumerate() {
        assert_eq!(t.id, i as u32, "ids are assigned in detection order");
        let p = t.points[0];
        let bx = (p.x as usize) * 16 / W;
        let by = (p.y as usize) * 16 / H;
        let bucket = by * 16 + bx;
        assert!(
            bucket >= last_bucket,
            "buckets must be walked row-major: {bucket} came after {last_bucket}"
        );
        last_bucket = bucket;
    }
    // No two features on top of each other.
    for a in tracks {
        for b in tracks {
            if a.id < b.id {
                let d = (a.points[0].x - b.points[0].x).hypot(a.points[0].y - b.points[0].y);
                assert!(
                    d >= 6.0 - 1e-9,
                    "features {} and {} are {d} apart",
                    a.id,
                    b.id
                );
            }
        }
    }
}

#[test]
fn a_translating_quad_is_tracked_to_the_true_displacement() {
    let motions: Vec<Motion> = (0..10)
        .map(|i| Motion::translate(centre(), 1.7 * i as f64, -1.1 * i as f64))
        .collect();
    let set = run(&motions, TrackSettings::default(), vec![]);
    let e = errors(&set, &motions);
    // The median is the accuracy claim — a fiftieth of a pixel. The tail is a
    // handful of features sitting on a near-straight edge, where the solve is
    // ill-conditioned along the edge and always will be; what matters is that
    // the tail stays bounded rather than running away with the sequence.
    assert!(
        median(e.clone()) < 0.05,
        "median error {} px",
        median(e.clone())
    );
    assert!(
        quantile(e.clone(), 0.9) < 0.6,
        "p90 error {}",
        quantile(e.clone(), 0.9)
    );
    assert!(
        quantile(e.clone(), 1.0) < 2.0,
        "worst error {}",
        quantile(e, 1.0)
    );
    // Followed from the first frame to the last, not merely "live": a track
    // born on the final frame is live too, and counting those would let a
    // tracker that drops everything after one step pass this.
    let spanned = set.tracks_over(0, 9).count();
    assert!(spanned > 50, "{spanned} tracks spanned the sequence");
}

#[test]
fn a_rotating_quad_is_tracked_to_the_true_displacement() {
    let motions: Vec<Motion> = (0..10)
        .map(|i| Motion::rotate(centre(), 0.010 * i as f64))
        .collect();
    let set = run(&motions, TrackSettings::default(), vec![]);
    let e = errors(&set, &motions);
    assert!(
        median(e.clone()) < 0.15,
        "median error {}",
        median(e.clone())
    );
    assert!(
        quantile(e.clone(), 0.9) < 0.6,
        "p90 error {}",
        quantile(e, 0.9)
    );
    let spanned = set.tracks_over(0, 9).count();
    assert!(spanned > 40, "{spanned} tracks spanned the sequence");
}

#[test]
fn a_scaling_quad_is_tracked_to_the_true_displacement() {
    let motions: Vec<Motion> = (0..10)
        .map(|i| Motion::scale(centre(), 1.012f64.powi(i)))
        .collect();
    let set = run(&motions, TrackSettings::default(), vec![]);
    let e = errors(&set, &motions);
    assert!(
        median(e.clone()) < 0.15,
        "median error {}",
        median(e.clone())
    );
    assert!(
        quantile(e.clone(), 0.9) < 0.6,
        "p90 error {}",
        quantile(e, 0.9)
    );
    let spanned = set.tracks_over(0, 9).count();
    assert!(spanned > 40, "{spanned} tracks spanned the sequence");
}

#[test]
fn a_sheared_quad_is_tracked_to_the_true_displacement() {
    // The affine case proper: shear plus a slight squash plus a translation, so
    // no similarity model could explain it.
    let motions: Vec<Motion> = (0..8)
        .map(|i| Motion::shear(centre(), 0.008 * i as f64))
        .collect();
    let set = run(&motions, TrackSettings::default(), vec![]);
    let e = errors(&set, &motions);
    assert!(
        median(e.clone()) < 0.20,
        "median error {}",
        median(e.clone())
    );
    assert!(
        quantile(e.clone(), 0.9) < 0.7,
        "p90 error {}",
        quantile(e, 0.9)
    );
    let spanned = set.tracks_over(0, 7).count();
    assert!(spanned > 40, "{spanned} tracks spanned the sequence");
}

// --- The zoom detector's food ----------------------------------------------

#[test]
fn a_zoom_pair_reads_its_log_scale_out_of_the_affine_matrices() {
    // The phase-2 zoom-burst detector's whole input: a pair of frames differing
    // only by a scale about the centre must read as log(s) within 2 %
    // (docs/impl/tracking.md §5).
    for s in [1.12f64, 1.25, 0.85] {
        let motions = [Motion::identity(centre()), Motion::scale(centre(), s)];
        let set = run(&motions, TrackSettings::default(), vec![]);
        let got = set
            .median_log_scale(0)
            .expect("a zoom pair has steps to read");
        let want = s.ln();
        assert!(
            (got - want).abs() <= 0.02 * want.abs(),
            "scale {s}: median log-scale {got}, wanted {want}"
        );
    }
}

#[test]
fn a_still_pair_reads_no_zoom() {
    let motions = [
        Motion::identity(centre()),
        Motion::translate(centre(), 1.5, 0.8),
    ];
    let set = run(&motions, TrackSettings::default(), vec![]);
    let got = set.median_log_scale(0).expect("steps exist");
    assert!(got.abs() < 0.01, "a pure translation read log-scale {got}");
}

// --- Verification ----------------------------------------------------------

/// A sequence where an occluder sweeps left to right across the quad while the
/// picture underneath translates gently.
fn occluder_sequence(frames: usize) -> (Vec<Motion>, Vec<Vec<f32>>) {
    let motions: Vec<Motion> = (0..frames)
        .map(|i| Motion::translate(centre(), 0.9 * i as f64, 0.0))
        .collect();
    let planes = motions
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let x0 = -70.0 + 22.0 * i as f64;
            render(m, Some((x0, 0.0, x0 + 70.0, H as f64)))
        })
        .collect();
    (motions, planes)
}

fn run_planes(planes: &[Vec<f32>], settings: TrackSettings) -> TrackSet {
    let mut tracker = Tracker::new(settings);
    for (i, f) in planes.iter().enumerate() {
        tracker
            .push(i as i64, FramePlane::new(f, W, H).unwrap(), None)
            .unwrap();
    }
    tracker.finish()
}

#[test]
fn forward_backward_and_ncc_end_a_track_under_an_occluder() {
    let (_, planes) = occluder_sequence(9);
    let set = run_planes(&planes, TrackSettings::default());
    // Tracks born on frame 0 in the band the occluder crosses must die, not
    // follow the occluder or drift.
    let mut crossed = 0usize;
    let mut ended = 0usize;
    for t in set.tracks() {
        if t.first_frame() != 0 {
            continue;
        }
        let x = t.points[0].x;
        if !(60.0..190.0).contains(&x) {
            continue;
        }
        crossed += 1;
        if t.state == TrackState::Ended {
            ended += 1;
        }
    }
    assert!(crossed > 15, "only {crossed} tracks lay under the sweep");
    assert!(
        ended * 10 >= crossed * 9,
        "{ended} of {crossed} occluded tracks ended; the rest carried a wrong point"
    );
    // And nothing survived by teleporting: every ended track stopped before the
    // occluder had passed it.
    for t in set.tracks() {
        if t.state == TrackState::Ended {
            assert!(
                !t.points.is_empty(),
                "an ended track still holds the points it earned"
            );
        }
    }
}

#[test]
fn a_starved_bucket_is_detected_into_again() {
    let (_, planes) = occluder_sequence(12);
    let set = run_planes(&planes, TrackSettings::default());
    // The occluder wipes out the left of the frame and then moves off it, so
    // buckets there starve and must be repopulated with brand-new tracks.
    let reborn: Vec<&Track> = set
        .tracks()
        .iter()
        .filter(|t| t.first_frame() > 0 && t.points[0].x < 150.0)
        .collect();
    assert!(
        reborn.len() > 5,
        "re-detection produced only {} new tracks behind the occluder",
        reborn.len()
    );
    // Re-detected tracks are real tracks: they carry ids after the originals and
    // go on to be followed.
    let born_first = set.tracks().iter().filter(|t| t.first_frame() == 0).count();
    for t in &reborn {
        assert!(t.id as usize >= born_first, "ids stay monotonic");
    }
    assert!(
        reborn.iter().any(|t| t.points.len() > 2),
        "a re-detected track should go on to be followed"
    );
}

// --- Masks -----------------------------------------------------------------

#[test]
fn no_feature_is_born_inside_an_exclusion_mask() {
    let mask = rect_mask(120.0, 40.0, 220.0, 140.0, false);
    let set = run(
        &[Motion::identity(centre())],
        TrackSettings::default(),
        vec![mask.clone()],
    );
    assert!(
        !set.tracks().is_empty(),
        "the rest of the frame still tracks"
    );
    for t in set.tracks() {
        let p = t.points[0];
        assert!(
            !mask.excludes(p.x, p.y),
            "track {} was born at ({}, {}) inside the mask",
            t.id,
            p.x,
            p.y
        );
    }
}

#[test]
fn an_inverted_mask_keeps_features_inside_it() {
    let mask = rect_mask(120.0, 40.0, 220.0, 140.0, true);
    let set = run(
        &[Motion::identity(centre())],
        TrackSettings::default(),
        vec![mask.clone()],
    );
    assert!(
        !set.tracks().is_empty(),
        "an inverted mask must still allow its own interior"
    );
    for t in set.tracks() {
        let p = t.points[0];
        assert!(
            p.x >= 120.0 && p.x <= 220.0 && p.y >= 40.0 && p.y <= 140.0,
            "track {} was born at ({}, {}) outside the inverted mask",
            t.id,
            p.x,
            p.y
        );
    }
}

#[test]
fn a_track_wandering_into_a_mask_ends() {
    // A rightward pan with a mask on the right: tracks march into it and must
    // stop at its edge rather than cross it.
    let motions: Vec<Motion> = (0..12)
        .map(|i| Motion::translate(centre(), 6.0 * i as f64, 0.0))
        .collect();
    let mask = rect_mask(230.0, 0.0, 320.0, 180.0, false);
    let set = run(&motions, TrackSettings::default(), vec![mask.clone()]);
    let mut entered = 0usize;
    for t in set.tracks() {
        for p in &t.points {
            assert!(
                !mask.excludes(p.x, p.y),
                "track {} recorded ({}, {}) inside the mask",
                t.id,
                p.x,
                p.y
            );
        }
        let last = t.points[t.points.len() - 1];
        if t.state == TrackState::Ended && last.x > 200.0 {
            entered += 1;
        }
    }
    assert!(
        entered > 3,
        "only {entered} tracks were stopped at the mask edge"
    );
}

#[test]
fn both_polarities_agree_on_the_boundary() {
    // The bare-polyline carriage this time, and a comp→source factor of 1.
    let poly = flatten_path(
        &Mask::rectangle(10.0, 10.0, 40.0, 40.0).path,
        MASK_PATH_TOLERANCE_PX,
    );
    let inside = ExclusionMask::from_polyline(&poly, false, 1.0);
    let outside = ExclusionMask::from_polyline(&poly, true, 1.0);
    for (x, y) in [(30.0, 30.0), (5.0, 5.0), (60.0, 30.0), (30.0, 60.0)] {
        assert_ne!(
            inside.excludes(x, y),
            outside.excludes(x, y),
            "({x}, {y}) must be excluded by exactly one polarity"
        );
    }
    assert!(inside.excludes(30.0, 30.0));
    assert!(!inside.excludes(5.0, 5.0));
}

// --- Seeding ---------------------------------------------------------------

#[test]
fn the_pyramid_carries_a_jump_a_single_level_would_miss() {
    // The other half of the seeding story, and the one that says the pyramid is
    // load-bearing rather than decorative: the same 11 px jump, no flow seed at
    // all, solved once with a single level and once with three. If the
    // coarse-to-fine carry-down ever breaks, this is the test that notices —
    // nothing else in this file moves a feature further than a level-0 search
    // can reach on its own.
    let motions = [
        Motion::identity(centre()),
        Motion::translate(centre(), 11.0, -7.0),
    ];
    let carried = |levels: usize| {
        let set = run(
            &motions,
            TrackSettings {
                levels,
                ..TrackSettings::default()
            },
            vec![],
        );
        set.tracks().iter().filter(|t| t.points.len() > 1).count()
    };
    let (one, three) = (carried(1), carried(3));
    assert!(
        three > 20 * one.max(1),
        "three levels carried {three} tracks, one level {one}"
    );
}

#[test]
fn a_flow_seed_carries_a_jump_a_single_level_would_miss() {
    // One pyramid level, so the KLT's own capture range is a couple of pixels;
    // the pair moves 11. Without a seed almost everything dies or is refused;
    // with the flow field handed in as the seed, the tracks survive. That is
    // exactly §2's "flow is a seed, never a verdict".
    let settings = TrackSettings {
        levels: 1,
        ..TrackSettings::default()
    };
    let motions = [
        Motion::identity(centre()),
        Motion::translate(centre(), 11.0, -7.0),
    ];
    let unseeded = run(&motions, settings, vec![]);

    let a = render(&motions[0], None);
    let b = render(&motions[1], None);
    let flow = vec![[11.0f32, -7.0f32]; W * H];
    let mut tracker = Tracker::new(settings);
    tracker
        .push(0, FramePlane::new(&a, W, H).unwrap(), None)
        .unwrap();
    tracker
        .push(
            1,
            FramePlane::new(&b, W, H).unwrap(),
            Some(FlowSeed::new(&flow, W, H).unwrap()),
        )
        .unwrap();
    let seeded = tracker.finish();

    let followed = |set: &TrackSet| set.tracks().iter().filter(|t| t.points.len() > 1).count();
    assert!(
        followed(&seeded) > 4 * followed(&unseeded).max(1),
        "seeded {} vs unseeded {} tracks carried across the jump",
        followed(&seeded),
        followed(&unseeded)
    );
    let e = errors(&seeded, &motions);
    assert!(
        median(e.clone()) < 0.15,
        "seeded median error {}",
        median(e)
    );
}

// --- Determinism and the store ---------------------------------------------

#[test]
fn two_identical_runs_produce_the_identical_track_set() {
    let motions: Vec<Motion> = (0..6)
        .map(|i| Motion::rotate(centre(), 0.008 * i as f64))
        .collect();
    let a = run(&motions, TrackSettings::default(), vec![]);
    let b = run(&motions, TrackSettings::default(), vec![]);
    assert_eq!(a, b, "two runs over the same frames must agree bit for bit");
}

#[test]
fn the_store_answers_the_questions_phase_two_will_ask() {
    let motions: Vec<Motion> = (0..6)
        .map(|i| Motion::translate(centre(), 2.0 * i as f64, 0.0))
        .collect();
    let set = run(&motions, TrackSettings::default(), vec![]);

    assert_eq!(set.source_size(), (W, H));
    assert_eq!(set.frame_range(), Some((0, 5)));

    let over = set.tracks_over(0, 5).count();
    assert!(over > 40, "{over} tracks span the whole range");

    let pairs = set.correspondences(0, 5);
    assert_eq!(pairs.len(), over);
    for c in &pairs {
        let t = set.get(c.id).expect("a correspondence names a real track");
        assert_eq!(t.point_at(0).map(|p| [p.x, p.y]), Some(c.from));
        assert_eq!(t.point_at(5).map(|p| [p.x, p.y]), Some(c.to));
        // Two frames apart, five steps: the motion is 2 px per frame.
        assert!((c.to[0] - c.from[0] - 10.0).abs() < 0.5);
    }

    for t in set.tracks() {
        assert_eq!(
            t.steps.len() + 1,
            t.points.len(),
            "one step between each neighbouring pair"
        );
        for (i, p) in t.points.iter().enumerate() {
            assert_eq!(p.frame, t.first_frame() + i as i64, "points are contiguous");
        }
        assert!(t.state != TrackState::Moving, "phase 1 never segments");
    }
    assert!(set.get(u32::MAX).is_none());
}

// --- Boundary errors -------------------------------------------------------

#[test]
fn the_boundary_refuses_what_it_cannot_use() {
    let short = vec![0.0f32; 10];
    assert!(matches!(
        FramePlane::new(&short, 320, 180),
        Err(TrackError::PlaneSize { .. })
    ));
    assert!(matches!(
        FramePlane::new(&short, 0, 0),
        Err(TrackError::PlaneSize { .. })
    ));

    let a = vec![0.5f32; W * H];
    let mut tracker = Tracker::new(TrackSettings::default());
    tracker
        .push(4, FramePlane::new(&a, W, H).unwrap(), None)
        .unwrap();
    assert!(matches!(
        tracker.push(4, FramePlane::new(&a, W, H).unwrap(), None),
        Err(TrackError::FrameOrder { got: 4, last: 4 })
    ));
    let small = vec![0.5f32; 64 * 64];
    assert!(matches!(
        tracker.push(5, FramePlane::new(&small, 64, 64).unwrap(), None),
        Err(TrackError::SizeChanged { .. })
    ));
    let flow = vec![[0.0f32, 0.0f32]; 64 * 64];
    assert!(matches!(
        tracker.push(
            5,
            FramePlane::new(&a, W, H).unwrap(),
            Some(FlowSeed::new(&flow, 64, 64).unwrap())
        ),
        Err(TrackError::SeedSize { .. })
    ));
}

#[test]
fn an_empty_run_is_an_empty_set_not_a_fault() {
    let set = Tracker::new(TrackSettings::default()).finish();
    assert_eq!(set.source_size(), (0, 0));
    assert_eq!(set.frame_range(), None);
    assert_eq!(set.median_log_scale(0), None);
    assert!(set.correspondences(0, 1).is_empty());
}

// --- Perf sanity (a number in the output, not a gate) ----------------------

/// Not a gate — docs/13-PERFORMANCE-RULES owns the budgets, and phase 1 has none
/// yet (the impl note's open question: CPU first, profiled, and only then moved
/// to WGSL). This prints one number so the next person has something to compare
/// against. Run with:
/// `cargo test -p lumit-track --release -- --ignored --nocapture perf`
#[test]
#[ignore = "prints a timing; not a gate"]
fn perf_100_features_over_30_frames_of_640x360() {
    const PW: usize = 640;
    const PH: usize = 360;
    // A fully textured frame this time — the perf question is about the whole
    // raster, not about a quad on a flat field.
    let frames: Vec<Vec<f32>> = (0..30)
        .map(|i| {
            let dx = 1.3 * i as f64;
            let dy = -0.8 * i as f64;
            let mut out = vec![0.0f32; PW * PH];
            for y in 0..PH {
                for x in 0..PW {
                    out[y * PW + x] = texture(x as f64 - dx, y as f64 - dy);
                }
            }
            out
        })
        .collect();
    // 100 features: a 10×10 grid with one per bucket.
    // Both with and without re-detection, because the Shi-Tomasi response map
    // is a whole-frame pass and re-detection is what pays for it: the two
    // numbers say where the time actually goes.
    for redetect_below in [1usize, 0] {
        let settings = TrackSettings {
            grid: (10, 10),
            per_bucket: 1,
            redetect_below,
            ..TrackSettings::default()
        };
        let start = std::time::Instant::now();
        let mut tracker = Tracker::new(settings);
        for (i, f) in frames.iter().enumerate() {
            tracker
                .push(i as i64, FramePlane::new(f, PW, PH).unwrap(), None)
                .unwrap();
        }
        let set = tracker.finish();
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        println!(
            "lumit-track perf: {} tracks, redetect_below={redetect_below}, 30 frames of {PW}x{PH} in {ms:.0} ms ({:.1} ms/frame)",
            set.tracks().len(),
            ms / 30.0
        );
    }
}
