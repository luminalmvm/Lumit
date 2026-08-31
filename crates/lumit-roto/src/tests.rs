//! The RB1 half of docs/impl/roto.md §10: synthetic shots with **known
//! mattes**, shapes rendered here rather than loaded, so every claim is an
//! assertion rather than a look.

use crate::{
    base_seeds, warp_and_seed, FlowField, FrameRgb, RotoError, RotoSettings, RotoSolver,
    RotoStroke, Seed, Seeds, StrokeKind,
};

const SUBJECT: [f32; 3] = [0.85, 0.55, 0.20];
const BACKDROP: [f32; 3] = [0.12, 0.18, 0.35];

/// A deterministic, structured wobble so no region of a test shot is flat —
/// a flat picture would let the geodesic walk look better than it is.
///
/// Smooth rather than per-pixel noise, and deliberately: γ multiplies the
/// colour step *between neighbours*, so white noise would make walking across
/// a flat wall cost more than crossing the subject's edge. Real footage's
/// texture is spatially correlated; this is a cheap stand-in for that.
fn texture(x: i32, y: i32, amp: f32) -> f32 {
    ((x as f32 * 0.13).sin() * 0.5 + (y as f32 * 0.11).cos() * 0.5) * amp
}

fn tint(base: [f32; 3], x: i32, y: i32, amp: f32) -> [f32; 3] {
    let t = texture(x, y, amp);
    [
        (base[0] + t).clamp(0.0, 1.0),
        (base[1] + t).clamp(0.0, 1.0),
        (base[2] + t).clamp(0.0, 1.0),
    ]
}

fn frame_from<F: Fn(u32, u32) -> [f32; 3]>(w: u32, h: u32, f: F) -> Vec<f32> {
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&f(x, y));
        }
    }
    out
}

fn stroke(points: &[(f32, f32)], radius: f32, kind: StrokeKind, frame: i64) -> RotoStroke {
    RotoStroke {
        id: uuid::Uuid::nil(),
        points: points.to_vec(),
        radius,
        kind,
        frame,
    }
}

fn iou(matte: &[f32], truth: &[bool]) -> f32 {
    let mut inter = 0usize;
    let mut union = 0usize;
    for (a, t) in matte.iter().zip(truth.iter()) {
        let m = *a >= 0.5;
        if m && *t {
            inter += 1;
        }
        if m || *t {
            union += 1;
        }
    }
    inter as f32 / union.max(1) as f32
}

fn solve_one(w: u32, h: u32, rgb: &[f32], seeds: &Seeds) -> Vec<f32> {
    let mut solver = RotoSolver::new(RotoSettings::default());
    let mut out = vec![0.0f32; (w as usize) * (h as usize)];
    solver
        .solve(FrameRgb::new(rgb, w, h).unwrap(), seeds, &mut out)
        .unwrap();
    out
}

// ---------------------------------------------------------------------------
// §10.1 — the single-frame solve
// ---------------------------------------------------------------------------

const DW: u32 = 128;
const DH: u32 = 128;
const DR: f32 = 40.0;

fn disc_shot(cx: f32, cy: f32, r: f32) -> (Vec<f32>, Vec<bool>) {
    let mut truth = Vec::with_capacity((DW as usize) * (DH as usize));
    for y in 0..DH {
        for x in 0..DW {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            truth.push(dx * dx + dy * dy <= r * r);
        }
    }
    let rgb = frame_from(DW, DH, |x, y| {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        if dx * dx + dy * dy <= r * r {
            tint(SUBJECT, x as i32, y as i32, 0.06)
        } else {
            tint(BACKDROP, x as i32, y as i32, 0.06)
        }
    });
    (rgb, truth)
}

#[test]
fn textured_disc_solves_to_its_analytic_shape() {
    let (rgb, truth) = disc_shot(64.0, 64.0, DR);
    // One stroke inside, nothing outside: the border ring is the background.
    let seeds = base_seeds(
        DW,
        DH,
        &[stroke(
            &[(52.0, 64.0), (76.0, 64.0)],
            3.0,
            StrokeKind::Foreground,
            0,
        )],
    )
    .unwrap();
    let matte = solve_one(DW, DH, &rgb, &seeds);
    assert!(iou(&matte, &truth) >= 0.98, "IoU {}", iou(&matte, &truth));
}

#[test]
fn the_edge_band_is_monotone_outwards() {
    let (rgb, _) = disc_shot(64.0, 64.0, DR);
    let seeds = base_seeds(
        DW,
        DH,
        &[stroke(
            &[(52.0, 64.0), (76.0, 64.0)],
            3.0,
            StrokeKind::Foreground,
            0,
        )],
    )
    .unwrap();
    let matte = solve_one(DW, DH, &rgb, &seeds);
    // Ring means rather than one ray: the shot is textured on purpose, and a
    // single ray would be reading the texture, not the edge.
    let mut previous = f32::INFINITY;
    for ring in 30..=50 {
        let r = ring as f32;
        let mut sum = 0.0f32;
        let mut count = 0.0f32;
        for step in 0..360 {
            let a = step as f32 * std::f32::consts::TAU / 360.0;
            let x = (64.0 + r * a.cos()).round() as i32;
            let y = (64.0 + r * a.sin()).round() as i32;
            if x < 0 || y < 0 || x >= DW as i32 || y >= DH as i32 {
                continue;
            }
            sum += matte[(y as usize) * (DW as usize) + (x as usize)];
            count += 1.0;
        }
        let mean = sum / count.max(1.0);
        assert!(
            mean <= previous + 1e-3,
            "ring {ring} rose to {mean} from {previous}"
        );
        previous = mean;
    }
}

#[test]
fn the_low_contrast_neck_leaks_by_design() {
    // The documented ceiling, pinned: a dumbbell whose neck is the subject's
    // own colour costs nothing to walk through, so the far weight joins the
    // matte. If this ever stops leaking, the algorithm changed and the note's
    // §2 ceiling — and the correction loop that exists because of it — needs
    // rereading.
    let (w, h) = (160u32, 96u32);
    let inside = |x: u32, y: u32| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let near = (fx - 45.0).powi(2) + (fy - 48.0).powi(2) <= 24.0f32.powi(2);
        let far = (fx - 115.0).powi(2) + (fy - 48.0).powi(2) <= 24.0f32.powi(2);
        let neck = (44.0..=116.0).contains(&fx) && (fy - 48.0).abs() <= 6.0;
        (near, far, neck)
    };
    let rgb = frame_from(w, h, |x, y| {
        let (near, far, neck) = inside(x, y);
        if near || far || neck {
            tint(SUBJECT, x as i32, y as i32, 0.04)
        } else {
            tint(BACKDROP, x as i32, y as i32, 0.04)
        }
    });
    let seeds = base_seeds(
        w,
        h,
        &[stroke(
            &[(40.0, 48.0), (50.0, 48.0)],
            3.0,
            StrokeKind::Foreground,
            0,
        )],
    )
    .unwrap();
    let matte = solve_one(w, h, &rgb, &seeds);
    let far_centre = matte[(48 * w as usize) + 115];
    assert!(
        far_centre > 0.5,
        "the far weight stopped leaking (α = {far_centre}); the pinned ceiling moved"
    );
}

// ---------------------------------------------------------------------------
// Propagation: a written-down analytic flow, no lumit-flow anywhere
// ---------------------------------------------------------------------------

const PW: u32 = 160;
const PH: u32 = 96;
const PR: f32 = 18.0;
const PCY: f32 = 48.0;
const FRAMES: usize = 31;

/// Two pixels a frame — inside the note's ≤ 8 px/frame test motion.
fn subject_centre(t: usize) -> (f32, f32) {
    (40.0 + 2.0 * t as f32, PCY)
}

fn in_subject(t: usize, x: u32, y: u32) -> bool {
    let (cx, cy) = subject_centre(t);
    let dx = x as f32 + 0.5 - cx;
    let dy = y as f32 + 0.5 - cy;
    dx * dx + dy * dy <= PR * PR
}

fn moving_frame(t: usize) -> Vec<f32> {
    let (cx, cy) = subject_centre(t);
    frame_from(PW, PH, |x, y| {
        if in_subject(t, x, y) {
            // The subject's texture travels with it, as a real one would.
            let lx = (x as f32 - cx).round() as i32;
            let ly = (y as f32 - cy).round() as i32;
            tint(SUBJECT, lx + 512, ly + 512, 0.06)
        } else {
            tint(BACKDROP, x as i32, y as i32, 0.06)
        }
    })
}

fn moving_truth(t: usize) -> Vec<bool> {
    let mut truth = Vec::with_capacity((PW as usize) * (PH as usize));
    for y in 0..PH {
        for x in 0..PW {
            truth.push(in_subject(t, x, y));
        }
    }
    truth
}

/// The flow from frame `to` back to frame `from`: the subject's own motion
/// inside the subject, nothing outside, which is exactly the truth of this
/// synthetic shot.
fn moving_flow(to: usize, from: usize) -> (Vec<f32>, Vec<u8>, Vec<f32>) {
    let n = (PW as usize) * (PH as usize);
    let mut flow = vec![0.0f32; n * 2];
    let d = subject_centre(from).0 - subject_centre(to).0;
    for y in 0..PH {
        for x in 0..PW {
            if in_subject(to, x, y) {
                let i = ((y * PW + x) as usize) * 2;
                flow[i] = d;
            }
        }
    }
    (flow, vec![1u8; n], vec![1.0f32; n])
}

struct Shot<'a> {
    frames: usize,
    width: u32,
    height: u32,
    base: usize,
    strokes: &'a [RotoStroke],
}

/// Propagate outward from the base, exactly as §3 has it: warp, seed, stamp
/// the frame's own strokes over the warped seeds, solve.
fn propagate<Fr, Fl>(shot: &Shot<'_>, frame_at: Fr, flow_at: Fl) -> Vec<Vec<f32>>
where
    Fr: Fn(usize) -> Vec<f32>,
    Fl: Fn(usize, usize) -> (Vec<f32>, Vec<u8>, Vec<f32>),
{
    let (w, h) = (shot.width, shot.height);
    let settings = RotoSettings::default();
    let mut solver = RotoSolver::new(settings);
    let n = (w as usize) * (h as usize);
    let mut mattes = vec![vec![0.0f32; n]; shot.frames];
    let mut seeds = Seeds::new(w, h).unwrap();

    let on = |t: usize| -> Vec<RotoStroke> {
        shot.strokes
            .iter()
            .filter(|s| s.frame == t as i64)
            .cloned()
            .collect()
    };

    let base_rgb = frame_at(shot.base);
    let base = base_seeds(w, h, &on(shot.base)).unwrap();
    solver
        .solve(
            FrameRgb::new(&base_rgb, w, h).unwrap(),
            &base,
            &mut mattes[shot.base],
        )
        .unwrap();

    let mut step = |mattes: &mut Vec<Vec<f32>>, solver: &mut RotoSolver, to: usize, from: usize| {
        let prev = mattes[from].clone();
        let (flow, validity, confidence) = flow_at(to, from);
        let field = FlowField::new(&flow, &validity, &confidence, w, h).unwrap();
        warp_and_seed(&prev, &field, settings.confidence_floor, &mut seeds).unwrap();
        seeds.stamp_all(&on(to));
        let rgb = frame_at(to);
        solver
            .solve(FrameRgb::new(&rgb, w, h).unwrap(), &seeds, &mut mattes[to])
            .map_err(|e| format!("frame {to}: {e}"))
            .unwrap();
    };

    for t in shot.base + 1..shot.frames {
        step(&mut mattes, &mut solver, t, t - 1);
    }
    for t in (0..shot.base).rev() {
        step(&mut mattes, &mut solver, t, t + 1);
    }
    mattes
}

#[test]
fn a_translating_subject_survives_thirty_frames() {
    let strokes = [stroke(
        &[(30.0, PCY), (50.0, PCY)],
        3.0,
        StrokeKind::Foreground,
        0,
    )];
    let shot = Shot {
        frames: FRAMES,
        width: PW,
        height: PH,
        base: 0,
        strokes: &strokes,
    };
    let mattes = propagate(&shot, moving_frame, moving_flow);
    for (t, matte) in mattes.iter().enumerate() {
        let score = iou(matte, &moving_truth(t));
        assert!(score >= 0.95, "frame {t} scored {score}");
    }
}

#[test]
fn a_base_in_the_middle_solves_both_directions() {
    let base = 15usize;
    let (cx, _) = subject_centre(base);
    let strokes = [stroke(
        &[(cx - 10.0, PCY), (cx + 10.0, PCY)],
        3.0,
        StrokeKind::Foreground,
        base as i64,
    )];
    let shot = Shot {
        frames: FRAMES,
        width: PW,
        height: PH,
        base,
        strokes: &strokes,
    };
    let mattes = propagate(&shot, moving_frame, moving_flow);
    for t in [0usize, 7, base, 23, FRAMES - 1] {
        let score = iou(&mattes[t], &moving_truth(t));
        assert!(score >= 0.95, "frame {t} scored {score}");
    }
}

#[test]
fn two_runs_are_bit_identical() {
    let strokes = [stroke(
        &[(30.0, PCY), (50.0, PCY)],
        3.0,
        StrokeKind::Foreground,
        0,
    )];
    let shot = Shot {
        frames: FRAMES,
        width: PW,
        height: PH,
        base: 0,
        strokes: &strokes,
    };
    let a = propagate(&shot, moving_frame, moving_flow);
    let b = propagate(&shot, moving_frame, moving_flow);
    for (t, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(
            x.iter()
                .zip(y.iter())
                .all(|(p, q)| p.to_bits() == q.to_bits()),
            "frame {t} differed between runs"
        );
    }
}

// ---------------------------------------------------------------------------
// §10.3 — an occluder crossing in front
// ---------------------------------------------------------------------------

const BAR: [f32; 3] = [0.05, 0.05, 0.06];

fn bar_x(t: usize) -> f32 {
    20.0 + 4.0 * t as f32
}

/// A pole hanging into frame from the top, crossing the subject's upper half.
///
/// Two things about its shape are load-bearing rather than convenient, and both
/// are honest limits of a seeded segmentation. It reaches the top of the frame,
/// so the pole itself is joined to the backdrop and can be reached by a
/// background seed; an occluder that floats *inside* the subject with no seed
/// on it is absorbed, because nothing in the picture says otherwise. And it
/// stops at the subject's middle, so the subject stays in one piece; an
/// occluder that crosses it completely splits it in two, and the half with no
/// seeds cannot be reached from the half that has them. Either case is a
/// correction stroke — §6's loop — not a defect in the arithmetic.
fn in_bar(t: usize, x: u32, y: u32) -> bool {
    let fx = x as f32 + 0.5;
    let fy = y as f32 + 0.5;
    (fx - bar_x(t)).abs() <= 5.0 && fy <= PCY
}

fn occluded_frame(t: usize) -> Vec<f32> {
    let (cx, cy) = subject_centre(t);
    frame_from(PW, PH, |x, y| {
        if in_bar(t, x, y) {
            tint(
                BAR,
                x as i32 - bar_x(t).round() as i32 + 512,
                y as i32,
                0.02,
            )
        } else if in_subject(t, x, y) {
            let lx = (x as f32 - cx).round() as i32;
            let ly = (y as f32 - cy).round() as i32;
            tint(SUBJECT, lx + 512, ly + 512, 0.06)
        } else {
            tint(BACKDROP, x as i32, y as i32, 0.06)
        }
    })
}

fn occluded_truth(t: usize) -> Vec<bool> {
    let mut truth = Vec::with_capacity((PW as usize) * (PH as usize));
    for y in 0..PH {
        for x in 0..PW {
            truth.push(in_subject(t, x, y) && !in_bar(t, x, y));
        }
    }
    truth
}

/// The stick moves faster than the subject; along its outline the flow has
/// nothing honest to say, so the confidence goes to zero in a collar there and
/// those pixels seed nothing — the rule this test exists to exercise.
fn occluded_flow(to: usize, from: usize) -> (Vec<f32>, Vec<u8>, Vec<f32>) {
    let n = (PW as usize) * (PH as usize);
    let mut flow = vec![0.0f32; n * 2];
    let mut confidence = vec![1.0f32; n];
    let subject_d = subject_centre(from).0 - subject_centre(to).0;
    let bar_d = bar_x(from) - bar_x(to);
    for y in 0..PH {
        for x in 0..PW {
            let i = (y * PW + x) as usize;
            if in_bar(to, x, y) {
                flow[i * 2] = bar_d;
            } else if in_subject(to, x, y) {
                flow[i * 2] = subject_d;
            }
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let dilated = (fx - bar_x(to)).abs() <= 7.0 && fy <= PCY + 2.0;
            let eroded = (fx - bar_x(to)).abs() <= 3.0 && fy <= PCY - 2.0;
            if dilated && !eroded {
                confidence[i] = 0.0;
            }
        }
    }
    (flow, vec![1u8; n], confidence)
}

#[test]
fn an_occluder_crossing_neither_leaks_nor_loses_the_subject() {
    let strokes = [stroke(
        &[(30.0, PCY), (50.0, PCY)],
        3.0,
        StrokeKind::Foreground,
        0,
    )];
    let shot = Shot {
        frames: FRAMES,
        width: PW,
        height: PH,
        base: 0,
        strokes: &strokes,
    };
    let mattes = propagate(&shot, occluded_frame, occluded_flow);
    for (t, matte) in mattes.iter().enumerate() {
        let score = iou(matte, &occluded_truth(t));
        assert!(score >= 0.90, "frame {t} scored {score}");
        // Nothing of the occluder away from the subject may be claimed.
        let mut claimed = 0usize;
        for y in 0..PH {
            for x in 0..PW {
                if in_bar(t, x, y)
                    && !in_subject(t, x, y)
                    && mattes[t][(y * PW + x) as usize] >= 0.5
                {
                    claimed += 1;
                }
            }
        }
        assert!(
            claimed < 40,
            "frame {t} leaked {claimed} px onto the occluder"
        );
    }
    // And the subject is whole again once the bar has gone past it.
    let last = iou(&mattes[FRAMES - 1], &occluded_truth(FRAMES - 1));
    assert!(last >= 0.95, "the subject did not recover: {last}");
}

// ---------------------------------------------------------------------------
// §10.4 — the correction loop
// ---------------------------------------------------------------------------

const CORRECTION: usize = 12;

/// A third colour standing still in the backdrop, below the subject's path:
/// the thing a careless first stroke claims, and — because its edges are real
/// colour steps — the thing that stays claimed until somebody says otherwise.
const DISTRACTOR: [f32; 3] = [0.55, 0.20, 0.60];
const DISTRACTOR_AT: (f32, f32) = (40.0, 76.0);

fn in_distractor(x: u32, y: u32) -> bool {
    let dx = x as f32 + 0.5 - DISTRACTOR_AT.0;
    let dy = y as f32 + 0.5 - DISTRACTOR_AT.1;
    dx * dx + dy * dy <= 10.0 * 10.0
}

fn flawed_frame(t: usize) -> Vec<f32> {
    let (cx, cy) = subject_centre(t);
    frame_from(PW, PH, |x, y| {
        if in_distractor(x, y) {
            tint(DISTRACTOR, x as i32, y as i32, 0.06)
        } else if in_subject(t, x, y) {
            let lx = (x as f32 - cx).round() as i32;
            let ly = (y as f32 - cy).round() as i32;
            tint(SUBJECT, lx + 512, ly + 512, 0.06)
        } else {
            tint(BACKDROP, x as i32, y as i32, 0.06)
        }
    })
}

#[test]
fn a_correction_carries_forward_and_leaves_the_prefix_alone() {
    // The note's own fixture: a deliberately wrong stroke at the base — a
    // background claim laid across the bottom of the subject — which the
    // propagation then carries faithfully, because a warped seed is as good as
    // any other seed. The user notices at frame 12 and paints over it.
    let base = [
        stroke(&[(30.0, PCY), (50.0, PCY)], 3.0, StrokeKind::Foreground, 0),
        // The wrong one: a subject claim laid on a backdrop object, which the
        // propagation then carries faithfully, because a warped seed is as good
        // as any other seed.
        stroke(
            &[(DISTRACTOR_AT.0, DISTRACTOR_AT.1)],
            3.0,
            StrokeKind::Foreground,
            0,
        ),
    ];
    let mut corrected = base.to_vec();
    corrected.push(stroke(
        &[(DISTRACTOR_AT.0, DISTRACTOR_AT.1)],
        13.0,
        StrokeKind::Background,
        CORRECTION as i64,
    ));

    let before = propagate(
        &Shot {
            frames: FRAMES,
            width: PW,
            height: PH,
            base: 0,
            strokes: &base,
        },
        flawed_frame,
        moving_flow,
    );
    let after = propagate(
        &Shot {
            frames: FRAMES,
            width: PW,
            height: PH,
            base: 0,
            strokes: &corrected,
        },
        flawed_frame,
        moving_flow,
    );

    // Everything between the base and the correction is untouched, to the bit:
    // influence flows outward from the base, and this is that rule asserted
    // from the arithmetic's side rather than the cache's.
    for t in 0..CORRECTION {
        assert!(
            before[t]
                .iter()
                .zip(after[t].iter())
                .all(|(p, q)| p.to_bits() == q.to_bits()),
            "frame {t} moved, and it is on the base's side of the correction"
        );
    }

    let last = FRAMES - 1;
    let wrong = iou(&before[last], &moving_truth(last));
    let fixed = iou(&after[last], &moving_truth(last));
    assert!(wrong < 0.85, "the wrong stroke did no damage: {wrong}");
    assert!(fixed > wrong + 0.05, "no improvement: {wrong} → {fixed}");
    assert!(
        fixed >= 0.95,
        "the correction did not carry forward: {fixed}"
    );
}

// ---------------------------------------------------------------------------
// §10.5 — the refine edge
// ---------------------------------------------------------------------------

/// A hard edge blurred by an explicit normalised Gaussian: the ground-truth
/// alpha, and the composite the filter has to recover it from.
fn feathered_edge(w: u32, h: u32, sigma: f32) -> (Vec<f32>, Vec<f32>) {
    let radius = (sigma * 3.0).ceil() as i32;
    let mut kernel = Vec::new();
    let mut total = 0.0f32;
    for k in -radius..=radius {
        let v = (-(k as f32 * k as f32) / (2.0 * sigma * sigma)).exp();
        kernel.push(v);
        total += v;
    }
    for v in kernel.iter_mut() {
        *v /= total;
    }
    let edge = (w / 2) as i32;
    let mut alpha = vec![0.0f32; (w as usize) * (h as usize)];
    for y in 0..h {
        for x in 0..w {
            let mut a = 0.0f32;
            for (j, k) in kernel.iter().enumerate() {
                let sx = x as i32 + j as i32 - radius;
                if sx < edge {
                    a += *k;
                }
            }
            alpha[(y * w + x) as usize] = a;
        }
    }
    let rgb = frame_from(w, h, |x, y| {
        let a = alpha[(y * w + x) as usize];
        let f = tint(SUBJECT, x as i32, y as i32, 0.04);
        let b = tint(BACKDROP, x as i32, y as i32, 0.04);
        [
            f[0] * a + b[0] * (1.0 - a),
            f[1] * a + b[1] * (1.0 - a),
            f[2] * a + b[2] * (1.0 - a),
        ]
    });
    (rgb, alpha)
}

fn mse(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = 0.0f64;
    for (p, q) in a.iter().zip(b.iter()) {
        sum += f64::from((p - q) * (p - q));
    }
    (sum / a.len().max(1) as f64) as f32
}

#[test]
fn the_refine_band_recovers_a_feathered_edge() {
    let (w, h) = (96u32, 64u32);
    let (rgb, truth) = feathered_edge(w, h, 2.5);
    let seeds = base_seeds(
        w,
        h,
        &[
            stroke(
                &[(18.0, 4.0), (18.0, 60.0)],
                16.0,
                StrokeKind::Foreground,
                0,
            ),
            stroke(
                &[(78.0, 4.0), (78.0, 60.0)],
                16.0,
                StrokeKind::Background,
                0,
            ),
        ],
    )
    .unwrap();
    let mut solver = RotoSolver::new(RotoSettings::default());
    let mut matte = vec![0.0f32; (w as usize) * (h as usize)];
    solver
        .solve(FrameRgb::new(&rgb, w, h).unwrap(), &seeds, &mut matte)
        .unwrap();
    let snapped: Vec<f32> = solver
        .alpha_raw()
        .iter()
        .map(|a| if *a > 0.5 { 1.0 } else { 0.0 })
        .collect();

    let filtered_mse = mse(&matte, &truth);
    let snapped_mse = mse(&snapped, &truth);
    assert!(
        filtered_mse < snapped_mse,
        "the filter did not beat the snap: {filtered_mse} against {snapped_mse}"
    );
    // Pinned: what this edge recovers to today (0.0017), with a little room.
    // It may only fall.
    assert!(filtered_mse < 0.0020, "MSE rose to {filtered_mse}");
}

#[test]
fn a_refine_stroke_opens_the_band_where_it_is_painted() {
    let (w, h) = (96u32, 64u32);
    let (rgb, _) = feathered_edge(w, h, 2.5);
    let base = [
        stroke(
            &[(18.0, 4.0), (18.0, 60.0)],
            16.0,
            StrokeKind::Foreground,
            0,
        ),
        stroke(
            &[(78.0, 4.0), (78.0, 60.0)],
            16.0,
            StrokeKind::Background,
            0,
        ),
    ];
    let plain = solve_one(w, h, &rgb, &base_seeds(w, h, &base).unwrap());
    // Well inside the foreground, where the band never reaches: the snap holds.
    let probe = (32 * w + 8) as usize;
    assert_eq!(plain[probe], 1.0);

    let mut widened = base.to_vec();
    widened.push(stroke(&[(8.0, 32.0)], 4.0, StrokeKind::Refine, 0));
    let refined = solve_one(w, h, &rgb, &base_seeds(w, h, &widened).unwrap());
    assert!(
        refined[probe] < 1.0,
        "the refine stroke did not open the band"
    );
    // And only where it was painted.
    let far = (32 * w + 24) as usize;
    assert_eq!(refined[far], plain[far]);
}

// ---------------------------------------------------------------------------
// Seeds, erosion and the refusals
// ---------------------------------------------------------------------------

#[test]
fn the_border_ring_is_the_default_background() {
    let seeds = base_seeds(
        32,
        32,
        &[stroke(&[(16.0, 16.0)], 2.0, StrokeKind::Foreground, 0)],
    )
    .unwrap();
    assert_eq!(seeds.at(0), Seed::Background);
    assert_eq!(seeds.at((16 * 32 + 16) as usize), Seed::Foreground);
    // A user who did paint background gets no ring.
    let painted = base_seeds(
        32,
        32,
        &[
            stroke(&[(16.0, 16.0)], 2.0, StrokeKind::Foreground, 0),
            stroke(&[(4.0, 28.0)], 2.0, StrokeKind::Background, 0),
        ],
    )
    .unwrap();
    assert_eq!(painted.at(0), Seed::None);
}

#[test]
fn a_later_stroke_wins_the_overlap() {
    let mut seeds = Seeds::new(32, 32).unwrap();
    seeds.stamp_all(&[
        stroke(&[(16.0, 16.0)], 4.0, StrokeKind::Foreground, 0),
        stroke(&[(16.0, 16.0)], 2.0, StrokeKind::Background, 0),
    ]);
    assert_eq!(seeds.at((16 * 32 + 16) as usize), Seed::Background);
    assert_eq!(seeds.at((16 * 32 + 19) as usize), Seed::Foreground);
}

#[test]
fn warped_seeds_are_eroded_and_low_confidence_seeds_nothing() {
    let (w, h) = (32u32, 32u32);
    let n = (w * h) as usize;
    // A solid square of matte, still.
    let mut prev = vec![0.0f32; n];
    for y in 10..22 {
        for x in 10..22 {
            prev[(y * w + x) as usize] = 1.0;
        }
    }
    let flow = vec![0.0f32; n * 2];
    let validity = vec![1u8; n];
    let mut confidence = vec![1.0f32; n];
    // One untrusted column across the square.
    for y in 0..h {
        confidence[(y * w + 16) as usize] = 0.0;
    }
    let field = FlowField::new(&flow, &validity, &confidence, w, h).unwrap();
    let mut seeds = Seeds::new(w, h).unwrap();
    warp_and_seed(&prev, &field, 0.5, &mut seeds).unwrap();

    // Two pixels in from the square's edge is where the foreground seeds start.
    assert_eq!(seeds.at((15 * w + 12) as usize), Seed::Foreground);
    assert_eq!(seeds.at((15 * w + 10) as usize), Seed::None);
    // The untrusted column seeds nothing, and takes its neighbours' erosion
    // with it.
    assert_eq!(seeds.at((15 * w + 16) as usize), Seed::None);
    assert_eq!(seeds.at((15 * w + 15) as usize), Seed::None);
    // Well outside is background, eroded from the square by the same two.
    assert_eq!(seeds.at(0), Seed::Background);
    assert_eq!(seeds.at((9 * w + 15) as usize), Seed::None);
}

#[test]
fn a_solve_without_both_seed_sets_is_refused() {
    let (w, h) = (16u32, 16u32);
    let rgb = vec![0.5f32; (w * h * 3) as usize];
    let mut seeds = Seeds::new(w, h).unwrap();
    seeds.stamp_all(&[stroke(&[(8.0, 8.0)], 2.0, StrokeKind::Foreground, 0)]);
    let mut out = vec![0.0f32; (w * h) as usize];
    let mut solver = RotoSolver::new(RotoSettings::default());
    assert_eq!(
        solver.solve(FrameRgb::new(&rgb, w, h).unwrap(), &seeds, &mut out),
        Err(RotoError::NoSeeds)
    );
}

#[test]
fn a_plane_of_the_wrong_length_is_refused() {
    assert!(matches!(
        FrameRgb::new(&[0.0; 10], 4, 4),
        Err(RotoError::PlaneSize { .. })
    ));
    assert!(matches!(
        FrameRgb::new(&[], 0, 4),
        Err(RotoError::BadSize { .. })
    ));
    let flow = vec![0.0f32; 32];
    let validity = vec![1u8; 16];
    let confidence = vec![1.0f32; 16];
    assert!(FlowField::new(&flow, &validity, &confidence, 4, 4).is_ok());
    assert!(FlowField::new(&flow, &validity, &confidence, 8, 2).is_ok());
    assert!(matches!(
        FlowField::new(&flow, &validity, &confidence, 5, 4),
        Err(RotoError::PlaneSize { .. })
    ));
}
