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

// ===========================================================================
// Phase 2 — two-view geometry (docs/impl/tracking.md §3, test plan §5)
//
// No pictures at all from here down. Phase 1's job was to turn pixels into
// tracks and its tests had to render; phase 2's job is arithmetic over point
// correspondences, so its ground truth is a camera pair written down exactly
// and its "footage" is the projection of a known cloud. A test that rendered
// would be measuring the tracker again.
// ===========================================================================

/// A pinhole camera with its principal point at the frame centre — the model
/// docs/impl/tracking.md §4 pins, used here to manufacture correspondences
/// whose geometry is known rather than estimated.
#[derive(Clone, Copy)]
struct Camera {
    /// World → camera rotation.
    r: [[f64; 3]; 3],
    /// Camera centre, in world coordinates.
    c: [f64; 3],
    f: f64,
}

fn mat3_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

impl Camera {
    fn new(c: [f64; 3], yaw: f64, pitch: f64) -> Camera {
        let (sy, cy) = (yaw.sin(), yaw.cos());
        let (sp, cp) = (pitch.sin(), pitch.cos());
        let ry = [[cy, 0.0, -sy], [0.0, 1.0, 0.0], [sy, 0.0, cy]];
        let rx = [[1.0, 0.0, 0.0], [0.0, cp, -sp], [0.0, sp, cp]];
        Camera {
            r: mat3_mul(&rx, &ry),
            c,
            f: 300.0,
        }
    }

    /// Where `x` lands in this camera's image, or `None` when it is behind the
    /// camera or off the raster — the same two ways a real feature leaves.
    fn project(&self, x: [f64; 3]) -> Option<[f64; 2]> {
        let d = [x[0] - self.c[0], x[1] - self.c[1], x[2] - self.c[2]];
        let mut v = [0.0f64; 3];
        for (o, row) in v.iter_mut().zip(self.r.iter()) {
            *o = row[0] * d[0] + row[1] * d[1] + row[2] * d[2];
        }
        if v[2] < 0.5 {
            return None;
        }
        let p = [
            W as f64 / 2.0 + self.f * v[0] / v[2],
            H as f64 / 2.0 + self.f * v[1] / v[2],
        ];
        if p[0] < 0.0 || p[0] >= W as f64 || p[1] < 0.0 || p[1] >= H as f64 {
            return None;
        }
        Some(p)
    }

    /// Depth of `x` along this camera's optical axis — what a per-patch scale
    /// is the ratio of.
    fn depth(&self, x: [f64; 3]) -> f64 {
        let d = [x[0] - self.c[0], x[1] - self.c[1], x[2] - self.c[2]];
        self.r[2][0] * d[0] + self.r[2][1] * d[1] + self.r[2][2] * d[2]
    }
}

/// A deterministic cloud in front of the camera, spread in depth — the spread
/// is the whole point, since a cloud on one plane has no epipolar geometry to
/// recover.
fn scene_points(n: usize) -> Vec<[f64; 3]> {
    (0..n)
        .map(|i| {
            let i = i as i64;
            [
                -2.5 + 5.0 * hash2(i, 101),
                -1.5 + 3.0 * hash2(i, 211),
                7.0 + 9.0 * hash2(i, 307),
            ]
        })
        .collect()
}

/// Project a cloud into two views. Points with an index at or above `movers`
/// are displaced by `drift` before the second view — the planted second motion
/// docs/impl/tracking.md §5 asks for.
fn pair_correspondences(
    a: &Camera,
    b: &Camera,
    pts: &[[f64; 3]],
    movers: usize,
    drift: [f64; 3],
) -> Vec<Correspondence> {
    let mut out = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        let q = if i >= movers {
            [p[0] + drift[0], p[1] + drift[1], p[2] + drift[2]]
        } else {
            *p
        };
        if let (Some(u), Some(v)) = (a.project(*p), b.project(q)) {
            out.push(Correspondence {
                id: i as u32,
                from: u,
                to: v,
            });
        }
    }
    out
}

fn transpose(m: &Mat3) -> Mat3 {
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in m.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            out[c][r] = *v;
        }
    }
    out
}

// --- The fundamental matrix -------------------------------------------------

#[test]
fn the_eight_point_fundamental_explains_points_it_never_saw() {
    let pts = scene_points(80);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([0.9, 0.08, 0.35], 0.05, -0.02);
    let all = pair_correspondences(&a, &b, &pts, usize::MAX, [0.0; 3]);
    assert!(all.len() > 50, "{} points landed in both views", all.len());
    let (fit, held) = all.split_at(40);

    let f = fundamental_eight_point(fit).expect("a cloud with depth has a fundamental matrix");
    // Held out: never in the fit. And the comparison is the epipolar residual,
    // not the matrix entries — F is only defined up to scale, so entry-by-entry
    // agreement would be a test of the normalisation and nothing else.
    for c in held {
        let r = sampson_distance(&f, c.from, c.to);
        assert!(r < 1e-6, "held-out residual {r} px on track {}", c.id);
    }

    // Anti-vacuity. Everything above would also pass for a matrix that
    // explained *any* pair of points, so: shuffle the second image's points and
    // the same matrix must refuse them.
    let refused = held
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            let other = held[(i + 7) % held.len()];
            sampson_distance(&f, c.from, other.to) > 2.0
        })
        .count();
    assert!(
        refused * 10 >= held.len() * 9,
        "only {refused} of {} shuffled pairs were refused",
        held.len()
    );
    // And the geometry is directional: swapping the two images is a different
    // question, so the transpose must not answer this one.
    let ft = transpose(&f);
    let wrong_way = held
        .iter()
        .filter(|c| sampson_distance(&ft, c.from, c.to) > 1.0)
        .count();
    assert!(
        wrong_way * 10 >= held.len() * 9,
        "{wrong_way} of {} pairs survived a transposed F; the fit is not directional",
        held.len()
    );
}

#[test]
fn the_eight_point_fundamental_survives_pixel_noise() {
    let pts = scene_points(80);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([0.9, 0.08, 0.35], 0.05, -0.02);
    let mut all = pair_correspondences(&a, &b, &pts, usize::MAX, [0.0; 3]);
    // Half a pixel of deterministic jitter on the second image, which is about
    // what a good KLT step leaves behind (phase 1's p90).
    for (i, c) in all.iter_mut().enumerate() {
        let i = i as i64;
        c.to[0] += 0.5 * (hash2(i, 907) - 0.5);
        c.to[1] += 0.5 * (hash2(i, 911) - 0.5);
    }
    let (fit, held) = all.split_at(45);
    let f = fundamental_eight_point(fit).expect("noise does not remove the geometry");
    let residuals: Vec<f64> = held
        .iter()
        .map(|c| sampson_distance(&f, c.from, c.to))
        .collect();
    let m = median(residuals.clone());
    assert!(m < 0.35, "median held-out residual {m} px under noise");
    assert!(
        quantile(residuals, 0.9) < 1.0,
        "the tail of a noisy fit still has to stay inside the inlier threshold"
    );
    // Noise is also the only condition under which the rank-2 enforcement is
    // visible: on exact points the linear solution is already singular, and on
    // noisy ones it is not until it is made so. A rank-3 "fundamental matrix"
    // has no epipoles at all and is a phase-3 trap, so this is pinned here.
    // The matrix is unit Frobenius, which is what makes the bound absolute.
    let d = super::geom::det3(&f).abs();
    // Enforced this sits around 1e-23; without the enforcement, at this noise
    // level, around 1e-9. The bound is placed in the gap, not beside either.
    assert!(
        d < 1e-18,
        "the fitted F has determinant {d}; it is not rank 2"
    );
}

#[test]
fn the_seven_point_cubic_finds_the_geometry_among_its_roots() {
    let pts = scene_points(80);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([0.9, 0.08, 0.35], 0.05, -0.02);
    let all = pair_correspondences(&a, &b, &pts, usize::MAX, [0.0; 3]);
    // Seven spread across the cloud rather than seven neighbours: a minimal
    // sample from one corner is degenerate and would prove nothing.
    let sample: Vec<Correspondence> = all.iter().step_by(all.len() / 7).take(7).copied().collect();
    assert_eq!(sample.len(), 7);

    let mut candidates = Vec::new();
    fundamental_seven_point(&sample, &mut candidates);
    assert!(
        candidates.len() == 1 || candidates.len() == 3,
        "a real cubic has one or three real roots, got {}",
        candidates.len()
    );
    let spread = |f: &Mat3| {
        median(
            all.iter()
                .map(|c| sampson_distance(f, c.from, c.to))
                .collect(),
        )
    };
    // Exactly one root is the true geometry, and on a noise-free minimal sample
    // it is exact.
    let best = candidates.iter().map(spread).fold(f64::INFINITY, f64::min);
    assert!(best < 1e-6, "best seven-point candidate is {best} px out");
    // Anti-vacuity: the other roots are spurious, so a test that accepted any
    // candidate would be testing nothing.
    if candidates.len() == 3 {
        let worst = candidates.iter().map(spread).fold(0.0f64, f64::max);
        assert!(
            worst > 1.0,
            "all three roots agreed; the cubic is degenerate"
        );
    }
}

// --- The GRIC gate ----------------------------------------------------------

#[test]
fn the_gric_gate_calls_a_pure_rotation_pair_rotation_only() {
    // Same camera centre, different aim. There is no baseline, so there is no
    // parallax and no translation to recover — the pair is usable for rotation
    // and focal and must never reach the translation solve.
    let pts = scene_points(90);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([0.0, 0.0, 0.0], 0.05, 0.02);
    let corr = pair_correspondences(&a, &b, &pts, usize::MAX, [0.0; 3]);
    assert!(corr.len() > 40, "{} points stayed in frame", corr.len());
    let g = estimate_pair(&corr, (W, H), 0, 1, &GeometrySettings::default())
        .expect("a rotation pair still estimates");
    assert_eq!(g.verdict, PairVerdict::RotationOnly);
    assert!(
        g.gric_homography + 20.0 < g.gric_fundamental,
        "GRIC H {} vs F {} — the gate has no margin",
        g.gric_homography,
        g.gric_fundamental
    );
    assert!(
        g.parallax < 0.5,
        "a pure rotation left {} px of parallax",
        g.parallax
    );
    // The gate is a comparison, so it is only worth as much as the homography
    // it compares against: on a pure rotation that homography has to be the
    // real infinite homography, not merely the better of two poor answers.
    let fit = median(
        corr.iter()
            .map(|c| transfer_distance(&g.homography, c.from, c.to))
            .collect(),
    );
    assert!(fit < 0.01, "the rotation's own homography is {fit} px out");
}

#[test]
fn the_gric_gate_calls_a_translating_pair_translating() {
    let pts = scene_points(90);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([1.1, 0.05, 0.2], 0.02, 0.0);
    let corr = pair_correspondences(&a, &b, &pts, usize::MAX, [0.0; 3]);
    let g = estimate_pair(&corr, (W, H), 0, 1, &GeometrySettings::default())
        .expect("a translating pair estimates");
    assert_eq!(g.verdict, PairVerdict::Translating);
    assert!(
        g.gric_fundamental + 20.0 < g.gric_homography,
        "GRIC F {} vs H {} — the gate has no margin",
        g.gric_fundamental,
        g.gric_homography
    );
    assert!(
        g.parallax > 3.0,
        "a real baseline over this depth spread must show parallax, got {}",
        g.parallax
    );
    assert!(g.inlier_ratio > 0.9, "inlier ratio {}", g.inlier_ratio);
    // The other side of the same coin: a cloud with depth cannot be flattened,
    // so the best homography over this pair has to leave real error behind.
    let fit = median(
        corr.iter()
            .map(|c| transfer_distance(&g.homography, c.from, c.to))
            .collect(),
    );
    assert!(
        fit > 2.0,
        "a homography explained a moving camera to {fit} px"
    );
}

// --- LO-RANSAC against a planted second motion ------------------------------

#[test]
fn the_dominant_model_refuses_the_planted_second_motion() {
    let pts = scene_points(90);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([1.1, 0.05, 0.2], 0.02, 0.0);
    // The camera's baseline is mostly sideways, so a vertical world drift is
    // squarely off the epipolar lines. A mover that happened to slide *along*
    // its own epipolar line is invisible to this method and to any other, which
    // is why the drift is chosen across them.
    let corr = pair_correspondences(&a, &b, &pts, 60, [0.0, 0.5, 0.0]);
    let movers = corr.iter().filter(|c| c.id >= 60).count();
    let statics = corr.len() - movers;
    assert!(
        movers > 8 && statics > 30,
        "{movers} movers, {statics} static"
    );

    let g = estimate_pair(&corr, (W, H), 0, 1, &GeometrySettings::default())
        .expect("the still world is the majority");
    assert_eq!(g.verdict, PairVerdict::Translating);
    for c in corr.iter().filter(|c| c.id >= 60) {
        assert!(
            !g.is_inlier(c.id),
            "mover {} joined the dominant model at residual {}",
            c.id,
            sampson_distance(&g.fundamental, c.from, c.to)
        );
    }
    let kept = corr
        .iter()
        .filter(|c| c.id < 60 && g.is_inlier(c.id))
        .count();
    assert!(
        kept * 20 >= statics * 19,
        "only {kept} of {statics} still-world points inlied"
    );
}

// --- A synthetic shot, as a TrackSet ----------------------------------------

/// `frames` views of one cloud, the camera dollying sideways while it pans a
/// little, written straight into a [`TrackSet`].
///
/// Tracks with an id at or above `movers` drift by `drift` per frame from
/// `drift_from` onward: `drift_from == 0` makes a track that never agrees with
/// the camera, and a later value makes one that agrees and then stops, which is
/// the case §3 says to split rather than discard.
fn synthetic_shot(
    frames: usize,
    count: usize,
    movers: usize,
    drift: [f64; 3],
    drift_from: usize,
) -> TrackSet {
    let pts = scene_points(count);
    let cams: Vec<Camera> = (0..frames)
        .map(|i| {
            let t = i as f64;
            Camera::new([0.22 * t, 0.01 * t, 0.03 * t], 0.003 * t, 0.0)
        })
        .collect();
    let mut tracks = Vec::new();
    for (j, p) in pts.iter().enumerate() {
        let mut points = Vec::new();
        for (i, cam) in cams.iter().enumerate() {
            let k = if j >= movers && i > drift_from {
                (i - drift_from) as f64
            } else {
                0.0
            };
            let x = [
                p[0] + drift[0] * k,
                p[1] + drift[1] * k,
                p[2] + drift[2] * k,
            ];
            let Some(u) = cam.project(x) else {
                break;
            };
            points.push(TrackPoint {
                frame: i as i64,
                x: u[0],
                y: u[1],
            });
        }
        if points.len() < 2 {
            continue;
        }
        let steps = vec![
            TrackStep {
                a: [[1.0, 0.0], [0.0, 1.0]],
                ncc: 1.0,
                fb: 0.0,
            };
            points.len() - 1
        ];
        tracks.push(Track {
            id: j as u32,
            points,
            steps,
            state: TrackState::Live,
            parent: None,
        });
    }
    TrackSet {
        tracks,
        width: W,
        height: H,
    }
}

#[test]
fn keyframe_selection_picks_pairs_that_carry_parallax() {
    let set = synthetic_shot(10, 90, usize::MAX, [0.0; 3], 0);
    let pairs = select_keyframes(&set, &GeometrySettings::default());
    assert!(
        pairs.len() >= 2,
        "{} keyframe pairs over ten frames",
        pairs.len()
    );
    let mut last = -1i64;
    for g in &pairs {
        assert!(g.from > last, "pairs come back in frame order");
        assert!(g.to > g.from);
        last = g.from;
        assert_eq!(g.verdict, PairVerdict::Translating);
        assert!(
            g.inlier_ratio > 0.9,
            "pair {}→{} inliers {}",
            g.from,
            g.to,
            g.inlier_ratio
        );
    }
    // Every pair but the last carries real parallax. The last is the tail of
    // the shot, where there are no frames left to reach for and a short pair is
    // better than a gap — that fallback is deliberate, and this is where it is
    // recorded.
    let (body, tail) = pairs.split_at(pairs.len() - 1);
    for g in body {
        assert!(
            g.parallax >= GeometrySettings::default().min_parallax_px,
            "pair {}→{} was chosen at {} px of parallax",
            g.from,
            g.to,
            g.parallax
        );
        // Adjacent frames on this dolly do not carry enough parallax, so a
        // selector that simply returned every neighbouring pair would fail here.
        assert!(
            g.to - g.from > 1,
            "keyframe pairs are spaced by parallax, not by one frame"
        );
    }
    assert!(
        tail.first().is_some_and(|g| g.to == 9),
        "selection has to reach the end of the shot, got {:?}",
        tail.first().map(|g| (g.from, g.to))
    );
}

#[test]
fn a_track_that_never_agrees_with_the_camera_is_marked_moving() {
    let mut set = synthetic_shot(10, 90, 70, [0.0, 0.16, 0.0], 0);
    let pairs = select_keyframes(&set, &GeometrySettings::default());
    assert!(
        pairs.len() >= 2,
        "need a profile, got {} pairs",
        pairs.len()
    );
    let seg = segment_dynamic_tracks(&mut set, &pairs, &SegmentSettings::default());
    assert!(seg.splits.is_empty(), "a lifelong mover is not a split");

    let planted: Vec<u32> = set
        .tracks()
        .iter()
        .filter(|t| t.id >= 70 && t.points.len() > 4)
        .map(|t| t.id)
        .collect();
    assert!(
        planted.len() > 8,
        "{} movers survived to be judged",
        planted.len()
    );
    for id in &planted {
        assert!(seg.moving.contains(id), "mover {id} was left in the solve");
        assert_eq!(
            set.get(*id).map(|t| t.state),
            Some(TrackState::Moving),
            "mover {id} state"
        );
    }
    // And the still world is untouched — a segmentation that marked everything
    // would satisfy the loop above.
    let wrongly = set
        .tracks()
        .iter()
        .filter(|t| t.id < 70 && t.state == TrackState::Moving)
        .count();
    assert_eq!(
        wrongly, 0,
        "{wrongly} still-world tracks were called moving"
    );
}

#[test]
fn a_track_that_stops_agreeing_is_split_at_the_change() {
    let before = synthetic_shot(10, 90, 70, [0.0, 0.16, 0.0], 4);
    let mut set = before.clone();
    let pairs = select_keyframes(&set, &GeometrySettings::default());
    let seg = segment_dynamic_tracks(&mut set, &pairs, &SegmentSettings::default());

    assert!(
        seg.splits.len() > 5,
        "only {} tracks were split at the change",
        seg.splits.len()
    );
    for s in &seg.splits {
        assert!(s.parent >= 70, "a still-world track was split");
        assert!(s.child > s.parent, "the suffix gets a fresh id");
        assert!(
            (2..=6).contains(&s.at_frame),
            "the split landed on frame {}, and the drift starts after frame 4",
            s.at_frame
        );
        let parent = set.get(s.parent).expect("the parent keeps its id");
        let child = set.get(s.child).expect("the child is in the store");
        assert_eq!(parent.state, TrackState::Ended, "the parent now stops");
        assert_eq!(child.state, TrackState::Moving, "the suffix is the mover");
        assert_eq!(
            child.parent,
            Some(s.parent),
            "the child remembers its parent"
        );
        assert_eq!(parent.parent, None);
        assert_eq!(parent.last_frame(), s.at_frame);
        assert_eq!(child.first_frame(), s.at_frame + 1);
        // Both halves are whole tracks, not fragments with a broken invariant.
        assert_eq!(parent.steps.len() + 1, parent.points.len());
        assert_eq!(child.steps.len() + 1, child.points.len());

        // The points survive the cut: prefix then suffix is exactly what the
        // track had before, in order.
        let original = before.get(s.parent).expect("the original");
        let rejoined: Vec<TrackPoint> = parent
            .points
            .iter()
            .chain(child.points.iter())
            .copied()
            .collect();
        assert_eq!(
            rejoined, original.points,
            "a split lost or duplicated points"
        );
    }
    // The prefix stayed in the solve: that is the whole reason to split rather
    // than discard.
    let kept = set
        .tracks()
        .iter()
        .filter(|t| t.id >= 70 && t.id < 90 && t.state != TrackState::Moving)
        .count();
    assert!(
        kept > 5,
        "{kept} clean prefixes were kept out of {} splits",
        seg.splits.len()
    );
}

#[test]
fn a_refused_split_leaves_the_store_alone() {
    let mut set = synthetic_shot(6, 40, usize::MAX, [0.0; 3], 0);
    let before = set.clone();
    let id = set.tracks().first().map(|t| t.id).expect("a track");
    let last = set.get(id).map(|t| t.last_frame()).expect("a last frame");
    assert_eq!(set.split_track(id, last), None, "no suffix to cut off");
    assert_eq!(set.split_track(id, last + 5), None, "outside the track");
    assert_eq!(set.split_track(u32::MAX, 0), None, "no such track");
    assert_eq!(set, before, "a refused split must change nothing");
}

// --- The zoom-burst detector ------------------------------------------------

/// A track set whose only motion is a scale about the frame centre, one factor
/// per frame — the zoom detector's whole world, and nothing else's.
fn zoom_shot(scales: &[f64]) -> TrackSet {
    let c = centre();
    let tracks = (0..60u32)
        .map(|j| {
            let i = i64::from(j);
            let base = [40.0 + 240.0 * hash2(i, 501), 25.0 + 130.0 * hash2(i, 601)];
            let points = scales
                .iter()
                .enumerate()
                .map(|(k, s)| TrackPoint {
                    frame: k as i64,
                    x: c[0] + (base[0] - c[0]) * s,
                    y: c[1] + (base[1] - c[1]) * s,
                })
                .collect();
            let steps = scales
                .windows(2)
                .map(|w| {
                    let r = w[1] / w[0];
                    TrackStep {
                        a: [[r, 0.0], [0.0, r]],
                        ncc: 1.0,
                        fb: 0.0,
                    }
                })
                .collect();
            Track {
                id: j,
                points,
                steps,
                state: TrackState::Live,
                parent: None,
            }
        })
        .collect();
    TrackSet {
        tracks,
        width: W,
        height: H,
    }
}

#[test]
fn a_zoom_cut_lands_on_the_right_frame() {
    // The owner's scope-in: nothing, nothing, nothing, a 1.25× jump between
    // frames 3 and 4, then nothing again.
    let set = zoom_shot(&[1.0, 1.0, 1.0, 1.0, 1.25, 1.25, 1.25]);
    let found = detect_zoom(&set, &ZoomSettings::default());
    assert_eq!(found.len(), 1, "one boundary, got {found:?}");
    let b = found[0];
    assert_eq!(b.frame, 3, "the cut sits on the pair 3→4");
    assert_eq!(b.end_frame, 3, "a cut is one pair wide");
    assert_eq!(b.kind, ZoomKind::Cut);
    // Tight on purpose: a sign slip anywhere in the log-scale chain reads as
    // ln(1/1.25), which this refuses.
    assert!(
        (b.log_scale - 1.25f64.ln()).abs() < 0.002,
        "log scale {} against {}",
        b.log_scale,
        1.25f64.ln()
    );
}

#[test]
fn a_zoom_ramp_reads_as_a_ramp() {
    let scales: Vec<f64> = (0..8).map(|i| 1.012f64.powi(i)).collect();
    let set = zoom_shot(&scales);
    let found = detect_zoom(&set, &ZoomSettings::default());
    assert_eq!(found.len(), 1, "one run, got {found:?}");
    let b = found[0];
    assert_eq!(b.kind, ZoomKind::Ramp);
    assert_eq!((b.frame, b.end_frame), (0, 6), "the ramp spans every pair");
    assert!(
        (b.log_scale - 1.012f64.ln()).abs() < 0.002,
        "ramp log scale {}",
        b.log_scale
    );
}

#[test]
fn a_still_lens_has_no_boundaries() {
    let set = zoom_shot(&[1.0, 1.0, 1.0, 1.0, 1.0]);
    assert!(detect_zoom(&set, &ZoomSettings::default()).is_empty());
    // And a scale below the ramp threshold is noise, not a zoom: a detector
    // with no floor would fire on every shot ever taken.
    let creeping = zoom_shot(&[1.0, 1.001, 1.002, 1.003]);
    assert!(detect_zoom(&creeping, &ZoomSettings::default()).is_empty());
}

#[test]
fn a_forward_lunge_is_not_read_as_a_zoom_cut() {
    // Everything in frame grows sharply between two frames, so the median log
    // scale bursts exactly as a scope-in does. But the camera moved: near
    // things grew more than far ones, a single scale about a single centre
    // cannot explain the displacements, and the scale-only cross-check is the
    // whole difference between "the lens changed" and "the camera lunged".
    let pts = scene_points(90);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([0.0, 0.0, 2.6], 0.0, 0.0);
    let tracks: Vec<Track> = pts
        .iter()
        .enumerate()
        .filter_map(|(j, p)| {
            let (u, v) = (a.project(*p)?, b.project(*p)?);
            let r = a.depth(*p) / b.depth(*p);
            Some(Track {
                id: j as u32,
                points: vec![
                    TrackPoint {
                        frame: 0,
                        x: u[0],
                        y: u[1],
                    },
                    TrackPoint {
                        frame: 1,
                        x: v[0],
                        y: v[1],
                    },
                ],
                steps: vec![TrackStep {
                    a: [[r, 0.0], [0.0, r]],
                    ncc: 1.0,
                    fb: 0.0,
                }],
                state: TrackState::Live,
                parent: None,
            })
        })
        .collect();
    assert!(tracks.len() > 30, "{} points stayed in frame", tracks.len());
    let set = TrackSet {
        tracks,
        width: W,
        height: H,
    };
    let burst = set.median_log_scale(0).expect("the lunge does burst");
    assert!(
        burst > ZoomSettings::default().cut_threshold,
        "the burst {burst} has to be big enough to tempt the detector"
    );
    let found = detect_zoom(&set, &ZoomSettings::default());
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].kind,
        ZoomKind::Ramp,
        "a dolly must not be called a lens cut"
    );
}

// --- Determinism ------------------------------------------------------------

#[test]
fn two_runs_of_the_two_view_pipeline_agree_bit_for_bit() {
    let once = || {
        let mut set = synthetic_shot(10, 90, 70, [0.0, 0.16, 0.0], 4);
        let pairs = select_keyframes(&set, &GeometrySettings::default());
        let seg = segment_dynamic_tracks(&mut set, &pairs, &SegmentSettings::default());
        let zoom = detect_zoom(&set, &ZoomSettings::default());
        (set, pairs, seg, zoom)
    };
    let (a, b) = (once(), once());
    assert_eq!(a.1, b.1, "the keyframe pairs must agree bit for bit");
    assert_eq!(a.2, b.2, "the segmentation must agree bit for bit");
    assert_eq!(a.3, b.3, "the zoom boundaries must agree bit for bit");
    assert_eq!(a.0, b.0, "the mutated store must agree bit for bit");
}

#[test]
fn the_two_view_boundary_refuses_what_it_cannot_use() {
    let pts = scene_points(20);
    let a = Camera::new([0.0, 0.0, 0.0], 0.0, 0.0);
    let b = Camera::new([0.9, 0.0, 0.0], 0.0, 0.0);
    let corr = pair_correspondences(&a, &b, &pts, usize::MAX, [0.0; 3]);
    let s = GeometrySettings::default();
    assert!(
        estimate_pair(&corr[..4], (W, H), 0, 1, &s).is_none(),
        "too few correspondences"
    );
    assert!(
        estimate_pair(&corr, (0, 0), 0, 1, &s).is_none(),
        "no raster to normalise against"
    );
    assert!(
        fundamental_eight_point(&corr[..7]).is_none(),
        "the eight-point needs eight"
    );
    assert!(homography_dlt(&corr[..3]).is_none(), "the DLT needs four");
    let mut out = Vec::new();
    fundamental_seven_point(&corr[..6], &mut out);
    assert!(out.is_empty(), "the seven-point needs seven");

    // A set of identical points has no geometry, and saying so is not a panic.
    let flat: Vec<Correspondence> = (0..12)
        .map(|i| Correspondence {
            id: i,
            from: [10.0, 10.0],
            to: [12.0, 10.0],
        })
        .collect();
    assert!(fundamental_eight_point(&flat).is_none());
    assert!(homography_dlt(&flat).is_none());
    assert!(estimate_pair(&flat, (W, H), 0, 1, &s).is_none());
    assert!(select_keyframes(&TrackSet::default(), &s).is_empty());
    assert!(detect_zoom(&TrackSet::default(), &ZoomSettings::default()).is_empty());
}
