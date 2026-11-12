#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use proptest::prelude::*;

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

fn key(t: Rational, v: f64, side: SideInterp) -> Keyframe {
    Keyframe {
        time: t,
        value: v,
        interp_in: side,
        interp_out: side,
    }
}

/// **A fade is a pair of ordinary keys** (docs/09 §6): between the
/// −∞ knee and the level the property already held, shaped from the
/// easing vocabulary that already exists, with everything outside the
/// span left alone.
#[test]
fn a_fade_is_an_eased_pair_that_leaves_the_rest_of_the_curve_alone() {
    let floor = -100.0;
    let level = Property::fixed(-3.0);

    // Fade in: silence at the start of the span, the level at its end.
    let faded = level
        .with_fade(rat(0, 1), rat(1, 1), FadeShape::Ease, floor, true)
        .expect("a fade in");
    assert!((faded.value_at(0.0) - floor).abs() < 1e-9);
    assert!((faded.value_at(1.0) - -3.0).abs() < 1e-9);
    assert!(
        faded.value_at(5.0) == -3.0,
        "past the last key a property holds its level"
    );
    let Animation::Keyframed(keys) = &faded.animation else {
        panic!("a fade is keyframed");
    };
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].interp_out, EASY_EASE, "Ease eases both keys");
    assert_eq!(keys[1].interp_in, EASY_EASE);

    // Fade out: the level at the start of the span, silence at its end.
    let out = level
        .with_fade(rat(2, 1), rat(3, 1), FadeShape::Linear, floor, false)
        .expect("a fade out");
    assert!((out.value_at(2.0) - -3.0).abs() < 1e-9);
    assert!((out.value_at(3.0) - floor).abs() < 1e-9);
    let Animation::Keyframed(keys) = &out.animation else {
        panic!("keyframed");
    };
    assert!(keys.iter().all(|k| k.interp_in == SideInterp::Linear));

    // Exponential eases the loud key and leaves the silent one straight,
    // so the level hangs where it was and then falls away.
    let exp = level
        .with_fade(rat(0, 1), rat(1, 1), FadeShape::Exponential, floor, false)
        .expect("a fade out");
    let Animation::Keyframed(keys) = &exp.animation else {
        panic!("keyframed");
    };
    assert_eq!(keys[0].interp_out, EASY_EASE, "the loud key is eased");
    assert_eq!(keys[1].interp_in, SideInterp::Linear);
    assert!(
        exp.value_at(0.25) > out.value_at(2.25),
        "an exponential fade is still louder a quarter of the way down"
    );

    // The two halves coexist: fading a property that already has a fade
    // in leaves those keys where they are.
    let both = faded
        .with_fade(rat(3, 1), rat(4, 1), FadeShape::Ease, floor, false)
        .expect("a fade out over a fade in");
    let Animation::Keyframed(keys) = &both.animation else {
        panic!("keyframed");
    };
    assert_eq!(keys.len(), 4, "two keys each, in time order");
    assert!(keys
        .windows(2)
        .all(|w| w[0].time.to_f64() < w[1].time.to_f64()));
    assert!((both.value_at(0.0) - floor).abs() < 1e-9);
    assert!((both.value_at(4.0) - floor).abs() < 1e-9);
    assert!(
        (both.value_at(2.0) - -3.0).abs() < 1e-9,
        "full level between"
    );

    // Running one twice over the same span reshapes it rather than
    // stacking a second curve on top.
    let again = both
        .with_fade(rat(3, 1), rat(4, 1), FadeShape::Linear, floor, false)
        .expect("a reshape");
    let Animation::Keyframed(keys) = &again.animation else {
        panic!("keyframed");
    };
    assert_eq!(keys.len(), 4, "still four keys, not six");

    // Refusals: no span, and an expression (a fade would rewrite what
    // somebody typed).
    assert!(level
        .with_fade(rat(1, 1), rat(1, 1), FadeShape::Ease, floor, true)
        .is_none());
    let expression = Property {
        animation: Animation::Expression("time".into()),
        extra: serde_json::Map::new(),
    };
    assert!(expression
        .with_fade(rat(0, 1), rat(1, 1), FadeShape::Ease, floor, true)
        .is_none());
}

#[test]
fn evaluate_speed_is_the_exact_derivative() {
    // Linear span 0→10 over 1 s: constant slope 10; flat/held outside.
    let lin = [
        key(rat(0, 1), 0.0, SideInterp::Linear),
        key(rat(1, 1), 10.0, SideInterp::Linear),
    ];
    assert!((evaluate_speed(&lin, 0.5).unwrap() - 10.0).abs() < 1e-9);
    assert_eq!(evaluate_speed(&lin, -1.0), Some(0.0));
    assert_eq!(evaluate_speed(&lin, 2.0), Some(0.0));

    // Easy-ease span: speed ~0 at the keys, positive in the middle, and the
    // analytic derivative agrees with a central finite difference of value.
    let ease = [
        key(rat(0, 1), 0.0, EASY_EASE),
        key(rat(1, 1), 10.0, EASY_EASE),
    ];
    let (start, middle) = (
        evaluate_speed(&ease, 0.02).unwrap(),
        evaluate_speed(&ease, 0.5).unwrap(),
    );
    assert!((0.0..2.0).contains(&start) && start < middle * 0.2); // slow start
    assert!((middle - 15.0).abs() < 1e-6); // fast middle (peak of easy-ease)
    for &t in &[0.15_f64, 0.4, 0.6, 0.85] {
        let h = 1e-4;
        let fd = (evaluate(&ease, t + h).unwrap() - evaluate(&ease, t - h).unwrap()) / (2.0 * h);
        assert!(
            (evaluate_speed(&ease, t).unwrap() - fd).abs() < 1e-3,
            "at t={t}: analytic {} vs finite {fd}",
            evaluate_speed(&ease, t).unwrap()
        );
    }
}

#[test]
fn linear_hold_and_clamping() {
    let keys = [
        key(rat(0, 1), 0.0, SideInterp::Linear),
        key(rat(1, 1), 10.0, SideInterp::Linear),
        Keyframe {
            time: rat(2, 1),
            value: 20.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Hold,
        },
        key(rat(3, 1), 5.0, SideInterp::Linear),
    ];
    assert_eq!(evaluate(&keys, -1.0), Some(0.0)); // clamp before
    assert_eq!(evaluate(&keys, 0.5), Some(5.0)); // linear
    assert_eq!(evaluate(&keys, 2.5), Some(20.0)); // hold-out wins the span
    assert_eq!(evaluate(&keys, 9.0), Some(5.0)); // clamp after
}

#[test]
fn easy_ease_is_flat_at_both_keys_and_monotone() {
    let keys = [
        key(rat(0, 1), 0.0, EASY_EASE),
        key(rat(1, 1), 100.0, EASY_EASE),
    ];
    // Flat tangents: near the keys the value barely moves.
    let near0 = evaluate(&keys, 0.01).unwrap();
    let near1 = evaluate(&keys, 0.99).unwrap();
    assert!(near0 < 0.5, "start not flat: {near0}");
    assert!(near1 > 99.5, "end not flat: {near1}");
    // Midpoint of a symmetric ease is the midpoint value.
    let mid = evaluate(&keys, 0.5).unwrap();
    assert!((mid - 50.0).abs() < 1e-9, "mid {mid}");
    // Monotone in, monotone out.
    let mut prev = f64::MIN;
    for i in 0..=1000 {
        let v = evaluate(&keys, i as f64 / 1000.0).unwrap();
        assert!(v >= prev - 1e-9, "not monotone at {i}: {v} < {prev}");
        prev = v;
    }
}

#[test]
fn full_influence_spike_is_exact_not_explosive() {
    // 100% influence both sides with zero speed: the x-curve has dx=0 at
    // the endpoints — the case that diverges under plain Newton.
    let spike = SideInterp::Bezier {
        speed: 0.0,
        influence: 1.0,
    };
    let keys = [key(rat(0, 1), 0.0, spike), key(rat(1, 1), 1.0, spike)];
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        let v = evaluate(&keys, t).unwrap();
        assert!(v.is_finite());
        assert!((-1e-9..=1.0 + 1e-9).contains(&v), "t={t} v={v}");
    }
    assert!((evaluate(&keys, 0.5).unwrap() - 0.5).abs() < 1e-9);
}

/// **A key inserted mid-span must not move the curve**. Sampled
/// densely across the whole span, before and after, and compared: the razor
/// puts one of these on every retimed layer it cuts, and a cut that changed
/// the speed ramp would be a cut nobody could trust.
#[test]
fn inserting_a_key_preserves_the_curve() {
    for (out_side, in_side) in [
        (EASY_EASE, EASY_EASE),
        (
            SideInterp::Bezier {
                speed: 4.0,
                influence: 0.75,
            },
            SideInterp::Bezier {
                speed: -2.0,
                influence: 0.1,
            },
        ),
        (SideInterp::Linear, EASY_EASE),
        (SideInterp::Linear, SideInterp::Linear),
    ] {
        let mut p = Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: rat(0, 1),
                    value: 0.0,
                    interp_in: SideInterp::Linear,
                    interp_out: out_side,
                },
                Keyframe {
                    time: rat(2, 1),
                    value: 10.0,
                    interp_in: in_side,
                    interp_out: SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        };
        let before: Vec<f64> = (0..=200)
            .map(|i| p.value_at(f64::from(i) / 100.0))
            .collect();

        assert!(p.insert_key_preserving_shape(rat(3, 4)), "a key landed");
        let Animation::Keyframed(keys) = &p.animation else {
            panic!("still keyframed");
        };
        assert_eq!(keys.len(), 3, "one key added, in the middle");
        assert!((keys[1].time.to_f64() - 0.75).abs() < 1e-12);

        for (i, was) in before.iter().enumerate() {
            let t = f64::from(i as u32) / 100.0;
            let now = p.value_at(t);
            assert!(
                (now - was).abs() < 1e-6,
                "at t={t} the curve moved: {was} -> {now}"
            );
        }
    }
}

proptest! {
    /// solve_u(x(u)) == u to 1e-10 over random monotone cubics,
    /// including dx = 0 endpoints (keyframe-eval.md test plan §2).
    #[test]
    fn solve_round_trips(
        b1 in 0.001f64..=1.0,
        b2 in 0.001f64..=1.0,
        s1 in -5.0f64..5.0,
        s2 in -5.0f64..5.0,
        u in 0.0f64..=1.0,
    ) {
        let cubic = CubicSpan::from_ae(0.0, 0.0, 1.0, 1.0, s1, b1, s2, b2);
        let t = CubicSpan::bezier(&cubic.x, u);
        let solved = cubic.solve_u(t);
        let t_back = CubicSpan::bezier(&cubic.x, solved);
        // Compare in x-space: distinct u can map to equal x at flat spots.
        prop_assert!((t_back - t).abs() < 1e-10, "t {t} → u {solved} → {t_back}");
    }

    /// Evaluation stays within the hull of the two key values whenever
    /// both handles point "inward" (no overshoot without overshooting
    /// handles) — and is always finite.
    #[test]
    fn no_spurious_overshoot(
        b1 in 0.001f64..=1.0,
        b2 in 0.001f64..=1.0,
        t in 0.0f64..=1.0,
    ) {
        let keys = [
            key(rat(0,1), 0.0, SideInterp::Bezier { speed: 0.0, influence: b1 }),
            key(rat(1,1), 1.0, SideInterp::Bezier { speed: 0.0, influence: b2 }),
        ];
        let v = evaluate(&keys, t).unwrap();
        prop_assert!(v.is_finite());
        prop_assert!((-1e-9..=1.0 + 1e-9).contains(&v));
    }
}

/// The same budget on a *dense* curve — an imported AE camera arrives with
/// a key per frame, thousands per property. The linear span scan this
/// replaced made each sample cost the whole list (~2,400 keys × 7 camera
/// properties per rendered frame on the real project that found it), so a
/// sparse-only bench held while dense curves missed the gate by orders of
/// magnitude. Fails in minutes rather than milliseconds without bisection.
#[test]
fn million_evaluations_stay_cheap_on_a_dense_curve() {
    let keys: Vec<Keyframe> = (0..2_400)
        .map(|i| key(rat(i, 25), (i % 7) as f64, SideInterp::Linear))
        .collect();
    let start = std::time::Instant::now();
    let mut acc = 0.0;
    for i in 0..1_000_000u32 {
        acc += evaluate(&keys, f64::from(i % 2_399) / 25.0 + 0.017).unwrap_or(0.0);
    }
    let elapsed = start.elapsed();
    assert!(acc.is_finite());
    // The same 40× debug headroom as the sparse bench above.
    assert!(elapsed.as_millis() < 800, "1M dense evals took {elapsed:?}");
}

/// Bisection answers exactly what the linear walk answered, spans and
/// speeds both — sampled between keys, exactly on interior keys, and at
/// both ends — on a curve dense enough that an off-by-one lands on a
/// different span with a different value.
#[test]
fn span_bisection_matches_the_linear_walk() {
    let sides = [
        SideInterp::Linear,
        SideInterp::Hold,
        EASY_EASE,
        SideInterp::Bezier {
            speed: 3.0,
            influence: 0.25,
        },
    ];
    let keys: Vec<Keyframe> = (0..500)
        .map(|i| key(rat(i, 10), ((i * 37) % 11) as f64, sides[(i % 4) as usize]))
        .collect();
    let linear_idx = |t: f64| {
        keys.windows(2)
            .position(|w| t < w[1].time.to_f64())
            .unwrap_or(keys.len() - 2)
    };
    for i in 0..4_990 {
        for t in [f64::from(i) / 100.0, f64::from(i) / 100.0 + 0.003] {
            if t <= keys[0].time.to_f64() || t >= keys[keys.len() - 1].time.to_f64() {
                continue;
            }
            assert_eq!(
                span_index(&keys, t),
                linear_idx(t),
                "span for t = {t} diverged"
            );
        }
    }
}

// -----------------------------------------------------------------------
// Tangent modes — Auto / Clamp / Free (docs/impl/keyframe-eval.md §6).
// -----------------------------------------------------------------------

fn auto(clamped: bool) -> SideInterp {
    SideInterp::Auto {
        clamped,
        speed: 0.0,
        influence: 1.0 / 3.0,
    }
}

/// **Auto recomputes whenever a neighbour moves.** The same key, the same
/// mode, a moved neighbour — a different tangent, and so a different
/// curve. A free side would have gone on reading the speed it stored.
#[test]
fn an_automatic_tangent_follows_a_moved_neighbour() {
    let mut keys = [
        key(rat(0, 1), 0.0, auto(false)),
        key(rat(1, 1), 10.0, auto(false)),
        key(rat(2, 1), 30.0, auto(false)),
    ];
    let before = evaluate(&keys, 1.5).unwrap();
    keys[2].value = 200.0;
    let after = evaluate(&keys, 1.5).unwrap();
    assert!((auto_speed(&keys, 1, false) - 100.0).abs() < 1e-12);
    assert!((after - before).abs() > 1.0);

    // With both sides free the stored speed is all there is, so moving the
    // neighbour's *value* changes the span's ends but not the tangent.
    let free = SideInterp::Bezier {
        speed: 15.0,
        influence: 1.0 / 3.0,
    };
    let frozen = [
        key(rat(0, 1), 0.0, free),
        key(rat(1, 1), 10.0, free),
        key(rat(2, 1), 200.0, free),
    ];
    assert!((side_params(resolved_side(&frozen, 1, true), 0.0).0 - 15.0).abs() < 1e-12);
}

/// **Clamp is Auto with no overshoot.** A peak key — both neighbours
/// below it — gets a flat tangent, so no sample in either span rises above
/// it; the smooth tangent tilts and does overshoot.
#[test]
fn a_clamped_tangent_does_not_overshoot_its_neighbours() {
    let peak = |side: SideInterp| {
        [
            key(rat(0, 1), 0.0, SideInterp::Linear),
            key(rat(1, 1), 10.0, side),
            key(rat(2, 1), 9.0, SideInterp::Linear),
        ]
    };
    let clamped = peak(auto(true));
    assert_eq!(auto_speed(&clamped, 1, true), 0.0);
    for i in 0..=100 {
        let t = i as f64 / 50.0;
        assert!(
            evaluate(&clamped, t).unwrap() <= 10.0 + 1e-9,
            "clamped overshot at t={t}"
        );
    }
    // The unclamped one aims from 0 to 9 across two seconds — uphill past
    // the peak — and does rise above it.
    let smooth = peak(auto(false));
    assert!((auto_speed(&smooth, 1, false) - 4.5).abs() < 1e-12);
    let over = (0..=100)
        .map(|i| evaluate(&smooth, i as f64 / 50.0).unwrap())
        .fold(f64::MIN, f64::max);
    assert!(over > 10.0 + 1e-6, "smooth tangent should overshoot");
}

/// **Free → Auto → Free keeps the custom ease** (the study's explicit
/// bar). The mode switch files the ease inside the automatic side and
/// hands it back untouched.
#[test]
fn a_round_trip_through_auto_keeps_the_custom_ease() {
    let custom = SideInterp::Bezier {
        speed: 7.25,
        influence: 0.82,
    };
    assert_eq!(custom.tangent_mode(), TangentMode::Free);
    let automatic = custom.with_tangent_mode(TangentMode::Auto);
    assert_eq!(automatic.tangent_mode(), TangentMode::Auto);
    let clamped = automatic.with_tangent_mode(TangentMode::Clamp);
    assert_eq!(clamped.tangent_mode(), TangentMode::Clamp);
    assert_eq!(clamped.with_tangent_mode(TangentMode::Free), custom);

    // A straight side has no ease of its own to keep; it returns eased,
    // which is what it has been while automatic.
    let straight = SideInterp::Linear.with_tangent_mode(TangentMode::Auto);
    assert_eq!(
        straight.with_tangent_mode(TangentMode::Free),
        SideInterp::Bezier {
            speed: 0.0,
            influence: 1.0 / 3.0
        }
    );
    // Free asked of an already-free side changes nothing at all.
    assert_eq!(
        SideInterp::Hold.with_tangent_mode(TangentMode::Free),
        SideInterp::Hold
    );
}
