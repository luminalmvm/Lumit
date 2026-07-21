use super::*;
use crate::anim::{Animation, Property};
use crate::model::{Composition, EffectInstance, EffectNamespace, EffectValue, Layer};

// Posterize time (docs/08 §3.25): the held comp time snaps down to the coarser
// grid. The two comp times that share a held frame MUST return the exact same
// tau (that equality is what lets the frame cache dedup them) and never divide
// by zero on a degenerate rate.
#[test]
fn posterize_held_time_snaps_to_the_grid() {
    // 10 fps grid, no phase: every time in [0.3, 0.4) holds at 0.3.
    assert_eq!(posterize_held_time(0.30, 10.0, 0.0), 0.3);
    assert_eq!(posterize_held_time(0.35, 10.0, 0.0), 0.3);
    assert!((posterize_held_time(0.399, 10.0, 0.0) - 0.3).abs() < 1e-9);
    // The next step lands exactly on 0.4.
    assert!((posterize_held_time(0.40, 10.0, 0.0) - 0.4).abs() < 1e-9);
    // Two times sharing a held frame agree bit-for-bit (the dedup property):
    // at 12 fps the cell [4/12, 5/12) holds both 0.34 and 0.40 at 4/12.
    assert_eq!(
        posterize_held_time(0.34, 12.0, 0.0),
        posterize_held_time(0.40, 12.0, 0.0)
    );
    // A phase offset shifts where the steps land.
    assert!((posterize_held_time(0.35, 10.0, 0.05) - 0.35).abs() < 1e-9);
    // A degenerate rate holds nothing and never divides by zero.
    assert_eq!(posterize_held_time(0.42, 0.0, 0.0), 0.42);
    assert_eq!(posterize_held_time(0.42, -5.0, 0.0), 0.42);
}

// stack_posterize finds the effect, resolves its grid, and reports nothing for
// a bypassed stack or a plain one — so a layer with no Posterize pays nothing.
// The Scope choice is gone (K-166): the reach is implied by the carrier.
#[test]
fn stack_posterize_detects_and_resolves() {
    let mut e = instantiate("posterize_time").unwrap();
    // No scope parameter any more (K-166); default rate 12, phase 0.
    assert!(e.params.iter().all(|p| p.id != "scope"));
    let p = stack_posterize(std::slice::from_ref(&e), true, 0.0).unwrap();
    assert_eq!(p.rate, 12.0);
    assert_eq!(p.phase, 0.0);
    for param in &mut e.params {
        if param.id == "rate" {
            param.value = EffectValue::Float(Property::fixed(8.0));
        }
    }
    let p = stack_posterize(std::slice::from_ref(&e), true, 0.0).unwrap();
    assert_eq!(p.rate, 8.0);
    // Bypassed (fx off) or disabled → nothing.
    assert!(stack_posterize(std::slice::from_ref(&e), false, 0.0).is_none());
    e.enabled = false;
    assert!(stack_posterize(std::slice::from_ref(&e), true, 0.0).is_none());
    // A plain stack reports nothing.
    let blur = instantiate("blur").unwrap();
    assert!(stack_posterize(std::slice::from_ref(&blur), true, 0.0).is_none());
}

// this_layer_effect_time (docs/08 §3.25, K-166): any live Posterize holds this
// layer's own stack on the coarse grid; a plain or bypassed stack leaves the
// layer time untouched.
#[test]
fn this_layer_effect_time_holds_the_stack_on_the_grid() {
    let mut e = instantiate("posterize_time").unwrap();
    for p in &mut e.params {
        if p.id == "rate" {
            p.value = EffectValue::Float(Property::fixed(10.0));
        }
    }
    // 10 fps grid, no offset: t = 0.35 holds at 0.3.
    assert!((this_layer_effect_time(std::slice::from_ref(&e), true, 0.35, 0.0) - 0.3).abs() < 1e-9);
    // The hold is computed on comp time `lt + start_offset` and mapped back, so a
    // layer offset by 1.0s still lands its held effects on the same comp grid:
    // held comp time floor(3.5)/10 = 0.3, minus the offset → -0.7.
    assert!(
        (this_layer_effect_time(std::slice::from_ref(&e), true, -0.65, 1.0) - (-0.7)).abs() < 1e-9
    );
    // Bypassed or plain stacks are untouched.
    assert_eq!(
        this_layer_effect_time(std::slice::from_ref(&e), false, 0.35, 0.0),
        0.35
    );
    let blur = instantiate("blur").unwrap();
    assert_eq!(
        this_layer_effect_time(std::slice::from_ref(&blur), true, 0.35, 0.0),
        0.35
    );
}

// posterize_sample_times (docs/08 §3.25): the decode planner's per-layer held
// comp time — the piece that makes Posterize Time step *footage playback*, not
// only comp-driven animation. An Everything-below adjustment holds every layer
// beneath it; a This-layer Posterize holds only its own layer; a plain stack is
// left at the live playhead. This is the FX-1 regression: the sampled time must
// snap to the rate.
#[test]
fn posterize_sample_times_snap_covered_layers_to_the_grid() {
    use crate::model::{LayerKind, Switches, TransformGroup};
    use crate::time::{CompTime, Rational};
    let secs = |n: i64, d: i64| CompTime(Rational::new(n, d).unwrap());
    let layer = |kind: LayerKind, effects: Vec<EffectInstance>| Layer {
        id: uuid::Uuid::now_v7(),
        name: "l".into(),
        kind,
        in_point: secs(0, 1),
        out_point: secs(10, 1),
        start_offset: secs(0, 1),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: crate::anim::Property::zero(),
        blend: Default::default(),
        masks: Vec::new(),
        effects,
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let footage = |effects| {
        layer(
            LayerKind::Solid {
                def: uuid::Uuid::now_v7(),
            },
            effects,
        )
    };

    // Everything-below Posterize at 10 fps on an adjustment (index 0, the top),
    // two plain layers beneath. At t = 0.37 the layers below snap to the 0.3
    // grid; the adjustment carrying the effect is not held by its own effect.
    let mut post = instantiate("posterize_time").unwrap();
    for p in &mut post.params {
        if p.id == "rate" {
            p.value = EffectValue::Float(Property::fixed(10.0));
        }
    }
    let layers = vec![
        layer(LayerKind::Adjustment, vec![post.clone()]),
        footage(vec![]),
        footage(vec![]),
    ];
    let st = posterize_sample_times(&layers, 0.37);
    // Every layer below the adjustment snaps to the 10 fps grid. The adjustment's
    // own sample time snaps too, but that is unused (it has no source to decode).
    assert!((st[0] - 0.3).abs() < 1e-9);
    assert!(
        (st[1] - 0.3).abs() < 1e-9,
        "a layer below snaps to the 10 fps grid"
    );
    assert!((st[2] - 0.3).abs() < 1e-9);

    // K-166: a Posterize on a plain (footage) layer holds ONLY that layer's own
    // sampling — the reach is implied by the carrier, so a non-adjustment
    // carrier never holds the layers beneath it.
    let on_footage = vec![footage(vec![post.clone()]), footage(vec![])];
    let stf = posterize_sample_times(&on_footage, 0.37);
    assert!((stf[0] - 0.3).abs() < 1e-9, "the posterised footage snaps");
    assert!(
        (stf[1] - 0.37).abs() < 1e-9,
        "a layer below a plain-layer Posterize stays live (K-166)"
    );

    // No live Posterize → every layer stays at the live playhead.
    let st = posterize_sample_times(&[footage(vec![]), footage(vec![])], 0.37);
    assert!(st.iter().all(|&s| (s - 0.37).abs() < 1e-9));
}

// stack_accumulation_mb (docs/08 §3.26) finds the effect, resolves its shutter
// and Mix, and derives the centred sub-frame offsets; a bypassed or plain stack
// reports nothing, and it resolves to no per-pixel op (executed at the
// orchestration layer, like Posterize).
#[test]
fn stack_accumulation_mb_detects_resolves_and_offsets() {
    let e = instantiate("accumulation_mb").unwrap();
    let p = stack_accumulation_mb(std::slice::from_ref(&e), true, 0.0).unwrap();
    assert_eq!(p.samples, 8); // default
    assert_eq!(p.shutter_angle, 180.0);
    assert_eq!(p.shutter_phase, -90.0);
    assert!((p.mix - 1.0).abs() < 1e-9);
    // Eight centred sub-frame offsets across the open shutter (the shared
    // per-layer motion-blur shutter maths).
    assert_eq!(p.sample_offsets().len(), 8);
    // Force on all layers defaults off, so the sample renders force no per-layer
    // motion blur (FX-18).
    assert!(!p.force_all);
    assert!(p.forced_layer_mb().is_none());
    // With it on, the forced shutter carries this effect's own angle/phase/
    // samples and is enabled, so every layer smears in each sample render.
    let forced = AccumulationMbParams {
        force_all: true,
        ..p
    };
    let mb = forced
        .forced_layer_mb()
        .expect("force_all yields a shutter");
    assert!(mb.enabled);
    assert_eq!(mb.shutter_angle, p.shutter_angle);
    assert_eq!(mb.shutter_phase, p.shutter_phase);
    assert_eq!(mb.samples, p.samples);
    // A degenerate single sample is no blur — empty offsets, so the caller falls
    // back to the plain frame-time composite.
    let one = AccumulationMbParams { samples: 1, ..p };
    assert!(one.sample_offsets().is_empty());
    // Bypassed or a plain stack report nothing.
    assert!(stack_accumulation_mb(std::slice::from_ref(&e), false, 0.0).is_none());
    let blur = instantiate("blur").unwrap();
    assert!(stack_accumulation_mb(std::slice::from_ref(&blur), true, 0.0).is_none());
    // No per-pixel op: it never reaches a kernel.
    assert!(resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE
    )
    .is_empty());
}

// A Posterize Time effect has no per-pixel op: it must resolve to nothing (it is
// executed at the orchestration layer, not in run_ops), exactly like a
// placeholder — so it never reaches a kernel.
#[test]
fn posterize_resolves_to_no_op() {
    let e = instantiate("posterize_time").unwrap();
    assert!(resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE
    )
    .is_empty());
}

#[test]
fn instantiate_carries_declared_defaults() {
    // Gaussian blur (match_name "blur", K-137): Radius + Mix only, no Edges
    // control (that stayed on Radial alone).
    let e = instantiate("blur").unwrap();
    assert_eq!(e.effect.match_name, "blur");
    assert_eq!(e.effect.version, 1);
    assert!(e.enabled);
    assert_eq!(e.float_at("radius", 0.0), Some(1.5));
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));
    assert!(
        e.param("edge").is_none(),
        "Gaussian dropped the Edges control"
    );
    // Radial blur keeps the Edges control, defaulting to Repeat (1).
    let radial = instantiate("radial_blur").unwrap();
    assert!(matches!(radial.param("edge"), Some(EffectValue::Choice(1))));
    assert!(instantiate("nonsense").is_none());
}

#[test]
fn resolve_stack_evaluates_converts_and_skips_dead_effects() {
    let mut e = instantiate("blur").unwrap();
    // 1.5% of a 1000px diagonal = 15px.
    let r = resolve_stack(&[e.clone()], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Blur {
            radius_px: 15.0,
            edge: 1,
            mix: 1.0
        }]
    );
    e.enabled = false;
    assert!(resolve_stack(&[e.clone()], 0.0, 1000.0, 1.0, &MarkerContext::NONE).is_empty());
    e.enabled = true;
    e.effect.namespace = EffectNamespace::Placeholder;
    assert!(
        resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE).is_empty(),
        "placeholders render as identity"
    );
}

// docs/impl/temporal-rerender.md §5: in a held/sub-frame re-render an effect
// flagged sample_temporally == false resolves at the true frame time, while the
// rest of the stack samples the held time. resolve_stack_temporal is the
// per-effect time split both the preview and export re-render drive; with the
// two times equal it is byte-identical to resolve_stack (the ordinary render is
// unchanged).
#[test]
fn resolve_stack_temporal_pins_non_sampling_effects_to_the_frame_time() {
    use crate::anim::{Keyframe, SideInterp};
    use crate::time::Rational;
    // A blur whose radius ramps 0%→100% over one second, so a held time and a
    // frame time resolve to visibly different radii.
    let key = |time: Rational, value: f64| Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let ramp = Property {
        animation: Animation::Keyframed(vec![
            key(Rational::ZERO, 0.0),
            key(Rational::new(1, 1).unwrap(), 100.0),
        ]),
        extra: serde_json::Map::new(),
    };
    let mut e = instantiate("blur").unwrap();
    for p in &mut e.params {
        if p.id == "radius" {
            p.value = EffectValue::Float(ramp.clone());
        }
    }
    let radius_of = |r: &[Resolved]| match r.first() {
        Some(Resolved::Blur { radius_px, .. }) => *radius_px,
        _ => panic!("expected a blur"),
    };
    // Sample time 0.2 (radius 20% → 200px of a 1000px diagonal), frame time 0.8
    // (80% → 800px). With the flag ON (the default) the effect samples the held
    // time; with it OFF it holds at the frame time.
    let sampled = resolve_stack_temporal(
        std::slice::from_ref(&e),
        0.2,
        0.8,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert!((radius_of(&sampled) - 200.0).abs() < 0.01);
    e.sample_temporally = false;
    let held = resolve_stack_temporal(
        std::slice::from_ref(&e),
        0.2,
        0.8,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert!((radius_of(&held) - 800.0).abs() < 0.01);
    // Equal times ⇒ byte-identical to resolve_stack (ordinary render unchanged),
    // whatever the flag.
    assert_eq!(
        resolve_stack_temporal(
            std::slice::from_ref(&e),
            0.5,
            0.5,
            1000.0,
            1.0,
            &MarkerContext::NONE
        ),
        resolve_stack(
            std::slice::from_ref(&e),
            0.5,
            1000.0,
            1.0,
            &MarkerContext::NONE
        ),
    );
}

#[test]
fn dof_instantiates_unset_and_resolves_its_floats() {
    let e = instantiate("dof").unwrap();
    assert_eq!(e.effect.match_name, "dof");
    assert_eq!(e.effect.version, 1);
    // A fresh depth reference is unset — the effect is a labelled no-op
    // until a layer is picked (its run_ops depth slot is None, a
    // passthrough), the sanctioned exception the File parameter also takes.
    assert!(matches!(e.param("depth"), Some(EffectValue::Layer(None))));
    assert_eq!(e.layer_ref("depth"), None);
    assert_eq!(e.float_at("focus", 0.0), Some(0.5));
    assert_eq!(e.float_at("range", 0.0), Some(0.1));
    assert_eq!(e.float_at("aperture", 0.0), Some(8.0));
    assert_eq!(e.float_at("near_aperture", 0.0), Some(8.0));
    assert_eq!(e.float_at("far_aperture", 0.0), Some(8.0));
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));
    // Depth invert is off by default (the historical reading).
    assert!(matches!(
        e.param("depth_invert"),
        Some(EffectValue::Bool(false))
    ));
    // Display defaults to Rendered (the normal blurred output).
    assert!(matches!(e.param("display"), Some(EffectValue::Choice(0))));

    // resolve_stack carries only the scalars; the depth is threaded beside
    // the op. The default Aperture master (8) is unity, so each side
    // resolves to its Near/Far radius (8) scaled by the §2.3 preview factor
    // (here 0.5 → 4 raster px). A `dof` always resolves to exactly one
    // Resolved::Dof, so it stays 1:1 and in order with the depth-input list
    // even when the depth reference is unset.
    let r = resolve_stack(&[e], 0.0, 1000.0, 0.5, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Dof {
            focus: 0.5,
            range: 0.1,
            near_aperture: 4.0,
            far_aperture: 4.0,
            depth_invert: false,
            display: 0,
            mix: 1.0,
        }]
    );
}

#[test]
fn dof_near_far_override_and_fall_back_to_the_aperture_master() {
    // Near/Far override the per-side radii; the Aperture master scales both
    // about its default 8. Set Aperture 16 (master 2×), Near 10, Far 4.
    let mut e = instantiate("dof").unwrap();
    for p in e.params.iter_mut() {
        match p.id.as_str() {
            "aperture" => p.value = EffectValue::Float(Property::fixed(16.0)),
            "near_aperture" => p.value = EffectValue::Float(Property::fixed(10.0)),
            "far_aperture" => p.value = EffectValue::Float(Property::fixed(4.0)),
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Dof {
            focus: 0.5,
            range: 0.1,
            near_aperture: 20.0, // 10 · (16/8)
            far_aperture: 8.0,   // 4 · (16/8)
            depth_invert: false,
            display: 0,
            mix: 1.0,
        }]
    );

    // A legacy instance saved before the Near/Far pair existed has only
    // `aperture`; both sides then fall back to it, reproducing the old
    // symmetric single-aperture behaviour exactly.
    let mut legacy = instantiate("dof").unwrap();
    for p in legacy.params.iter_mut() {
        if p.id == "aperture" {
            p.value = EffectValue::Float(Property::fixed(12.0));
        }
    }
    legacy
        .params
        .retain(|p| p.id != "near_aperture" && p.id != "far_aperture");
    let r = resolve_stack(
        std::slice::from_ref(&legacy),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Dof {
            focus: 0.5,
            range: 0.1,
            near_aperture: 12.0, // 8 (default) · (12/8)
            far_aperture: 12.0,
            depth_invert: false,
            display: 0,
            mix: 1.0,
        }]
    );
}

#[test]
fn layer_param_round_trips_through_serde() {
    // A Layer parameter survives a JSON round-trip set and unset, and
    // `layer_ref` reads back the id the caller renders as the depth pass.
    let id = uuid::Uuid::now_v7();
    let mut e = instantiate("dof").unwrap();
    if let Some(p) = e.params.iter_mut().find(|p| p.id == "depth") {
        p.value = EffectValue::Layer(Some(id));
    }
    let json = serde_json::to_string(&e).unwrap();
    let back: EffectInstance = serde_json::from_str(&json).unwrap();
    assert_eq!(back.layer_ref("depth"), Some(id));

    // The unset reference round-trips as such (a passthrough, never lost).
    let unset = EffectValue::Layer(None);
    let j = serde_json::to_string(&unset).unwrap();
    assert_eq!(serde_json::from_str::<EffectValue>(&j).unwrap(), unset);
}

#[test]
fn temporal_window_is_zero_until_a_temporal_effect_joins() {
    // Every current built-in is single-frame (temporal &[0]), so any
    // stack of them needs only the current frame.
    let blur = instantiate("blur").unwrap();
    let glow = instantiate("glow").unwrap();
    assert_eq!(
        stack_temporal_window(&[blur.clone(), glow.clone()], true),
        vec![0]
    );
    assert!(!stack_is_temporal(&[blur.clone(), glow.clone()], true));
    // Bypassed stack, empty stack, and a disabled effect all reduce to
    // the current frame only.
    assert_eq!(stack_temporal_window(&[blur.clone(), glow], false), vec![0]);
    assert_eq!(stack_temporal_window(&[], true), vec![0]);
    let mut off = blur.clone();
    off.enabled = false;
    assert_eq!(stack_temporal_window(&[off], true), vec![0]);
    // The window always contains 0 and is sorted/deduped — pinned so a
    // temporal effect's offsets union cleanly with the current frame.
    assert!(stack_temporal_window(&[blur], true).contains(&0));
}

#[test]
fn motion_blur_window_reaches_the_next_frame_and_wants_flow() {
    // Motion blur's window is {0, 1}: the current frame and one ahead,
    // the pair the flow engine measures motion between.
    let mb = instantiate("motion_blur").unwrap();
    let one = std::slice::from_ref(&mb);
    assert_eq!(stack_temporal_window(one, true), vec![0, 1]);
    assert!(stack_is_temporal(one, true));
    // The flow-field gate is set by motion blur and nothing else current.
    assert_eq!(stack_flow_neighbour(one, true), Some(1));
    let blur = instantiate("blur").unwrap();
    let echo = instantiate("echo").unwrap();
    assert_eq!(stack_flow_neighbour(&[blur.clone(), echo], true), None);
    // Bypassed by the layer fx switch, or disabled, it wants nothing.
    assert_eq!(stack_flow_neighbour(one, false), None);
    let mut off = mb.clone();
    off.enabled = false;
    assert_eq!(stack_flow_neighbour(std::slice::from_ref(&off), true), None);
}

#[test]
fn datamosh_window_reaches_the_prior_frame_and_wants_flow() {
    // Datamosh's window is {-1, 0}: the current frame and one behind,
    // read statically off the schema (K-107 — no per-instance toggle,
    // unlike the old combined Glitch's dynamic special case).
    let dm = instantiate("datamosh").unwrap();
    let one = std::slice::from_ref(&dm);
    assert_eq!(stack_temporal_window(one, true), vec![-1, 0]);
    assert!(stack_is_temporal(one, true));
    assert_eq!(stack_flow_neighbour(one, true), Some(-1));

    // A plain Block glitch stays single-frame.
    let plain = instantiate("block_glitch").unwrap();
    let plain_one = std::slice::from_ref(&plain);
    assert_eq!(stack_temporal_window(plain_one, true), vec![0]);
    assert!(!stack_is_temporal(plain_one, true));
    assert_eq!(stack_flow_neighbour(plain_one, true), None);

    // Disabled, or the layer fx switch off, Datamosh wants nothing.
    let mut off = dm.clone();
    off.enabled = false;
    assert_eq!(
        stack_temporal_window(std::slice::from_ref(&off), true),
        vec![0]
    );
    assert_eq!(stack_flow_neighbour(std::slice::from_ref(&off), true), None);
    assert_eq!(stack_flow_neighbour(one, false), None);
}

#[test]
fn motion_blur_and_datamosh_together_the_first_in_stack_order_wins() {
    // K-104: a layer can carry only one flow field per frame in v1: if
    // both a live Motion blur and a live Datamosh are in the same
    // stack, whichever comes first wins the single slot.
    let mb = instantiate("motion_blur").unwrap();
    let dm = instantiate("datamosh").unwrap();
    assert_eq!(
        stack_flow_neighbour(&[mb.clone(), dm.clone()], true),
        Some(1)
    );
    assert_eq!(stack_flow_neighbour(&[dm, mb], true), Some(-1));
}

#[test]
fn datamosh_instantiates_and_resolves() {
    let e = instantiate("datamosh").unwrap();
    assert_eq!(e.float_at("intensity", 0.0), Some(1.0));
    assert_eq!(e.float_at("displacement", 0.0), Some(4.0));
    assert_eq!(e.float_at("bloom", 0.0), Some(0.6));
    assert_eq!(e.float_at("reset_interval", 0.0), Some(0.0));
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));

    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    // Reset off (interval 0) → full ramp; displacement 4 → 4 taps.
    assert_eq!(
        r,
        vec![Resolved::Datamosh {
            intensity: 1.0,
            displacement: 4.0,
            bloom: 0.6,
            steps: 4,
            mix: 1.0,
        }]
    );

    // Intensity 0 and Mix 0 both resolve cleanly (the bit-exact
    // passthrough is enforced where the op actually runs, in lumit-gpu
    // and lumit-ui — this pins the resolve step carries both zeros
    // through untouched).
    let mut zero_intensity = e.clone();
    for p in &mut zero_intensity.params {
        if p.id == "intensity" {
            p.value = EffectValue::Float(Property::fixed(0.0));
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&zero_intensity),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Datamosh {
            intensity: 0.0,
            displacement: 4.0,
            bloom: 0.6,
            steps: 4,
            mix: 1.0,
        }]
    );
}

#[test]
fn datamosh_intensity_ceiling_is_open_and_displacement_migrates() {
    // FX-14/K-148/K-161: the Intensity hard cap is lifted (K-135), so a typed
    // value above 1 resolves through for a punchier tear; Displacement is
    // clamped at 1 below and open above.
    let mut e = instantiate("datamosh").unwrap();
    for p in &mut e.params {
        if p.id == "intensity" {
            p.value = EffectValue::Float(Property::fixed(2.5));
        }
        if p.id == "displacement" {
            p.value = EffectValue::Float(Property::fixed(9.0));
        }
    }
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Datamosh {
            intensity: 2.5,
            displacement: 9.0,
            bloom: 0.6,
            steps: 9,
            mix: 1.0,
        }]
    );

    // An old project (K-148) carries `streak_length`, not `displacement`: the
    // resolve reads it as the reach fallback, so the loaded look is unchanged.
    let mut legacy = instantiate("datamosh").unwrap();
    for p in &mut legacy.params {
        if p.id == "displacement" {
            p.id = "streak_length".to_string();
            p.value = EffectValue::Float(Property::fixed(7.0));
        }
    }
    let r = resolve_stack(&[legacy], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Datamosh {
            intensity: 1.0,
            displacement: 7.0,
            bloom: 0.6,
            steps: 7,
            mix: 1.0,
        }]
    );
}

/// Resolve one datamosh instance at `lt` and return its `(intensity,
/// displacement)`; a small helper for the reset-ramp test.
fn datamosh_reach(e: &EffectInstance, lt: f64) -> (f32, f32) {
    match &resolve_stack(
        std::slice::from_ref(e),
        lt,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    )[..]
    {
        [Resolved::Datamosh {
            intensity,
            displacement,
            ..
        }] => (*intensity, *displacement),
        other => panic!("expected one Datamosh, got {other:?}"),
    }
}

#[test]
fn datamosh_reset_interval_ramps_the_melt() {
    // K-164: a non-zero Reset interval ramps the melt from a clean frame just
    // after each reset up to full by the next — a pure function of layer time.
    let mut e = instantiate("datamosh").unwrap();
    for p in &mut e.params {
        if p.id == "reset_interval" {
            p.value = EffectValue::Float(Property::fixed(2.0));
        }
    }
    // At each reset boundary (t = 0, 2, 4 s with a 2 s interval) the melt is a
    // clean frame: intensity and displacement both 0.
    assert_eq!(datamosh_reach(&e, 0.0), (0.0, 0.0));
    assert_eq!(datamosh_reach(&e, 2.0), (0.0, 0.0));
    // Half-way through the interval the ramp is 0.5.
    let (mid_i, mid_d) = datamosh_reach(&e, 1.0);
    assert!((mid_i - 0.5).abs() < 1e-6, "intensity 1.0 × 0.5");
    assert!((mid_d - 2.0).abs() < 1e-6, "displacement 4 × 0.5");
    // Just before the next reset the ramp is near full.
    let (late_i, late_d) = datamosh_reach(&e, 1.9);
    assert!(
        late_i > mid_i && late_d > mid_d,
        "the melt grows across the run"
    );
    // Interval 0 (the default) leaves the melt at full strength always.
    let off = instantiate("datamosh").unwrap();
    assert_eq!(
        datamosh_reach(&off, 0.0),
        (1.0, 4.0),
        "reset off → full melt at t=0"
    );
}

#[test]
fn cpu_apply_datamosh_is_a_passthrough() {
    // The single-buffer CPU dispatcher cannot carry a neighbour frame or
    // a flow field, so Resolved::Datamosh degrades to a no-op here,
    // exactly like Echo and Motion blur.
    let (w, h) = (5u32, 5u32);
    let img = transform_card(w, h);
    let mut out = img.clone();
    cpu::apply(
        &mut out,
        w,
        h,
        &Resolved::Datamosh {
            intensity: 1.0,
            displacement: 4.0,
            bloom: 0.6,
            steps: 4,
            mix: 1.0,
        },
    );
    assert_eq!(out, img);
}

#[test]
fn resolve_motion_blur_converts_shutter_and_rounds_samples() {
    let e = instantiate("motion_blur").unwrap();
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    // Defaults: 180° → shutter_frac 0.5, 16 samples, full mix, Rendered view.
    assert_eq!(
        r,
        vec![Resolved::MotionBlur {
            shutter_frac: 0.5,
            samples: 16,
            mix: 1.0,
            view: MbView::Rendered,
        }]
    );
    // A custom stack: 90° halves the streak; Samples rounds and clamps.
    let mut e = instantiate("motion_blur").unwrap();
    for p in e.params.iter_mut() {
        match p.id.as_str() {
            "shutter_angle" => p.value = EffectValue::Float(Property::fixed(90.0)),
            "samples" => p.value = EffectValue::Float(Property::fixed(8.4)),
            "mix" => p.value = EffectValue::Float(Property::fixed(50.0)),
            _ => {}
        }
    }
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::MotionBlur {
            shutter_frac: 0.25,
            samples: 8,
            mix: 0.5,
            view: MbView::Rendered,
        }]
    );
    // The View row resolves the diagnostic choices (FX-19).
    let mut e = instantiate("motion_blur").unwrap();
    for p in e.params.iter_mut() {
        if p.id == "view" {
            p.value = EffectValue::Choice(2);
        }
    }
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert!(matches!(
        r.as_slice(),
        [Resolved::MotionBlur {
            view: MbView::Confidence,
            ..
        }]
    ));
}

#[test]
fn cpu_motion_blur_still_and_zero_shutter_are_passthrough() {
    // A 9x9 with one bright premultiplied pixel in the middle.
    let (w, h) = (9u32, 9u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let mid = ((4 * w + 4) * 4) as usize;
    img[mid..mid + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]);
    let n = (w * h) as usize;

    let full = vec![1.0f32; n]; // full confidence: streak unscaled

    // Zero flow everywhere: every tap lands on the pixel itself, so with
    // Mix 1 the output is the bit-exact input whatever the shutter.
    let (zu, zv) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut still = img.clone();
    cpu::motion_blur(
        &mut still,
        w,
        h,
        &zu,
        &zv,
        &full,
        0.5,
        16,
        1.0,
        MbView::Rendered,
    );
    assert_eq!(still, img, "still pixels do not blur");

    // A real motion but a closed shutter (frac 0) is also identity.
    let (mu, mv) = (vec![3.0f32; n], vec![0.0f32; n]);
    let mut shut = img.clone();
    cpu::motion_blur(
        &mut shut,
        w,
        h,
        &mu,
        &mv,
        &full,
        0.0,
        16,
        1.0,
        MbView::Rendered,
    );
    assert_eq!(shut, img, "a closed shutter does not blur");

    // Mix 0 returns the input exactly, whatever the motion.
    let mut mixed = img.clone();
    cpu::motion_blur(
        &mut mixed,
        w,
        h,
        &mu,
        &mv,
        &full,
        0.5,
        16,
        0.0,
        MbView::Rendered,
    );
    assert_eq!(mixed, img, "mix 0 is a passthrough");

    // Zero confidence collapses the streak to nothing (FX-19), so even a real
    // motion and open shutter leave the input bit-exact.
    let zero = vec![0.0f32; n];
    let mut suspect = img.clone();
    cpu::motion_blur(
        &mut suspect,
        w,
        h,
        &mu,
        &mv,
        &zero,
        0.5,
        16,
        1.0,
        MbView::Rendered,
    );
    assert_eq!(suspect, img, "zero confidence does not blur");
}

#[test]
fn cpu_motion_blur_smears_along_the_flow() {
    // A vertical edge (left half bright, right half dark) smeared by a
    // constant horizontal flow should soften the edge along x while
    // leaving a pixel deep inside a flat region unchanged (a box streak
    // over constant colour is that colour) — the defining behaviour.
    let (w, h) = (16u32, 4u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let v = if x < w / 2 { 1.0 } else { 0.0 };
            img[i..i + 4].copy_from_slice(&[v, v, v, 1.0]);
        }
    }
    let n = (w * h) as usize;
    let (u, vv) = (vec![8.0f32; n], vec![0.0f32; n]); // 8px horizontal
    let full = vec![1.0f32; n];
    let mut out = img.clone();
    cpu::motion_blur(
        &mut out,
        w,
        h,
        &u,
        &vv,
        &full,
        0.5,
        16,
        1.0,
        MbView::Rendered,
    ); // streak 4px

    // Indices on row 0 (a closure keeps clippy's erasing-op lint happy and
    // reads clearly as column, row).
    let idx = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    // A pixel far inside the bright flat region is untouched (1.0).
    let flat = idx(2, 0);
    assert!((out[flat] - 1.0).abs() < 1e-4, "flat interior is preserved");
    // A pixel far inside the dark flat region stays dark.
    let dark = idx(13, 0);
    assert!(out[dark].abs() < 1e-4, "dark interior stays dark");
    // The pixel just right of the edge picks up light from the bright
    // side it was smeared across — a genuine, directional softening.
    let edge = idx(8, 0);
    assert!(
        out[edge] > 0.05 && out[edge] < 0.95,
        "the edge softens along the flow: {}",
        out[edge]
    );
}

// The View diagnostics (FX-19): Motion vectors colour-code the raw flow (mid-
// grey where still, redder for +x, greener for +y) and Confidence shows the 0..1
// field as opaque greyscale — both ignore the source and Mix.
#[test]
fn cpu_motion_blur_view_diagnostics() {
    let (w, h) = (2u32, 1u32);
    let n = (w * h) as usize;
    // Pixel 0 still; pixel 1 moving +16 px in x.
    let u = vec![0.0f32, 16.0];
    let v = vec![0.0f32, 0.0];
    let conf = vec![1.0f32, 0.25];
    let src = vec![0.7f32; n * 4]; // arbitrary source — diagnostics ignore it

    // Motion vectors: still pixel is mid-grey (0.5, 0.5, 0.5, 1); the +16 px
    // pixel saturates red at 0.5 + 16/32 = 1.0.
    let mut mv = src.clone();
    cpu::motion_blur(
        &mut mv,
        w,
        h,
        &u,
        &v,
        &conf,
        0.5,
        16,
        1.0,
        MbView::MotionVectors,
    );
    assert_eq!(&mv[0..4], &[0.5, 0.5, 0.5, 1.0]);
    assert_eq!(&mv[4..8], &[1.0, 0.5, 0.5, 1.0]);

    // Confidence: opaque greyscale of the 0..1 field.
    let mut cf = src.clone();
    cpu::motion_blur(
        &mut cf,
        w,
        h,
        &u,
        &v,
        &conf,
        0.5,
        16,
        1.0,
        MbView::Confidence,
    );
    assert_eq!(&cf[0..4], &[1.0, 1.0, 1.0, 1.0]);
    assert_eq!(&cf[4..8], &[0.25, 0.25, 0.25, 1.0]);
}

#[test]
fn cpu_datamosh_zero_intensity_is_the_bit_exact_current_frame() {
    let (w, h) = (6u32, 4u32);
    let n = (w * h) as usize;
    let current: Vec<f32> = (0..n * 4).map(|i| (i % 7) as f32 * 0.1).collect();
    let prev: Vec<f32> = (0..n * 4).map(|i| (i % 5) as f32 * 0.2).collect();
    let (u, v) = (vec![3.0f32; n], vec![-2.0f32; n]);
    // The melt has no effect at intensity 0 — the blend collapses to `current`.
    let out = cpu::datamosh(&current, &prev, w, h, &u, &v, 0.0, 8.0, 0.7, 8);
    assert_eq!(out, current, "intensity 0 is a bit-exact passthrough");
}

#[test]
fn cpu_datamosh_full_intensity_reads_the_shifted_previous_frame() {
    // A single bright premultiplied pixel in `prev`; a one-step walk whose
    // flow points straight at it should recover that pixel's colour at the
    // sampling position, not `current`'s.
    let (w, h) = (9u32, 9u32);
    let n = (w * h) as usize;
    let current = vec![0.0f32; n * 4]; // all black
    let mut prev = vec![0.0f32; n * 4];
    let bright = ((4 * w + 6) * 4) as usize; // (x=6, y=4)
    prev[bright..bright + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]);
    // Output pixel (4, 4) walks one step of flow u = 2 (× displacement 1) to
    // (6, 4).
    let mut u = vec![0.0f32; n];
    let v = vec![0.0f32; n];
    u[(4 * w + 4) as usize] = 2.0;
    let out = cpu::datamosh(&current, &prev, w, h, &u, &v, 1.0, 1.0, 0.6, 1);
    let i = ((4 * w + 4) * 4) as usize;
    assert_eq!(&out[i..i + 4], &[4.0, 2.0, 1.0, 1.0]);
    // A pixel whose flow is zero and whose `prev` neighbourhood is dark
    // stays dark (current is also dark there) — no bleed from elsewhere.
    assert_eq!(&out[0..4], &[0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn cpu_datamosh_displacement_scales_the_flow_reach() {
    // A single bright pixel at (6, 4). A flow of u = 1 reaches it only when
    // Displacement doubles the one-step reach to 2 (K-161): the walk then
    // predicts two frames of motion from one frame's flow.
    let (w, h) = (9u32, 9u32);
    let n = (w * h) as usize;
    let current = vec![0.0f32; n * 4];
    let mut prev = vec![0.0f32; n * 4];
    let bright = ((4 * w + 6) * 4) as usize; // (x=6, y=4)
    prev[bright..bright + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]);
    let mut u = vec![0.0f32; n];
    let v = vec![0.0f32; n];
    u[(4 * w + 4) as usize] = 1.0; // one frame of flow points halfway there
    let i = ((4 * w + 4) * 4) as usize;

    // Displacement 1, one step: u = 1 lands on (5, 4) — the bright pixel unreached.
    let short = cpu::datamosh(&current, &prev, w, h, &u, &v, 1.0, 1.0, 0.6, 1);
    assert_eq!(&short[i..i + 4], &[0.0, 0.0, 0.0, 0.0]);
    // Displacement 2, one step: u × 2 = 2 lands on (6, 4) — now recovered.
    let long = cpu::datamosh(&current, &prev, w, h, &u, &v, 1.0, 2.0, 0.6, 1);
    assert_eq!(&long[i..i + 4], &[4.0, 2.0, 1.0, 1.0]);
}

#[test]
fn cpu_datamosh_bloom_accumulates_the_far_trail() {
    // A constant rightward flow walks the streamline across four columns; a
    // bright pixel sits at the far end. Bloom 0 keeps only the nearest step
    // (missing it); Bloom 1 averages the whole walk (pulling it in). The dial
    // is monotone between (K-161).
    let (w, h) = (12u32, 9u32);
    let n = (w * h) as usize;
    let current = vec![0.0f32; n * 4]; // black
    let mut prev = vec![0.0f32; n * 4];
    let far = ((4 * w + 8) * 4) as usize; // (x=8, y=4), the far end of the walk
    prev[far..far + 4].copy_from_slice(&[4.0, 0.0, 0.0, 1.0]);
    let (u, v) = (vec![1.0f32; n], vec![0.0f32; n]); // one column per step
    let i = ((4 * w + 4) * 4) as usize; // output pixel (4, 4)
                                        // Four steps from (4,4) sample prev at columns 5, 6, 7, 8 — the bright
                                        // pixel is the last (fourth) tap.
    let r = |bloom: f32| cpu::datamosh(&current, &prev, w, h, &u, &v, 1.0, 4.0, bloom, 4)[i];
    assert_eq!(r(0.0), 0.0, "bloom 0 keeps only the near step (dark)");
    // Bloom 1 averages 4 taps: (0 + 0 + 0 + 4) / 4 = 1.0 in red.
    assert!(
        (r(1.0) - 1.0).abs() < 1e-5,
        "bloom 1 pulls in the far trail"
    );
    let mid = r(0.5);
    assert!(mid > 0.0 && mid < 1.0, "bloom is monotone between 0 and 1");
}

#[test]
fn echo_defaults_to_screen_caps_at_16_and_migrates_legacy_modes() {
    // FX-17/K-149: the default blend mode is Screen (index 3), Echoes clamps
    // to the raised 16-frame window, and the legacy mode indices 0/1/2 still
    // resolve to Add/Behind/Max so old projects load unchanged.
    let e = instantiate("echo").unwrap();
    assert!(matches!(e.param("mode"), Some(EffectValue::Choice(3))));

    // Echoes 20 clamps to 16 non-zero geometric weights (decay^k).
    let mut over = e.clone();
    for p in &mut over.params {
        if p.id == "echoes" {
            p.value = EffectValue::Float(Property::fixed(20.0));
        }
        if p.id == "decay" {
            p.value = EffectValue::Float(Property::fixed(0.5));
        }
    }
    let r = resolve_stack(&[over], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    let Resolved::Echo { weights, mode, .. } = r[0] else {
        panic!("expected an echo op");
    };
    assert_eq!(mode, 3, "default mode is Screen");
    assert!(weights.iter().all(|w| *w > 0.0), "all 16 taps are live");
    assert!((weights[0] - 0.5).abs() < 1e-6 && (weights[15] - 0.5f32.powi(16)).abs() < 1e-9);

    // Every mode index (0 Behind … 13 Divide, T21) resolves through unchanged,
    // and an out-of-range index clamps to the top of the list rather than
    // panicking.
    for m in [0u32, 1, 2, 8, 12, 13] {
        let mut old = e.clone();
        for p in &mut old.params {
            if p.id == "mode" {
                p.value = EffectValue::Choice(m);
            }
        }
        let r = resolve_stack(&[old], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        let Resolved::Echo { mode, .. } = r[0] else {
            panic!("expected an echo op");
        };
        assert_eq!(mode, m, "mode index preserved");
    }
    let mut oob = e.clone();
    for p in &mut oob.params {
        if p.id == "mode" {
            p.value = EffectValue::Choice(99);
        }
    }
    let r = resolve_stack(&[oob], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    let Resolved::Echo { mode, .. } = r[0] else {
        panic!("expected an echo op");
    };
    assert_eq!(mode, 13, "out-of-range mode clamps to the last (Divide)");
}

#[test]
fn cpu_echo_blend_modes_combine_a_single_tap() {
    // One opaque grey pixel echoed by one darker opaque neighbour, weight 1,
    // Mix 1 (so the output is the pure combine). Values chosen to be exact in
    // f32: 0.5 and 0.25. The mode indices are the T21 order (0 Behind …
    // 13 Divide); each mode applies to all four premultiplied channels.
    let current = [0.5f32, 0.5, 0.5, 1.0];
    let neighbour = [0.25f32, 0.25, 0.25, 1.0];
    let mut weights = [0.0f32; 16];
    weights[0] = 1.0;
    let run = |mode: u32| cpu::echo(&current, &[(-1, &neighbour)], weights, mode, 1.0);

    // Behind (0): accumulator over the echo — opaque accumulator wins.
    assert_eq!(run(0), vec![0.5, 0.5, 0.5, 1.0]);
    // In front (1): echo over the accumulator — opaque echo wins.
    assert_eq!(run(1), vec![0.25, 0.25, 0.25, 1.0]);
    // Add (2): 0.5 + 0.25 = 0.75; alpha 1 + 1 = 2.
    assert_eq!(run(2), vec![0.75, 0.75, 0.75, 2.0]);
    // Screen (3): 0.5 + 0.25 − 0.5×0.25 = 0.625; alpha 1 + 1 − 1 = 1.
    assert_eq!(run(3), vec![0.625, 0.625, 0.625, 1.0]);
    // Multiply (4): 0.5 × 0.25 = 0.125.
    assert_eq!(run(4), vec![0.125, 0.125, 0.125, 1.0]);
    // Overlay (5): accumulator 0.5 ≤ 0.5 → 2·0.5·0.25 = 0.25; alpha 1.
    assert_eq!(run(5), vec![0.25, 0.25, 0.25, 1.0]);
    // Hard light (7): echo 0.25 ≤ 0.5 → 2·0.5·0.25 = 0.25; alpha 1.
    assert_eq!(run(7), vec![0.25, 0.25, 0.25, 1.0]);
    // Lighten (8): max(0.5, 0.25) = 0.5 — the leading frame wins.
    assert_eq!(run(8), vec![0.5, 0.5, 0.5, 1.0]);
    // Darken (9): min(0.5, 0.25) = 0.25.
    assert_eq!(run(9), vec![0.25, 0.25, 0.25, 1.0]);
    // Difference (10): |0.5 − 0.25| = 0.25; alpha |1 − 1| = 0.
    assert_eq!(run(10), vec![0.25, 0.25, 0.25, 0.0]);
    // Exclusion (11): 0.5 + 0.25 − 2·0.5·0.25 = 0.5; alpha 1 + 1 − 2 = 0.
    assert_eq!(run(11), vec![0.5, 0.5, 0.5, 0.0]);
    // Subtract (12): max(0.5 − 0.25, 0) = 0.25; alpha max(1 − 1, 0) = 0.
    assert_eq!(run(12), vec![0.25, 0.25, 0.25, 0.0]);
    // Divide (13): 0.5 ÷ 0.25 = 2.0; alpha 1 ÷ 1 = 1.
    assert_eq!(run(13), vec![2.0, 2.0, 2.0, 1.0]);
}

#[test]
fn cpu_blur_identity_energy_and_mix() {
    // A 9x9 with one bright premultiplied pixel in the middle.
    let (w, h) = (9u32, 9u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let mid = ((4 * w + 4) * 4) as usize;
    img[mid..mid + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]); // HDR > 1

    // Radius 0 is the identity.
    let mut id = img.clone();
    cpu::blur_gaussian(&mut id, w, h, 0.0, 1, 1.0);
    assert_eq!(id, img);

    // A blur spreads but conserves energy away from edges (repeat policy,
    // small radius, bright pixel far from borders).
    let mut blurred = img.clone();
    cpu::blur_gaussian(&mut blurred, w, h, 2.0, 1, 1.0);
    assert!(blurred[mid] < img[mid], "peak flattens");
    let sum = |v: &[f32]| v.iter().step_by(4).sum::<f32>(); // red plane
    assert!((sum(&blurred) - sum(&img)).abs() < 1e-3, "energy conserved");

    // Mix 0 returns the input exactly, whatever the radius.
    let mut mixed = img.clone();
    cpu::blur_gaussian(&mut mixed, w, h, 5.0, 1, 0.0);
    assert_eq!(mixed, img);

    // Transparent edges lose energy when the kernel hangs off the border.
    let mut corner = vec![0.0f32; (w * h * 4) as usize];
    corner[0..4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
    let mut t = corner.clone();
    cpu::blur_gaussian(&mut t, w, h, 3.0, 0, 1.0);
    let mut rep = corner;
    cpu::blur_gaussian(&mut rep, w, h, 3.0, 1, 1.0);
    assert!(sum(&t) < sum(&rep), "transparent edge sheds energy");
}

#[test]
fn sharpen_instantiates_and_resolves() {
    let e = instantiate("sharpen").unwrap();
    assert_eq!(e.float_at("amount", 0.0), Some(60.0));
    assert_eq!(e.float_at("radius", 0.0), Some(0.4));
    assert_eq!(e.float_at("threshold", 0.0), Some(0.05));
    assert!(matches!(
        e.param("luminance_only"),
        Some(EffectValue::Bool(true))
    ));
    // 0.4% of a 1000px diagonal = 4px; amount 60% = 0.6.
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Sharpen {
            amount: 0.6,
            radius_px: 4.0,
            threshold: 0.05,
            luma_only: true,
            mix: 1.0
        }]
    );
}

/// A step edge for sharpen tests: left half dark, right half bright,
/// fully opaque, with an HDR right side.
fn step_image(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let v = if x < w / 2 { 0.2 } else { 2.0 };
            img[i..i + 4].copy_from_slice(&[v, v * 0.5, v * 0.25, 1.0]);
        }
    }
    img
}

#[test]
fn cpu_sharpen_identity_edge_overshoot_and_threshold() {
    let (w, h) = (16u32, 8u32);
    let img = step_image(w, h);

    // Mix 0 is the exact identity.
    let mut m0 = img.clone();
    cpu::sharpen(&mut m0, w, h, 1.0, 3.0, 0.0, true, 0.0);
    assert_eq!(m0, img);

    // Amount 0 changes nothing (opaque pixels, so unpremultiply is exact).
    let mut a0 = img.clone();
    cpu::sharpen(&mut a0, w, h, 0.0, 3.0, 0.0, true, 1.0);
    for (a, b) in a0.iter().zip(&img) {
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    // A flat region is untouched; the step edge overshoots both ways.
    let mut s = img.clone();
    cpu::sharpen(&mut s, w, h, 1.0, 2.0, 0.0, true, 1.0);
    let px = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let far = px(1, 4);
    assert!((s[far] - img[far]).abs() < 1e-4, "flat area stays put");
    let dark_side = px(w / 2 - 1, 4);
    let bright_side = px(w / 2, 4);
    assert!(s[dark_side] < img[dark_side], "dark side of edge dips");
    assert!(s[bright_side] > img[bright_side], "bright side lifts");

    // A threshold above the edge contrast suppresses the sharpening.
    let mut t = img.clone();
    cpu::sharpen(&mut t, w, h, 1.0, 2.0, 1.0, true, 1.0);
    for (a, b) in t.iter().zip(&img) {
        assert!((a - b).abs() < 1e-5, "threshold 1.0 gates the edge detail");
    }

    // Fully transparent input stays fully transparent (no invented light).
    let mut clear = vec![0.0f32; (w * h * 4) as usize];
    cpu::sharpen(&mut clear, w, h, 3.0, 2.0, 0.0, false, 1.0);
    assert!(clear.iter().all(|v| *v == 0.0));

    // Per-channel mode fringes where luma-only does not: on a pure
    // chroma edge (constant luma), luma-only is inert.
    let mut chroma = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            // Two colours with identical Rec. 709 luma.
            let (r, g, b) = if x < w / 2 {
                (0.5, 0.25, 0.0)
            } else {
                let r = 0.1f32;
                let b = 0.4f32;
                let g = (0.5 * cpu::LUMA[0] + 0.25 * cpu::LUMA[1] - r * cpu::LUMA[0]
                    + 0.0 * cpu::LUMA[2]
                    - b * cpu::LUMA[2])
                    / cpu::LUMA[1];
                (r, g, b)
            };
            chroma[i..i + 4].copy_from_slice(&[r, g, b, 1.0]);
        }
    }
    let mut luma_pass = chroma.clone();
    cpu::sharpen(&mut luma_pass, w, h, 2.0, 2.0, 0.0, true, 1.0);
    let mut chan_pass = chroma.clone();
    cpu::sharpen(&mut chan_pass, w, h, 2.0, 2.0, 0.0, false, 1.0);
    let dev = |out: &[f32]| {
        out.iter()
            .zip(&chroma)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max)
    };
    assert!(dev(&luma_pass) < 1e-4, "luma-only ignores chroma edges");
    assert!(dev(&chan_pass) > 0.05, "per-channel mode sharpens them");
}

#[test]
fn rgb_split_instantiates_and_resolves() {
    let e = instantiate("rgb_split").unwrap();
    assert_eq!(e.float_at("amount", 0.0), Some(0.4));
    assert_eq!(e.float_at("angle", 0.0), Some(0.0));
    // Radial is gone (T17): RGB split is linear-only, chromatic aberration
    // owns the radial shape.
    assert!(e.param("radial").is_none());
    // The per-tap scale defaults reproduce the classic split (FX-9).
    assert_eq!(e.float_at("red_amount", 0.0), Some(100.0));
    assert_eq!(e.float_at("green_amount", 0.0), Some(0.0));
    assert_eq!(e.float_at("blue_amount", 0.0), Some(100.0));
    // The three tap tints default to red / green / blue (T17).
    assert_eq!(
        e.colour_at("channel_colour_1", 0.0),
        Some([1.0, 0.0, 0.0, 1.0])
    );
    assert_eq!(
        e.colour_at("channel_colour_2", 0.0),
        Some([0.0, 1.0, 0.0, 1.0])
    );
    assert_eq!(
        e.colour_at("channel_colour_3", 0.0),
        Some([0.0, 0.0, 1.0, 1.0])
    );
    // 0.4% of a 1000px diagonal = 4px.
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::RgbSplit {
            amount_px: 4.0,
            angle_deg: 0.0,
            scale: [1.0, 0.0, 1.0],
            tints: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mix: 1.0
        }]
    );
}

#[test]
fn rgb_split_per_channel_amounts_scale_each_channel() {
    // Per-channel amounts (FX-9): each per-cent scale resolves to a factor,
    // and a legacy instance (no per-channel params) falls back to 1 / 0 / 1.
    let mut e = instantiate("rgb_split").unwrap();
    for p in &mut e.params {
        match p.id.as_str() {
            "red_amount" => p.value = EffectValue::Float(Property::fixed(150.0)),
            "green_amount" => p.value = EffectValue::Float(Property::fixed(-50.0)),
            "blue_amount" => p.value = EffectValue::Float(Property::fixed(0.0)),
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::RgbSplit {
            amount_px: 4.0,
            angle_deg: 0.0,
            scale: [1.5, -0.5, 0.0],
            tints: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mix: 1.0
        }]
    );

    // A legacy instance missing the per-tap params still resolves to the
    // classic 1 / 0 / 1 scales and red / green / blue tints.
    e.params
        .retain(|p| !matches!(p.id.as_str(), "red_amount" | "green_amount" | "blue_amount"));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::RgbSplit {
            amount_px: 4.0,
            angle_deg: 0.0,
            scale: [1.0, 0.0, 1.0],
            tints: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mix: 1.0
        }]
    );
}

#[test]
fn cpu_rgb_split_shifts_channels_and_keeps_alpha() {
    // A white impulse in the middle of a black opaque frame.
    let (w, h) = (17u32, 9u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px[3] = 1.0;
    }
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let mid = at(8, 4);
    img[mid..mid + 3].copy_from_slice(&[1.0, 1.0, 1.0]);

    // The classic split's per-tap scales (FX-9): taps 0/2 full, tap 1 anchored.
    let classic = [1.0f32, 0.0, 1.0];
    // The classic red / green / blue tints (T17): each primary keeps only its
    // own channel of its tap, reproducing the channel-separated split.
    let classic_tints = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    // Amount 0 and mix 0 are both the exact identity.
    let mut a0 = img.clone();
    cpu::rgb_split(&mut a0, w, h, 0.0, 0.0, classic, classic_tints, 1.0);
    assert_eq!(a0, img);
    let mut m0 = img.clone();
    cpu::rgb_split(&mut m0, w, h, 3.0, 45.0, classic, classic_tints, 0.0);
    assert_eq!(m0, img);

    // Angle 0°, 2px: red lands 2px right of the impulse, blue 2px left,
    // green and alpha exactly where they were.
    let mut s = img.clone();
    cpu::rgb_split(&mut s, w, h, 2.0, 0.0, classic, classic_tints, 1.0);
    assert_eq!(s[at(10, 4)], 1.0, "red shifted +x");
    assert_eq!(s[at(8, 4)], 0.0, "red left the impulse");
    assert_eq!(s[at(6, 4) + 2], 1.0, "blue shifted -x");
    assert_eq!(s[at(8, 4) + 1], 1.0, "green stays");
    assert!(
        s.iter().skip(3).step_by(4).all(|a| *a == 1.0),
        "alpha follows green: untouched"
    );

    // Per-tap scales (FX-9): halving tap 0's scale halves its displacement,
    // so red now lands 1px (not 2px) right of the impulse; zeroing tap 2's
    // scale keeps blue on the impulse.
    let mut pc = img.clone();
    cpu::rgb_split(&mut pc, w, h, 2.0, 0.0, [0.5, 0.0, 0.0], classic_tints, 1.0);
    assert_eq!(pc[at(9, 4)], 1.0, "red at half scale shifts +1x");
    assert_eq!(pc[at(10, 4)], 0.0, "red no longer reaches +2x");
    assert_eq!(
        pc[at(8, 4) + 2],
        1.0,
        "blue at scale 0 stays on the impulse"
    );

    // Tints (T17): a white tint on tap 0 keeps the full colour of its sample,
    // so the shifted tap 0 now carries green and blue too — not just red.
    let white_tap0 = [[1.0f32, 1.0, 1.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
    let mut ti = img.clone();
    cpu::rgb_split(&mut ti, w, h, 2.0, 0.0, classic, white_tap0, 1.0);
    assert_eq!(ti[at(10, 4)], 1.0, "tap 0 red at +2x");
    assert_eq!(ti[at(10, 4) + 1], 1.0, "tap 0 green at +2x (white tint)");
    assert_eq!(ti[at(10, 4) + 2], 1.0, "tap 0 blue at +2x (white tint)");
    assert_eq!(ti[at(8, 4)], 0.0, "nothing left on the impulse");
}

#[test]
fn rgb_split_wavelength_bool_selects_the_variant() {
    // A fresh instance defaults to the classic split — and resolves to
    // the exact same Resolved value it did before the Bool existed.
    let mut e = instantiate("rgb_split").unwrap();
    assert!(matches!(
        e.param("wavelength"),
        Some(EffectValue::Bool(false))
    ));
    let classic = Resolved::RgbSplit {
        amount_px: 4.0,
        angle_deg: 0.0,
        scale: [1.0, 0.0, 1.0],
        tints: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        mix: 1.0,
    };
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(r, vec![classic]);

    // Wavelength on: the same numbers arrive as SpectralSplit, carrying the
    // default Samples (16).
    for p in &mut e.params {
        if p.id == "wavelength" {
            p.value = EffectValue::Bool(true);
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::SpectralSplit {
            amount_px: 4.0,
            angle_deg: 0.0,
            radial: false,
            samples: 16,
            tints: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mix: 1.0
        }]
    );

    // A legacy instance (saved before the Bool existed) has no
    // wavelength parameter and still resolves as the classic split.
    e.params.retain(|p| p.id != "wavelength");
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(r, vec![classic]);
}

/// The default channel tints — red / green / blue — that reproduce the
/// classic R-outward / B-inward / G-anchor split (P2/K-143).
const RGB_TINTS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[test]
fn chromatic_aberration_instantiates_and_resolves() {
    let e = instantiate("chromatic_aberration").unwrap();
    assert_eq!(e.float_at("amount", 0.0), Some(4.0));
    // The three channel colours default to red / green / blue (P2/K-143).
    assert_eq!(
        e.colour_at("channel_colour_1", 0.0),
        Some([1.0, 0.0, 0.0, 1.0])
    );
    assert_eq!(
        e.colour_at("channel_colour_2", 0.0),
        Some([0.0, 1.0, 0.0, 1.0])
    );
    assert_eq!(
        e.colour_at("channel_colour_3", 0.0),
        Some([0.0, 0.0, 1.0, 1.0])
    );
    assert!(matches!(
        e.param("wavelength"),
        Some(EffectValue::Bool(false))
    ));
    // px@comp, not % diag: diag_px does not enter the conversion, unlike
    // rgb_split's own Amount — only the preview-resolution px_scale does.
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::ChromaticAberration {
            amount_px: 4.0,
            tints: RGB_TINTS,
            mix: 1.0
        }]
    );
}

#[test]
fn chromatic_aberration_amount_scales_with_the_preview_factor() {
    let e = instantiate("chromatic_aberration").unwrap();
    // Half preview (px_scale 0.5): px@comp parameters scale down with
    // it, exactly like Glitch's Block size (§2.3).
    let r = resolve_stack(&[e], 0.0, 1000.0, 0.5, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::ChromaticAberration {
            amount_px: 2.0,
            tints: RGB_TINTS,
            mix: 1.0
        }]
    );
}

#[test]
fn chromatic_aberration_wavelength_reuses_the_spectral_split() {
    // Wavelength on (K-144): the effect reuses RGB split's spectral machinery
    // as a radial spectral split, carrying the Samples count.
    let mut e = instantiate("chromatic_aberration").unwrap();
    for p in &mut e.params {
        match p.id.as_str() {
            "wavelength" => p.value = EffectValue::Bool(true),
            "samples" => p.value = EffectValue::Float(Property::fixed(32.0)),
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::SpectralSplit {
            amount_px: 4.0,
            angle_deg: 0.0,
            radial: true,
            samples: 32,
            tints: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mix: 1.0
        }]
    );
}

/// K-167: with the classic tints normalised per channel, a UNIFORM image passes
/// through the classic split unchanged whatever colours the picker holds — the
/// picker tints only the misaligned fringes, never the whole picture (the
/// owner's "only affect the parts that aren't aligned").
#[test]
fn normalised_tints_leave_a_uniform_image_unchanged() {
    let raw = [[0.9f32, 0.4, 0.0], [0.2, 0.5, 0.3], [0.1, 0.8, 0.6]];
    let tints = normalise_tint_columns(raw);
    for c in 0..3usize {
        let sum: f32 = tints.iter().map(|t| t[c]).sum();
        assert!((sum - 1.0).abs() < 1e-6, "channel {c} sums to {sum}");
    }
    // A uniform frame through the classic split with those tints: unchanged
    // within float rounding (every tap samples the same colour).
    let (w, h) = (8u32, 6u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px.copy_from_slice(&[0.7, 0.3, 0.55, 1.0]);
    }
    let before = img.clone();
    cpu::rgb_split(&mut img, w, h, 2.0, 30.0, [1.0, 0.0, 1.0], tints, 1.0);
    for (a, b) in img.iter().zip(&before) {
        assert!((a - b).abs() < 1e-5, "{a} vs {b}");
    }
}

#[test]
fn wavelength_mode_honours_the_channel_picker() {
    // A1/K-163: the three-colour picker now drives the Wavelength dispersion,
    // so a custom set of colours arrives in the resolved SpectralSplit.
    let mut e = instantiate("rgb_split").unwrap();
    for p in &mut e.params {
        match p.id.as_str() {
            "wavelength" => p.value = EffectValue::Bool(true),
            "channel_colour_1" => {
                p.value = EffectValue::Colour([
                    Property::fixed(1.0),
                    Property::fixed(1.0),
                    Property::fixed(0.0),
                    Property::fixed(1.0),
                ])
            }
            "channel_colour_3" => {
                p.value = EffectValue::Colour([
                    Property::fixed(0.0),
                    Property::fixed(1.0),
                    Property::fixed(1.0),
                    Property::fixed(1.0),
                ])
            }
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::SpectralSplit { tints, .. } = r[0] else {
        panic!("expected a spectral split");
    };
    assert_eq!(tints[0], [1.0, 1.0, 0.0], "colour 1 → yellow end");
    assert_eq!(tints[2], [0.0, 1.0, 1.0], "colour 3 → cyan end");
}

#[test]
fn chromatic_aberration_custom_channel_colours_resolve_as_tints() {
    // The three-colour picker (P2/K-143): custom channel colours arrive as the
    // radial taps' tints. A legacy instance (no colour params) falls back to
    // red / green / blue.
    let mut e = instantiate("chromatic_aberration").unwrap();
    for p in &mut e.params {
        if p.id == "channel_colour_2" {
            p.value = EffectValue::Colour([
                Property::fixed(0.5),
                Property::fixed(0.25),
                Property::fixed(0.75),
                Property::fixed(1.0),
            ]);
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::ChromaticAberration {
            amount_px: 4.0,
            // Normalised per channel (K-167): r column 1 + 0.5 → 2/3, 1/3;
            // g column 0.25 alone → 1; b column 0.75 + 1 → 3/7, 4/7.
            tints: [
                [1.0 / 1.5, 0.0, 0.0],
                [0.5 / 1.5, 1.0, 0.75 / 1.75],
                [0.0, 0.0, 1.0 / 1.75]
            ],
            mix: 1.0
        }]
    );

    e.params.retain(|p| !p.id.starts_with("channel_colour_"));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::ChromaticAberration {
            amount_px: 4.0,
            tints: RGB_TINTS,
            mix: 1.0
        }]
    );
}

#[test]
fn cpu_chromatic_aberration_shifts_channels_radially_and_keeps_alpha() {
    // A white impulse in the middle of a black opaque frame — the same
    // corpus rgb_split's own radial-mode test uses.
    let (w, h) = (17u32, 9u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px[3] = 1.0;
    }
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let mid = at(8, 4);
    img[mid..mid + 3].copy_from_slice(&[1.0, 1.0, 1.0]);

    // Amount 0 and mix 0 are both the exact identity (the general
    // formula's own passthrough, mirroring rgb_split's un-guarded style).
    let mut a0 = img.clone();
    cpu::chromatic_aberration(&mut a0, w, h, 0.0, RGB_TINTS, 1.0);
    assert_eq!(a0, img);
    let mut m0 = img.clone();
    cpu::chromatic_aberration(&mut m0, w, h, 5.0, RGB_TINTS, 0.0);
    assert_eq!(m0, img);

    // The exact centre pixel is unmoved even at a huge amount: its own
    // (position − centre) vector is zero, so every tap collapses onto it.
    let mut c = img.clone();
    cpu::chromatic_aberration(&mut c, w, h, 20.0, RGB_TINTS, 1.0);
    assert_eq!(c[mid], 1.0, "frame-centre red is unmoved");
    assert_eq!(c[mid + 2], 1.0, "frame-centre blue is unmoved");
    assert_eq!(c[mid + 1], 1.0, "green untouched everywhere");

    // At Amount = half the frame diagonal, k is exactly 1: every
    // pixel's R sample point algebraically collapses onto the frame
    // centre (`pos − (pos − centre)·1 = centre`) — and because every
    // coordinate here is an integer or half-integer well inside f32's
    // exact range, that cancellation is bit-exact, not approximate. So
    // red reads the centre's own red value (the impulse, 1.0)
    // everywhere: a clean, exact witness that the offset visibly moves
    // colour off-centre, which a single arbitrary amount cannot give
    // (a lone one-texel impulse can fall clean outside a shifted tap's
    // bilinear footprint, missing it entirely).
    let (fw, fh) = (w as f32, h as f32);
    let diag = (fw * fw + fh * fh).sqrt();
    let mut half_diag = img.clone();
    cpu::chromatic_aberration(&mut half_diag, w, h, 0.5 * diag, RGB_TINTS, 1.0);
    assert!(
        half_diag.iter().step_by(4).all(|&r| r == 1.0),
        "every pixel's red reads the centre's red at Amount = half diagonal"
    );
}

#[test]
fn spectral_taps_span_the_offset_and_normalise() {
    // The variable-sample tap builder (FX-9/K-144, picker-driven A1/K-163): for
    // any count the taps span −1..+1 evenly, each colour column sums to 1
    // (uniform preservation), and the count is clamped to 3..=SPECTRAL_MAX_SAMPLES.
    let rgb = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for n in [3, 9, 16, 64] {
        let taps = spectral_taps(n, rgb);
        assert_eq!(taps.len(), n as usize, "n={n} taps");
        assert!((taps[0][3] - -1.0).abs() < 1e-6, "first tap is the red end");
        assert!(
            (taps[taps.len() - 1][3] - 1.0).abs() < 1e-6,
            "last tap is the blue end"
        );
        // Fractions strictly increase across the span.
        for pair in taps.windows(2) {
            assert!(pair[1][3] > pair[0][3], "n={n}: fractions increase");
        }
        for c in 0..3 {
            let sum: f32 = taps.iter().map(|t| t[c]).sum();
            assert!((sum - 1.0).abs() < 1e-5, "n={n} channel {c} sums to {sum}");
        }
    }
    // Clamping: below 3 and above the max both land in range.
    assert_eq!(spectral_taps(0, rgb).len(), 3);
    assert_eq!(
        spectral_taps(1000, rgb).len(),
        SPECTRAL_MAX_SAMPLES as usize
    );

    // A degenerate all-one-colour picker keeps that colour and zeroes the
    // others (the guarded column-normalisation never divides by zero).
    let all_red = [[1.0f32, 0.0, 0.0]; 3];
    let taps = spectral_taps(9, all_red);
    let rsum: f32 = taps.iter().map(|t| t[0]).sum();
    assert!(
        (rsum - 1.0).abs() < 1e-5,
        "red column still normalises to 1"
    );
    assert!(
        taps.iter().all(|t| t[1] == 0.0 && t[2] == 0.0),
        "no green/blue when the picker has none"
    );
}

#[test]
fn cpu_spectral_split_disperses_and_preserves_uniform() {
    let (w, h) = (17u32, 9u32);
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;

    // The default red/green/blue picker gradient (A1/K-163): red at the −1 end,
    // green astride, blue at the +1 end — the same directional arrangement the
    // old physical basis had, so these assertions are unchanged.
    let rgb = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    // A uniform image is unchanged (the gradient columns are normalised, and
    // clamp addressing keeps edges uniform too).
    let mut uniform = vec![0.0f32; (w * h * 4) as usize];
    for px in uniform.chunks_exact_mut(4) {
        px.copy_from_slice(&[0.5, 0.25, 0.125, 1.0]);
    }
    let before = uniform.clone();
    cpu::spectral_split(&mut uniform, w, h, 3.0, 25.0, false, 9, rgb, 1.0);
    for (i, (a, b)) in uniform.iter().zip(&before).enumerate() {
        assert!((a - b).abs() < 1e-6, "texel {i}: {a} vs {b}");
    }

    // A white impulse on an opaque black frame disperses: red mass
    // lands ahead of the impulse (the classic mode's R direction), blue
    // behind, green astride it — and alpha never moves.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px[3] = 1.0;
    }
    let mid = at(8, 4);
    img[mid..mid + 3].copy_from_slice(&[1.0, 1.0, 1.0]);

    // Mix 0 is the exact identity.
    let mut m0 = img.clone();
    cpu::spectral_split(&mut m0, w, h, 3.0, 45.0, false, 9, rgb, 0.0);
    assert_eq!(m0, img);

    let mut s = img.clone();
    cpu::spectral_split(&mut s, w, h, 2.0, 0.0, false, 9, rgb, 1.0);
    assert!(s[at(10, 4)] > 0.1, "red end lands +2x of the impulse");
    assert!(s[at(6, 4) + 2] > 0.3, "blue end lands -2x of the impulse");
    assert!(s[mid + 1] > 0.3, "green stays astride the impulse");
    assert!(s[at(10, 4) + 2] < 1e-6, "no blue leaks toward the red end");
    assert!(
        s.iter().skip(3).step_by(4).all(|a| *a == 1.0),
        "alpha stays put: mattes never fringe"
    );
}

#[test]
fn flash_envelope_decays_hits_and_holds_statics() {
    use crate::anim::{Keyframe, SideInterp};
    use crate::time::Rational;
    // A static trigger is a constant flash.
    assert_eq!(flash_envelope(&Property::fixed(0.5), 7.0, 0.12), 0.5);
    assert_eq!(flash_envelope(&Property::fixed(2.0), 0.0, 0.12), 1.0);

    // Keyframed: hits at t=1 (full) and t=2 (0.6), decay 0.5s.
    let key = |t: i64, v: f64| Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value: v,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let trig = Property {
        animation: Animation::Keyframed(vec![key(1, 1.0), key(2, 0.6)]),
        extra: serde_json::Map::new(),
    };
    assert_eq!(flash_envelope(&trig, 0.5, 0.5), 0.0, "before the first hit");
    assert_eq!(
        flash_envelope(&trig, 1.0, 0.5),
        1.0,
        "full on the hit frame"
    );
    let half_later = flash_envelope(&trig, 1.5, 0.5);
    assert!(
        (half_later - (-1.0f64).exp()).abs() < 1e-12,
        "1/e after one decay constant"
    );
    assert_eq!(
        flash_envelope(&trig, 2.0, 0.5),
        0.6,
        "second hit wins over the tail"
    );
    // Overlap takes the loudest: right after t=2 the first hit's tail
    // (1.0·e^-2) is quieter than the fresh 0.6 hit.
    let after = flash_envelope(&trig, 2.1, 0.5);
    assert!((after - 0.6 * (-0.2f64).exp()).abs() < 1e-12);

    // Decay 0 flashes only on the exact hit time.
    assert_eq!(flash_envelope(&trig, 1.0, 0.0), 1.0);
    assert_eq!(flash_envelope(&trig, 1.01, 0.0), 0.0);
}

#[test]
fn flash_instantiates_resolves_and_lights_within_the_footprint() {
    let e = instantiate("flash").unwrap();
    assert_eq!(e.float_at("trigger", 0.0), Some(0.0));
    assert_eq!(e.float_at("intensity", 0.0), Some(100.0));
    assert_eq!(e.float_at("decay", 0.0), Some(120.0));
    assert_eq!(e.colour_at("colour", 0.0), Some([1.0, 1.0, 1.0, 1.0]));
    // Trigger 0: resolves to a zero-strength (identity) flash — the
    // §1.2 trigger-driven exemption.
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Flash {
            strength: 0.0,
            colour: [1.0; 4],
            mix: 1.0
        }]
    );

    // CPU semantics: strength 1 paints the footprint the flash colour.
    let mut img = vec![
        0.5, 0.25, 0.1, 1.0, // opaque pixel
        0.2, 0.1, 0.05, 0.5, // half-transparent pixel
        0.0, 0.0, 0.0, 0.0, // empty pixel
    ];
    let before = img.clone();
    cpu::flash(&mut img, 1.0, [2.0, 1.0, 0.5, 1.0], 1.0);
    assert_eq!(&img[0..4], &[2.0, 1.0, 0.5, 1.0], "opaque: flash colour");
    assert_eq!(
        &img[4..8],
        &[1.0, 0.5, 0.25, 0.5],
        "half alpha: premultiplied flash"
    );
    assert_eq!(&img[8..12], &[0.0; 4], "empty pixels never light up");

    // Strength 0 and mix 0 are both the exact identity.
    let mut s0 = before.clone();
    cpu::flash(&mut s0, 0.0, [1.0; 4], 1.0);
    assert_eq!(s0, before);
    let mut m0 = before.clone();
    cpu::flash(&mut m0, 1.0, [1.0; 4], 0.0);
    assert_eq!(m0, before);
}

#[test]
fn colour_balance_instantiates_and_resolves_neutral() {
    let e = instantiate("colour_balance").unwrap();
    assert_eq!(e.colour_at("lift", 0.0), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(e.colour_at("gamma", 0.0), Some([1.0; 4]));
    assert_eq!(e.colour_at("gain", 0.0), Some([1.0; 4]));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::ColourBalance {
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
            mix: 1.0
        }]
    );
}

#[test]
fn saturation_instantiates_and_resolves_neutral() {
    let e = instantiate("saturation").unwrap();
    assert_eq!(e.float_at("saturation", 0.0), Some(100.0));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Saturation {
            saturation: 1.0,
            mix: 1.0
        }]
    );

    // K-135: the hard ceiling is open, so a heavy 400 % resolves to 4.0 —
    // no clamp to 200 — and the schema declares the open range.
    let s = schema("saturation").unwrap();
    let sat = s.params.iter().find(|p| p.id == "saturation").unwrap();
    assert!(matches!(
        sat.kind,
        ParamKind::Float {
            slider: (0.0, 400.0),
            hard: (Some(0.0), None),
            ..
        }
    ));
    let mut heavy = e;
    for p in &mut heavy.params {
        if p.id == "saturation" {
            p.value = EffectValue::Float(Property::fixed(400.0));
        }
    }
    assert_eq!(
        resolve_stack(&[heavy], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
        vec![Resolved::Saturation {
            saturation: 4.0,
            mix: 1.0
        }]
    );
}

#[test]
fn vibrancy_instantiates_and_resolves_neutral() {
    let e = instantiate("vibrancy").unwrap();
    // Default 0 = neutral (K-152): a fresh Vibrancy is the bit-exact identity.
    assert_eq!(e.float_at("amount", 0.0), Some(0.0));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Vibrancy {
            amount: 0.0,
            mix: 1.0
        }]
    );

    // K-135: the ceiling is open, so a heavy 250 % resolves to 2.5 — no clamp.
    let s = schema("vibrancy").unwrap();
    let amt = s.params.iter().find(|p| p.id == "amount").unwrap();
    assert!(matches!(
        amt.kind,
        ParamKind::Float {
            slider: (0.0, 200.0),
            hard: (Some(0.0), None),
            ..
        }
    ));
    let mut heavy = e;
    for p in &mut heavy.params {
        if p.id == "amount" {
            p.value = EffectValue::Float(Property::fixed(250.0));
        }
    }
    assert_eq!(
        resolve_stack(&[heavy], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
        vec![Resolved::Vibrancy {
            amount: 2.5,
            mix: 1.0
        }]
    );
}

#[test]
fn matte_key_instantiates_and_resolves_defaults() {
    let e = instantiate("matte_key").unwrap();
    // The defaults visibly key a green screen (a green screen colour + 100 %
    // gain); despill defaults full-on, and the view defaults to Final.
    assert_eq!(e.colour_at("key", 0.0), Some([0.0, 0.6, 0.0, 1.0]));
    assert_eq!(e.float_at("screen_gain", 0.0), Some(100.0));
    assert_eq!(e.float_at("screen_balance", 0.0), Some(50.0));
    assert_eq!(e.float_at("spill", 0.0), Some(100.0));
    assert_eq!(e.float_at("clip_white", 0.0), Some(100.0));
    assert!(matches!(e.param("view"), Some(EffectValue::Choice(0))));
    assert!(matches!(
        e.param("replace_method"),
        Some(EffectValue::Choice(2)) // Soft colour, as Keylight
    ));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::MatteKey(MatteKeyParams {
            view: 0,
            key: [0.0, 0.6, 0.0, 1.0],
            gain: 1.0,
            balance: 0.5,
            despill_bias: [0.5, 0.5, 0.5, 1.0],
            alpha_bias: [0.5, 0.5, 0.5, 1.0],
            spill: 1.0,
            clip_black: 0.0,
            clip_white: 1.0,
            clip_rollback: 0.0,
            replace_method: 2,
            replace_colour: [0.5, 0.5, 0.5, 1.0],
            mix: 1.0,
        })]
    );
}

#[test]
fn matte_key_migrates_pre_k154_projects() {
    // A project saved before K-154 stored only key / tolerance / softness /
    // spill / mix. It must still resolve (no crash): the Screen colour and Spill
    // carry over, tolerance/softness are ignored, and the new controls take
    // their Keylight defaults.
    let mut e = instantiate("matte_key").unwrap();
    e.params
        .retain(|p| matches!(p.id.as_str(), "key" | "spill" | "mix"));
    e.params.push(crate::model::EffectParam {
        id: "tolerance".into(),
        value: EffectValue::Float(Property::fixed(40.0)),
        extra: serde_json::Map::new(),
    });
    e.params.push(crate::model::EffectParam {
        id: "softness".into(),
        value: EffectValue::Float(Property::fixed(25.0)),
        extra: serde_json::Map::new(),
    });
    // Force the stored Spill to a legacy value to prove it carries over.
    for p in &mut e.params {
        if p.id == "spill" {
            p.value = EffectValue::Float(Property::fixed(30.0));
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::MatteKey(p) = r[0] else {
        panic!("expected a resolved matte key");
    };
    assert_eq!(p.key, [0.0, 0.6, 0.0, 1.0], "screen colour carries over");
    assert!((p.spill - 0.30).abs() < 1e-6, "legacy spill carries over");
    assert_eq!(p.gain, 1.0, "new gain takes its default");
    assert_eq!(p.balance, 0.5, "new balance takes its default");
    assert_eq!(p.view, 0, "new view defaults to Final");
}

#[test]
fn exposure_instantiates_resolves_and_gains_light() {
    let e = instantiate("exposure").unwrap();
    assert_eq!(e.float_at("stops", 0.0), Some(0.0));
    // 0 stops resolves to a neutral factor of 1.0.
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Exposure {
            factor: 1.0,
            mix: 1.0
        }]
    );
    // The CPU reference: 0 stops is identity; +1 stop (factor 2) doubles
    // RGB and leaves alpha alone; Mix 0 is the identity at any factor.
    let mut neutral = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::exposure(&mut neutral, 1.0, 1.0);
    assert_eq!(neutral, vec![0.4, 0.5, 0.6, 1.0]);
    let mut bright = vec![0.2_f32, 0.3, 0.1, 0.8];
    cpu::exposure(&mut bright, 2.0, 1.0);
    assert_eq!(bright, vec![0.4, 0.6, 0.2, 0.8]);
    let mut mixed = vec![0.2_f32, 0.3, 0.1, 1.0];
    cpu::exposure(&mut mixed, 3.0, 0.0);
    assert_eq!(mixed, vec![0.2, 0.3, 0.1, 1.0]);
}

#[test]
fn temperature_instantiates_resolves_and_warms_and_cools() {
    let e = instantiate("temperature").unwrap();
    assert_eq!(e.float_at("temperature", 0.0), Some(0.0));
    // Temperature 0 resolves to neutral gains of exactly 1.0 each.
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Temperature {
            gain_r: 1.0,
            gain_b: 1.0,
            mix: 1.0
        }]
    );
    // K-135: the range widens to ±150 slider / ±200 hard, with the stronger
    // ±0.75·k gain. +100 resolves to gains (1.75, 0.25): red boosted, blue
    // cut hard. −100 is the mirror (0.25, 1.75). The resolve step owns the
    // gain formula.
    let s = schema("temperature").unwrap();
    let temp = s.params.iter().find(|p| p.id == "temperature").unwrap();
    assert!(matches!(
        temp.kind,
        ParamKind::Float {
            slider: (-150.0, 150.0),
            hard: (Some(-200.0), Some(200.0)),
            ..
        }
    ));
    let mut warm = e.clone();
    for p in &mut warm.params {
        if p.id == "temperature" {
            p.value = EffectValue::Float(Property::fixed(100.0));
        }
    }
    assert_eq!(
        resolve_stack(&[warm], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
        vec![Resolved::Temperature {
            gain_r: 1.75,
            gain_b: 0.25,
            mix: 1.0
        }]
    );
    // At the +200 hard extreme the blue gain would be 1 − 1.5 = −0.5; the
    // resolver floors it at 0 (never a negative channel), red at 2.5.
    let mut hot = e.clone();
    for p in &mut hot.params {
        if p.id == "temperature" {
            p.value = EffectValue::Float(Property::fixed(200.0));
        }
    }
    assert_eq!(
        resolve_stack(&[hot], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
        vec![Resolved::Temperature {
            gain_r: 2.5,
            gain_b: 0.0,
            mix: 1.0
        }]
    );
    let mut cool = e;
    for p in &mut cool.params {
        if p.id == "temperature" {
            p.value = EffectValue::Float(Property::fixed(-100.0));
        }
    }
    assert_eq!(
        resolve_stack(&[cool], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
        vec![Resolved::Temperature {
            gain_r: 0.25,
            gain_b: 1.75,
            mix: 1.0
        }]
    );
    // The CPU reference: neutral gains are the bit-exact identity; a warm
    // shift (gains 1.5 / 0.5) boosts red and cuts blue, green and alpha
    // untouched; Mix 0 is the identity at any gains.
    let mut neutral = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::temperature(&mut neutral, 1.0, 1.0, 1.0);
    assert_eq!(neutral, vec![0.4, 0.5, 0.6, 1.0]);
    let mut hot = vec![0.5_f32, 0.5, 0.5, 0.8];
    cpu::temperature(&mut hot, 1.5, 0.5, 1.0);
    assert_eq!(hot, vec![0.75, 0.5, 0.25, 0.8]);
    let mut mixed = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::temperature(&mut mixed, 1.5, 0.5, 0.0);
    assert_eq!(mixed, vec![0.4, 0.5, 0.6, 1.0]);
}

#[test]
fn invert_instantiates_resolves_and_inverts() {
    let e = instantiate("invert").unwrap();
    // The only parameter is Mix, defaulting to 100 %.
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(r, vec![Resolved::Invert { mix: 1.0 }]);

    // The CPU reference: an opaque pixel inverts as 1 − c, alpha untouched.
    let mut opaque = vec![0.2_f32, 0.5, 0.9, 1.0];
    cpu::invert(&mut opaque, 1.0);
    for (v, want) in opaque.iter().zip([0.8_f32, 0.5, 0.1, 1.0]) {
        assert!((v - want).abs() < 1e-6, "opaque invert: {v} vs {want}");
    }
    // Mix 0 is the identity at any input.
    let mut m0 = vec![0.2_f32, 0.5, 0.9, 1.0];
    cpu::invert(&mut m0, 0.0);
    assert_eq!(m0, vec![0.2, 0.5, 0.9, 1.0]);

    // Half-alpha pixel: invert runs on the unpremultiplied colour and is
    // re-premultiplied — the round trip a naive invert of premultiplied
    // colour gets wrong. Straight (0.4,0.6,0.8) at alpha 0.5 is stored
    // premultiplied as (0.2,0.3,0.4); inverting the straight colour gives
    // (0.6,0.4,0.2), re-premultiplied to (0.3,0.2,0.1); alpha untouched.
    let mut half = vec![0.2_f32, 0.3, 0.4, 0.5];
    cpu::invert(&mut half, 1.0);
    for (v, want) in half.iter().zip([0.3_f32, 0.2, 0.1, 0.5]) {
        assert!((v - want).abs() < 1e-6, "half-alpha invert: {v} vs {want}");
    }

    // Scene-linear HDR values above 1 invert to honest negatives (§2.1).
    let mut hdr = vec![2.0_f32, 3.0, 0.5, 1.0];
    cpu::invert(&mut hdr, 1.0);
    for (v, want) in hdr.iter().zip([-1.0_f32, -2.0, 0.5, 1.0]) {
        assert!((v - want).abs() < 1e-6, "hdr invert: {v} vs {want}");
    }
}

#[test]
fn tint_instantiates_resolves_and_maps_luma() {
    let e = instantiate("tint").unwrap();
    assert_eq!(e.colour_at("black", 0.0), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(e.colour_at("white", 0.0), Some([1.0, 1.0, 1.0, 1.0]));
    // Defaults resolve to black→black, white→white (a greyscale mapping).
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Tint {
            black: [0.0, 0.0, 0.0],
            white: [1.0, 1.0, 1.0],
            mix: 1.0
        }]
    );

    // The CPU reference: default black→black / white→white maps every pixel
    // to its own Rec.709 luma in all three channels (a greyscale).
    let rgb = [0.8_f32, 0.2, 0.5];
    let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    let mut grey = vec![rgb[0], rgb[1], rgb[2], 1.0];
    cpu::tint(&mut grey, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0);
    for v in grey.iter().take(3) {
        assert!((v - luma).abs() < 1e-6, "greyscale luma: {v} vs {luma}");
    }
    assert_eq!(grey[3], 1.0, "alpha untouched");

    // A duotone: black→(0.1,0,0.2), white→(0.9,0.8,1.0). Each channel lerps
    // by the pixel's luma. Mix 0 is the identity at any colours.
    let black = [0.1_f32, 0.0, 0.2];
    let white = [0.9_f32, 0.8, 1.0];
    let mut duo = vec![rgb[0], rgb[1], rgb[2], 1.0];
    cpu::tint(&mut duo, black, white, 1.0);
    for c in 0..3 {
        let want = black[c] + (white[c] - black[c]) * luma;
        assert!(
            (duo[c] - want).abs() < 1e-6,
            "duotone ch{c}: {} vs {want}",
            duo[c]
        );
    }
    let mut m0 = vec![rgb[0], rgb[1], rgb[2], 1.0];
    cpu::tint(&mut m0, black, white, 0.0);
    assert_eq!(m0, vec![rgb[0], rgb[1], rgb[2], 1.0]);

    // Half-alpha pixel: the map runs on the unpremultiplied colour and is
    // re-premultiplied. Straight (0.8,0.2,0.5) at alpha 0.5 is stored
    // premultiplied as (0.4,0.1,0.25); with defaults it maps to the straight
    // luma in each channel, re-premultiplied to luma·0.5; alpha untouched.
    let mut half = vec![0.4_f32, 0.1, 0.25, 0.5];
    cpu::tint(&mut half, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0);
    for v in half.iter().take(3) {
        assert!((v - luma * 0.5).abs() < 1e-6, "half-alpha map: {v}");
    }
    assert_eq!(half[3], 0.5, "alpha untouched");
}

#[test]
fn hue_shift_is_neutral_at_zero_and_preserves_grey_and_luma() {
    let e = instantiate("hue_shift").unwrap();
    assert_eq!(e.float_at("angle", 0.0), Some(0.0));
    // Preserve luminance is on by default (K-136).
    assert_eq!(
        e.param("preserve_luminance"),
        Some(&EffectValue::Bool(true))
    );
    // 0° resolves to the identity matrix.
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::HueShift {
            m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            mix: 1.0
        }]
    );
    // Identity is bit-exact identity.
    let mut a = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::hue_shift(&mut a, hue_matrix(0.0), 1.0);
    assert_eq!(a, vec![0.4, 0.5, 0.6, 1.0]);
    // Rotating a neutral grey leaves it grey (rows each ~sum to 1), and any
    // rotation preserves Rec.709 luma to within rounding.
    let m = hue_matrix(90.0);
    let grey = [0.5_f32, 0.5, 0.5];
    let out = [
        m[0] * grey[0] + m[1] * grey[1] + m[2] * grey[2],
        m[3] * grey[0] + m[4] * grey[1] + m[5] * grey[2],
        m[6] * grey[0] + m[7] * grey[1] + m[8] * grey[2],
    ];
    for c in out {
        assert!((c - 0.5).abs() < 1e-3, "grey stays grey: {c}");
    }
    let lin = [0.8_f32, 0.2, 0.5];
    let luma_in = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    let ro = [
        m[0] * lin[0] + m[1] * lin[1] + m[2] * lin[2],
        m[3] * lin[0] + m[4] * lin[1] + m[5] * lin[2],
        m[6] * lin[0] + m[7] * lin[1] + m[8] * lin[2],
    ];
    let luma_out = 0.2126 * ro[0] + 0.7152 * ro[1] + 0.0722 * ro[2];
    assert!((luma_in - luma_out).abs() < 1e-3, "luma preserved");
}

#[test]
fn hue_shift_preserve_luminance_toggle_picks_the_matrix_branch() {
    // K-136: Preserve luminance off resolves to the plain-RGB rotation
    // (equal-weight spin about the grey axis); on keeps the Rec.709
    // constant-luminance one. The resolve step owns the branch; the kernel
    // is matrix-general, so both share one op.
    let mut off = instantiate("hue_shift").unwrap();
    for p in &mut off.params {
        match p.id.as_str() {
            "angle" => p.value = EffectValue::Float(Property::fixed(90.0)),
            "preserve_luminance" => p.value = EffectValue::Bool(false),
            _ => {}
        }
    }
    assert_eq!(
        resolve_stack(
            std::slice::from_ref(&off),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE
        ),
        vec![Resolved::HueShift {
            m: hue_matrix_rgb(90.0),
            mix: 1.0
        }]
    );
    // Preserve on (the default) at the same angle uses the Rec.709 matrix,
    // and the two matrices genuinely differ.
    let mut on = instantiate("hue_shift").unwrap();
    for p in &mut on.params {
        if p.id == "angle" {
            p.value = EffectValue::Float(Property::fixed(90.0));
        }
    }
    assert_eq!(
        resolve_stack(
            std::slice::from_ref(&on),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE
        ),
        vec![Resolved::HueShift {
            m: hue_matrix(90.0),
            mix: 1.0
        }]
    );
    assert_ne!(
        hue_matrix(90.0),
        hue_matrix_rgb(90.0),
        "the two hue branches are distinct"
    );

    // Both branches are the exact identity at 0° (neutral point bit-exact).
    assert_eq!(
        hue_matrix_rgb(0.0),
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
    );

    // The plain-RGB rotation keeps a neutral grey grey (rows each sum to 1),
    // but does NOT hold Rec.709 luminance — that is the whole point of the
    // toggle. It preserves the equal-weight (R+G+B) sum instead.
    let m = hue_matrix_rgb(120.0);
    let grey = [0.5_f32, 0.5, 0.5];
    let g_out = [
        m[0] * grey[0] + m[1] * grey[1] + m[2] * grey[2],
        m[3] * grey[0] + m[4] * grey[1] + m[5] * grey[2],
        m[6] * grey[0] + m[7] * grey[1] + m[8] * grey[2],
    ];
    for c in g_out {
        assert!((c - 0.5).abs() < 1e-3, "grey stays grey: {c}");
    }
    let lin = [0.8_f32, 0.2, 0.5];
    let ro = [
        m[0] * lin[0] + m[1] * lin[1] + m[2] * lin[2],
        m[3] * lin[0] + m[4] * lin[1] + m[5] * lin[2],
        m[6] * lin[0] + m[7] * lin[1] + m[8] * lin[2],
    ];
    let sum_in = lin[0] + lin[1] + lin[2];
    let sum_out = ro[0] + ro[1] + ro[2];
    assert!((sum_in - sum_out).abs() < 1e-3, "RGB sum preserved");
    let luma_in = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    let luma_out = 0.2126 * ro[0] + 0.7152 * ro[1] + 0.0722 * ro[2];
    assert!(
        (luma_in - luma_out).abs() > 1e-3,
        "plain-RGB rotation changes Rec.709 luma: {luma_in} vs {luma_out}"
    );
}

#[test]
fn contrast_is_neutral_at_100_and_pivots_about_mid_grey() {
    let e = instantiate("contrast").unwrap();
    assert_eq!(e.float_at("contrast", 0.0), Some(100.0));
    // 100 % resolves to a neutral factor of 1.0.
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(r, vec![Resolved::Contrast { k: 1.0, mix: 1.0 }]);

    // Neutral (k 1.0) is the bit-exact identity; Mix 0 is too at any k.
    let mut n = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::contrast(&mut n, 1.0, 1.0);
    assert_eq!(n, vec![0.4, 0.5, 0.6, 1.0]);
    let mut m0 = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::contrast(&mut m0, 2.5, 0.0);
    assert_eq!(m0, vec![0.4, 0.5, 0.6, 1.0]);

    // Mid-grey (0.5) is the fixed point of the pivot at any k.
    let mut grey = vec![0.5_f32, 0.5, 0.5, 1.0];
    cpu::contrast(&mut grey, 2.0, 1.0);
    for v in grey.iter().take(3) {
        assert!((v - 0.5).abs() < 1e-6, "mid-grey stays put");
    }

    // Opaque pixel, k 2.0: each channel moves twice as far from 0.5.
    let mut op = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::contrast(&mut op, 2.0, 1.0);
    for (v, want) in op.iter().zip([0.3_f32, 0.5, 0.7, 1.0]) {
        assert!((v - want).abs() < 1e-6, "opaque grade: {v} vs {want}");
    }

    // Half-alpha pixel: the grade runs on the unpremultiplied colour and
    // is re-premultiplied — the premult round trip that a naive grade on
    // premultiplied colour would get wrong. Straight (0.4,0.6,0.5) at
    // alpha 0.5 is stored premultiplied as (0.2,0.3,0.25); k 2.0 grades
    // the straight colour to (0.3,0.7,0.5), re-premultiplied to
    // (0.15,0.35,0.25); alpha is untouched.
    let mut half = vec![0.2_f32, 0.3, 0.25, 0.5];
    cpu::contrast(&mut half, 2.0, 1.0);
    for (v, want) in half.iter().zip([0.15_f32, 0.35, 0.25, 0.5]) {
        assert!((v - want).abs() < 1e-6, "half-alpha grade: {v} vs {want}");
    }

    // Empty pixels stay empty (unpremult reads black, re-premult is zero).
    let mut empty = vec![0.0_f32, 0.0, 0.0, 0.0];
    cpu::contrast(&mut empty, 2.0, 1.0);
    assert_eq!(empty, vec![0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn gamma_is_neutral_at_one_and_curves_per_channel() {
    let e = instantiate("gamma").unwrap();
    assert_eq!(e.float_at("gamma", 0.0), Some(1.0));
    // Default 1.0 resolves to a neutral gamma.
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Gamma {
            gamma: 1.0,
            mix: 1.0
        }]
    );

    // Neutral (gamma 1.0) is the bit-exact identity; Mix 0 is too at any
    // gamma (a short-circuit, not a reliance on pow(x, 1) == x).
    let mut n = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::gamma(&mut n, 1.0, 1.0);
    assert_eq!(n, vec![0.4, 0.5, 0.6, 1.0]);
    let mut m0 = vec![0.4_f32, 0.5, 0.6, 1.0];
    cpu::gamma(&mut m0, 2.2, 0.0);
    assert_eq!(m0, vec![0.4, 0.5, 0.6, 1.0]);

    // Opaque pixel, gamma 2.0: each channel becomes pow(u, 1/2).
    let mut op = vec![0.25_f32, 0.5, 0.81, 1.0];
    cpu::gamma(&mut op, 2.0, 1.0);
    for (v, want) in op.iter().zip([0.5_f32, 0.5_f32.powf(0.5), 0.9, 1.0]) {
        assert!((v - want).abs() < 1e-6, "opaque curve: {v} vs {want}");
    }

    // 0 and 1 are fixed points of any gamma (pow(0) = 0, pow(1) = 1).
    let mut ends = vec![0.0_f32, 1.0, 0.0, 1.0];
    cpu::gamma(&mut ends, 0.45, 1.0);
    assert!((ends[0] - 0.0).abs() < 1e-6 && (ends[1] - 1.0).abs() < 1e-6);

    // Half-alpha pixel: the curve runs on the unpremultiplied colour and is
    // re-premultiplied — the premult round trip a naive curve on
    // premultiplied colour would get wrong. Straight (0.25,0.81,0.49) at
    // alpha 0.5 is stored premultiplied as (0.125,0.405,0.245); gamma 2.0
    // curves the straight colour to (0.5,0.9,0.7), re-premultiplied to
    // (0.25,0.45,0.35); alpha is untouched.
    let mut half = vec![0.125_f32, 0.405, 0.245, 0.5];
    cpu::gamma(&mut half, 2.0, 1.0);
    for (v, want) in half.iter().zip([0.25_f32, 0.45, 0.35, 0.5]) {
        assert!((v - want).abs() < 1e-6, "half-alpha curve: {v} vs {want}");
    }

    // Negative scene-linear input is clamped to 0 before the pow (pow of a
    // negative base is undefined), so it curves to 0 rather than NaN.
    let mut neg = vec![-0.2_f32, 0.0, 0.0, 1.0];
    cpu::gamma(&mut neg, 2.0, 1.0);
    assert!(
        neg[0].is_finite() && neg[0].abs() < 1e-6,
        "clamped, not NaN: {}",
        neg[0]
    );

    // Empty pixels stay empty (unpremult reads black, re-premult is zero).
    let mut empty = vec![0.0_f32, 0.0, 0.0, 0.0];
    cpu::gamma(&mut empty, 2.0, 1.0);
    assert_eq!(empty, vec![0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn vignette_instantiates_and_resolves() {
    let e = instantiate("vignette").unwrap();
    assert_eq!(e.float_at("amount", 0.0), Some(0.5));
    assert_eq!(e.float_at("radius", 0.0), Some(0.75));
    assert_eq!(e.float_at("softness", 0.0), Some(0.5));
    assert_eq!(e.float_at("roundness", 0.0), Some(1.0));
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Vignette {
            amount: 0.5,
            radius: 0.75,
            softness: 0.5,
            roundness: 1.0,
            ramp: 1.0,
            mix: 1.0,
        }]
    );

    // K-135: Softness is open above, so 1.5 resolves un-clamped (Amount,
    // Radius and Roundness keep their 0..1 caps).
    let s = schema("vignette").unwrap();
    let soft = s.params.iter().find(|p| p.id == "softness").unwrap();
    assert!(matches!(
        soft.kind,
        ParamKind::Float {
            slider: (0.0, 2.0),
            hard: (Some(0.0), None),
            ..
        }
    ));
    let mut wide = e;
    for p in &mut wide.params {
        if p.id == "softness" {
            p.value = EffectValue::Float(Property::fixed(1.5));
        }
    }
    assert_eq!(
        resolve_stack(&[wide], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
        vec![Resolved::Vignette {
            amount: 0.5,
            radius: 0.75,
            softness: 1.5,
            roundness: 1.0,
            ramp: 1.0,
            mix: 1.0,
        }]
    );
}

#[test]
fn cpu_vignette_darkens_the_corners_and_is_neutral_at_zero_amount() {
    let (w, h) = (20u32, 20u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px.copy_from_slice(&[1.0, 1.0, 1.0, 1.0]); // opaque white
    }
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;

    // Amount 0 and mix 0 are both the exact identity (the early return
    // and the general blend formula's own 1·x + 0·y identity).
    let mut a0 = img.clone();
    cpu::vignette(&mut a0, w, h, 0.0, 0.75, 0.5, 1.0, 1.0, 1.0);
    assert_eq!(a0, img);
    let mut m0 = img.clone();
    cpu::vignette(&mut m0, w, h, 0.8, 0.2, 0.1, 1.0, 1.0, 0.0);
    assert_eq!(m0, img);

    // A tight, hard-edged, fully-strength vignette: the centre stays
    // lit, the corner goes dark, alpha is never touched.
    let mut v = img.clone();
    cpu::vignette(&mut v, w, h, 1.0, 0.2, 0.05, 1.0, 1.0, 1.0);
    let centre = at(10, 10);
    let corner = at(0, 0);
    assert!(v[centre] > 0.95, "centre stays lit: {}", v[centre]);
    assert!(v[corner] < 0.05, "corner goes dark: {}", v[corner]);
    assert_eq!(v[corner + 3], 1.0, "alpha is never touched");

    // K-135: Softness > 1 is a legal, wider feather (not clamped to 1). At
    // the same tight Radius, softness 1.5 spreads the falloff so the corner
    // is only partly darkened where the hard-edged case above was near
    // black, and every value stays finite and in gamut — no artefacts.
    let mut wide = img.clone();
    cpu::vignette(&mut wide, w, h, 1.0, 0.2, 1.5, 1.0, 1.0, 1.0);
    assert!(
        wide[corner] > v[corner],
        "wider feather darkens the corner less: {} vs {}",
        wide[corner],
        v[corner]
    );
    for s in &wide {
        assert!(s.is_finite() && *s >= 0.0, "no artefacts: {s}");
    }
}

#[test]
fn cpu_vignette_roundness_changes_the_shape_on_a_non_square_frame() {
    let (w, h) = (40u32, 20u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px.copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
    }
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    // The long edge's midpoint: circular roundness (normalised by the
    // short side, h here) reads its x distance as almost twice as far
    // as elliptical roundness (normalised by w itself) does, so a
    // Radius/Softness pair that fully darkens only one metric's reach
    // tells the two apart.
    let edge_right_mid = at(w - 1, h / 2);

    let mut circular = img.clone();
    cpu::vignette(&mut circular, w, h, 1.0, 0.9, 0.2, 1.0, 1.0, 1.0);
    let mut elliptical = img.clone();
    cpu::vignette(&mut elliptical, w, h, 1.0, 0.9, 0.2, 0.0, 1.0, 1.0);

    assert!(
        circular[edge_right_mid] < 1e-5,
        "circular is fully dark this far out: {}",
        circular[edge_right_mid]
    );
    assert!(
        elliptical[edge_right_mid] > 0.0,
        "elliptical has not fully darkened here: {}",
        elliptical[edge_right_mid]
    );
    assert!(
        circular[edge_right_mid] < elliptical[edge_right_mid],
        "circular darkens the long edge harder than elliptical: \
             circular {} elliptical {}",
        circular[edge_right_mid],
        elliptical[edge_right_mid]
    );
}

/// One opaque mid-grey-ish pixel, one half-alpha, one HDR, one empty —
/// the colour-effect test quartet.
fn colour_quartet() -> Vec<f32> {
    vec![
        0.25, 0.5, 0.1, 1.0, //
        0.1, 0.2, 0.05, 0.5, //
        4.0, 2.0, 1.0, 1.0, //
        0.0, 0.0, 0.0, 0.0,
    ]
}

#[test]
fn cpu_colour_balance_stages_behave() {
    let img = colour_quartet();

    // A neutral balance is the bit-exact identity (K-090 split: the
    // whole effect short-circuits, no unpremultiply round trip).
    let mut n = img.clone();
    cpu::colour_balance(&mut n, [0.0; 3], [1.0; 3], [1.0; 3], 1.0);
    assert_eq!(n, img);

    // Mix 0 is the exact identity whatever the balance.
    let mut m0 = img.clone();
    cpu::colour_balance(&mut m0, [0.5; 3], [2.0; 3], [3.0; 3], 0.0);
    assert_eq!(m0, img);

    // Gain doubles linear values; HDR stays unclipped (§2.1).
    let mut g = img.clone();
    cpu::colour_balance(&mut g, [0.0; 3], [1.0; 3], [2.0; 3], 1.0);
    assert_eq!(g[0], 0.5);
    assert_eq!(g[8], 8.0, "highlights never clip");

    // Lift raises blacks (empty alpha stays empty: premultiplied zero).
    let mut l = img.clone();
    cpu::colour_balance(&mut l, [0.1; 3], [1.0; 3], [1.0; 3], 1.0);
    assert!((l[2] - 0.2).abs() < 1e-6, "0.1 blue lifted by 0.1");
    assert_eq!(&l[12..16], &[0.0; 4], "empty pixels stay empty");

    // Gamma 2 is a square root in linear: 0.25 → 0.5.
    let mut ga = img.clone();
    cpu::colour_balance(&mut ga, [0.0; 3], [2.0; 3], [1.0; 3], 1.0);
    assert!((ga[0] - 0.5).abs() < 1e-6);

    // Alpha is untouched by any of it.
    for v in [&n, &m0, &g, &l, &ga] {
        assert_eq!(v[3], 1.0);
        assert_eq!(v[7], 0.5);
    }
}

#[test]
fn cpu_saturation_behaves() {
    let img = colour_quartet();

    // Saturation 1 is the bit-exact identity (whole-effect
    // short-circuit, K-090 split).
    let mut n = img.clone();
    cpu::saturate(&mut n, 1.0, 1.0);
    assert_eq!(n, img);

    // Mix 0 is the exact identity whatever the saturation.
    let mut m0 = img.clone();
    cpu::saturate(&mut m0, 0.0, 0.0);
    assert_eq!(m0, img);

    // Saturation 0 collapses to Rec. 709 luma (true greyscale).
    let mut s = img.clone();
    cpu::saturate(&mut s, 0.0, 1.0);
    let luma = 0.25 * cpu::LUMA[0] + 0.5 * cpu::LUMA[1] + 0.1 * cpu::LUMA[2];
    for (c, v) in s.iter().take(3).enumerate() {
        assert!((v - luma).abs() < 1e-6, "channel {c} at luma");
    }
    // The half-alpha pixel desaturates in unpremultiplied space: its
    // premultiplied channels all land on (unpremult luma) × alpha.
    let luma_half = (0.2 * cpu::LUMA[0] + 0.4 * cpu::LUMA[1] + 0.1 * cpu::LUMA[2]) * 0.5;
    for c in 0..3 {
        assert!((s[4 + c] - luma_half).abs() < 1e-6, "channel {c}");
    }
    assert_eq!(&s[12..16], &[0.0; 4], "empty pixels stay empty");

    // Oversaturation spreads channels apart and clamps at zero, never
    // clipping highlights (§2.1).
    let mut o = img.clone();
    cpu::saturate(&mut o, 2.0, 1.0);
    assert!(o[1] > 0.5, "dominant green pushes up");
    assert!(o[2] >= 0.0, "recessive blue clamps at zero, not negative");
    assert!(o[8] > 4.0, "HDR red keeps its headroom");

    // Alpha is untouched by any of it.
    for v in [&n, &m0, &s, &o] {
        assert_eq!(v[3], 1.0);
        assert_eq!(v[7], 0.5);
    }
}

#[test]
fn cpu_vibrance_behaves() {
    let img = colour_quartet();

    // Amount 0 is the bit-exact identity (whole-effect short-circuit, K-152).
    let mut n = img.clone();
    cpu::vibrance(&mut n, 0.0, 1.0);
    assert_eq!(n, img);

    // Mix 0 is the exact identity whatever the amount.
    let mut m0 = img.clone();
    cpu::vibrance(&mut m0, 1.0, 0.0);
    assert_eq!(m0, img);

    // The defining property: a boost lifts LESS-saturated pixels MORE. Two
    // opaque pixels — one near-neutral (low chroma), one vivid — boosted at
    // the same amount: the near-neutral's colourfulness grows by the larger
    // factor.
    let spread = |px: &[f32]| {
        let mx = px[0].max(px[1]).max(px[2]);
        let mn = px[0].min(px[1]).min(px[2]);
        mx - mn
    };
    let mut pair = vec![
        0.50, 0.55, 0.45, 1.0, // low saturation
        0.90, 0.10, 0.10, 1.0, // high saturation
    ];
    let before_low = spread(&pair[0..4]);
    let before_high = spread(&pair[4..8]);
    cpu::vibrance(&mut pair, 1.0, 1.0);
    let after_low = spread(&pair[0..4]);
    let after_high = spread(&pair[4..8]);
    assert!(
        after_low > before_low && after_high > before_high,
        "both pixels gain saturation"
    );
    assert!(
        after_low / before_low > after_high / before_high,
        "the less-saturated pixel gains more: {} vs {}",
        after_low / before_low,
        after_high / before_high
    );

    // Alpha is untouched; a transparent pixel stays empty.
    let mut q = img.clone();
    cpu::vibrance(&mut q, 1.5, 1.0);
    assert_eq!(q[3], 1.0);
    assert_eq!(q[7], 0.5);
    assert_eq!(&q[12..16], &[0.0; 4], "empty pixels stay empty");
}

#[test]
fn cpu_matte_key_behaves() {
    // A base op: default green screen, unit gain, mid balance, neutral biases,
    // no clips. `view` / `spill` / `replace_method` / `mix` are varied per case.
    let base = |view: u32, gain: f32, spill: f32, replace: u32, mix: f32| MatteKeyParams {
        view,
        key: [0.0, 0.6, 0.0, 1.0],
        gain,
        balance: 0.5,
        despill_bias: [0.5, 0.5, 0.5, 1.0],
        alpha_bias: [0.5, 0.5, 0.5, 1.0],
        spill,
        clip_black: 0.0,
        clip_white: 1.0,
        clip_rollback: 0.0,
        replace_method: replace,
        replace_colour: [0.5, 0.5, 0.5, 1.0],
        mix,
    };

    // A pixel exactly the screen colour keys out fully (alpha → 0), and its
    // premultiplied colour collapses with it.
    let mut on_key = vec![0.0_f32, 0.6, 0.0, 1.0];
    cpu::matte_key(&mut on_key, &base(0, 1.0, 1.0, 3, 1.0));
    assert_eq!(
        on_key,
        vec![0.0, 0.0, 0.0, 0.0],
        "the screen colour is removed"
    );

    // A half-alpha screen pixel (premultiplied [0,0.3,0,0.5] = straight
    // [0,0.6,0]) keys to nothing too — the keyer works on straight colour.
    let mut half = vec![0.0_f32, 0.3, 0.0, 0.5];
    cpu::matte_key(&mut half, &base(0, 1.0, 1.0, 3, 1.0));
    assert_eq!(
        half,
        vec![0.0, 0.0, 0.0, 0.0],
        "partial-alpha screen removed"
    );

    // A far-from-screen colour (red) is kept exactly — no primary excess, so
    // nothing to despill and nothing to replace.
    let red = vec![0.8_f32, 0.0, 0.0, 1.0];
    let mut r = red.clone();
    cpu::matte_key(&mut r, &base(0, 1.0, 1.0, 2, 1.0));
    assert_eq!(r, red, "far-from-screen pixels are kept exactly");

    // Mix 0 is the exact identity whatever the settings.
    let mut m0 = red.clone();
    cpu::matte_key(&mut m0, &base(0, 1.0, 1.0, 2, 0.0));
    assert_eq!(m0, red, "Mix 0 is the identity");

    // Despill: a kept pixel with a green excess over its red/blue reference has
    // its green pulled down to that reference at full despill. Gain 0 keeps the
    // pixel fully opaque so the despilled colour is what lands. [0.4,0.6,0.4]
    // has a red/blue reference of 0.4, so full despill flattens it to grey 0.4.
    let mut spill = vec![0.4_f32, 0.6, 0.4, 1.0];
    cpu::matte_key(&mut spill, &base(0, 0.0, 1.0, 3, 1.0));
    for (c, v) in spill.iter().take(3).enumerate() {
        assert!(
            (v - 0.4).abs() < 1e-6,
            "channel {c} despilled to the reference"
        );
    }
    assert_eq!(spill[3], 1.0, "a kept pixel keeps its alpha");

    // The key is continuous: a pixel with a middling green excess keeps a
    // partial alpha, never a hard 0 or 1 — what keeps the effect oracle-safe
    // (§1.6). [0.3,0.5,0.3] has excess 0.2 against a screen excess of 0.6, so
    // raw = 1/3 and the matte lands at 2/3. Spill off, so colour is untouched.
    let mut edge = vec![0.3_f32, 0.5, 0.3, 1.0];
    cpu::matte_key(&mut edge, &base(0, 1.0, 0.0, 3, 1.0));
    assert!(
        edge[3] > 0.0 && edge[3] < 1.0,
        "soft edge keeps a partial alpha: {}",
        edge[3]
    );

    // Screen matte view: the matte itself as opaque greyscale. The edge pixel's
    // matte is 2/3, so every RGB channel reads 2/3 and alpha is 1.
    let mut mv = vec![0.3_f32, 0.5, 0.3, 1.0];
    cpu::matte_key(&mut mv, &base(1, 1.0, 0.0, 3, 1.0));
    for (c, v) in mv.iter().take(3).enumerate() {
        assert!((v - 2.0 / 3.0).abs() < 1e-4, "matte channel {c} shows 2/3");
    }
    assert_eq!(mv[3], 1.0, "the screen-matte view is opaque");

    // Blue screens key too: the primary axis follows the screen colour's max
    // channel, so a blue key removes a blue pixel and keeps a red one.
    let blue_key = MatteKeyParams {
        key: [0.0, 0.0, 0.6, 1.0],
        ..base(0, 1.0, 1.0, 3, 1.0)
    };
    let mut on_blue = vec![0.0_f32, 0.0, 0.6, 1.0];
    cpu::matte_key(&mut on_blue, &blue_key);
    assert_eq!(on_blue, vec![0.0, 0.0, 0.0, 0.0], "a blue screen keys out");
    let mut red2 = vec![0.8_f32, 0.0, 0.0, 1.0];
    cpu::matte_key(&mut red2, &blue_key);
    assert_eq!(red2, vec![0.8, 0.0, 0.0, 1.0], "red survives a blue key");
}

#[test]
fn blur_family_split_resolves_each_effect_and_loads_legacy_as_gaussian() {
    // K-137: the old mode-driven blur is now three single-purpose effects.
    // Gaussian (match_name "blur") resolves at its Radius, fixed Repeat edge.
    let gaussian = instantiate("blur").unwrap();
    assert!(
        gaussian.param("mode").is_none(),
        "the mode control is gone (K-137)"
    );
    let r = resolve_stack(
        std::slice::from_ref(&gaussian),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Blur {
            radius_px: 15.0, // 1.5% of a 1000px diagonal
            edge: 1,
            mix: 1.0
        }]
    );

    // Directional blur reads Length/Angle (10% of 1000 = 100px), fixed Repeat.
    let dir = instantiate("directional_blur").unwrap();
    assert_eq!(dir.float_at("length", 0.0), Some(10.0));
    let r = resolve_stack(
        std::slice::from_ref(&dir),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::DirBlur {
            length_px: 100.0,
            angle_deg: 0.0,
            edge: 1,
            mix: 1.0
        }]
    );

    // Radial blur reads Centre/Amount/Type/Edges: Centre resolves to a
    // *fraction* (30/70%, unconverted — resolve_stack has no width/height to
    // scale it by), Amount 8% of 1000 = 80px, Type defaults to Spin, Edges
    // to Repeat.
    let mut radial = instantiate("radial_blur").unwrap();
    for p in &mut radial.params {
        match p.id.as_str() {
            "centre_x" => p.value = EffectValue::Float(Property::fixed(30.0)),
            "centre_y" => p.value = EffectValue::Float(Property::fixed(70.0)),
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&radial),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::RadialBlur {
            centre_frac: [0.3, 0.7],
            amount_px: 80.0,
            spin: true,
            edge: 1,
            mix: 1.0
        }]
    );

    // The Type choice flips Spin/Zoom; Edges is honoured (Mirror = 2).
    for p in &mut radial.params {
        match p.id.as_str() {
            "radial_type" => p.value = EffectValue::Choice(1),
            "edge" => p.value = EffectValue::Choice(2),
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&radial),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert!(matches!(
        r[..],
        [Resolved::RadialBlur {
            spin: false,
            edge: 2,
            ..
        }]
    ));

    // A project saved with the old combined blur (a "blur" instance carrying
    // mode/length/angle/edge) loads as Gaussian at its Radius — the leftover
    // params are simply ignored (K-137's "existing projects load as Gaussian").
    let mut legacy = instantiate("blur").unwrap();
    legacy.params.push(crate::model::EffectParam {
        id: "mode".into(),
        value: EffectValue::Choice(2), // was Radial
        extra: serde_json::Map::new(),
    });
    legacy.params.push(crate::model::EffectParam {
        id: "edge".into(),
        value: EffectValue::Choice(0),
        extra: serde_json::Map::new(),
    });
    let r = resolve_stack(
        std::slice::from_ref(&legacy),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Blur {
            radius_px: 15.0,
            edge: 1, // fixed Repeat, not the stored edge
            mix: 1.0
        }]
    );
}

#[test]
fn sharpen_simple_instantiates_and_resolves() {
    // K-138: the plain 3×3 sharpen (match_name "sharpen_simple"), separate
    // from the Unsharp mask ("sharpen").
    let e = instantiate("sharpen_simple").unwrap();
    assert_eq!(e.effect.match_name, "sharpen_simple");
    assert_eq!(e.float_at("amount", 0.0), Some(1.0));
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));
    let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::SharpenSimple {
            amount: 1.0,
            radius: 1.0,
            mix: 1.0
        }]
    );

    // The Unsharp mask keeps its own match_name and resolves as before.
    let unsharp = instantiate("sharpen").unwrap();
    assert_eq!(unsharp.effect.match_name, "sharpen");
    assert!(matches!(
        resolve_stack(&[unsharp], 0.0, 1000.0, 1.0, &MarkerContext::NONE)[..],
        [Resolved::Sharpen { .. }]
    ));
}

#[test]
fn cpu_sharpen_simple_identity_edge_overshoot_and_alpha() {
    let (w, h) = (16u32, 8u32);
    let img = step_image(w, h);

    // Amount 0 is the bit-exact identity, whatever the Mix.
    let mut a0 = img.clone();
    cpu::sharpen_simple(&mut a0, w, h, 0.0, 1.0, 1.0);
    assert_eq!(a0, img);

    // Mix 0 is the exact identity, whatever the Amount.
    let mut m0 = img.clone();
    cpu::sharpen_simple(&mut m0, w, h, 2.0, 1.0, 0.0);
    assert_eq!(m0, img);

    // A flat region is untouched (the high-pass of constant colour is zero);
    // the step edge overshoots both ways.
    let mut s = img.clone();
    cpu::sharpen_simple(&mut s, w, h, 1.0, 1.0, 1.0);
    let px = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let far = px(1, 4);
    assert!((s[far] - img[far]).abs() < 1e-5, "flat area stays put");
    let dark_side = px(w / 2 - 1, 4);
    let bright_side = px(w / 2, 4);
    assert!(s[dark_side] < img[dark_side], "dark side of edge dips");
    assert!(s[bright_side] > img[bright_side], "bright side lifts");

    // Fully transparent input stays fully transparent (no invented light).
    let mut clear = vec![0.0f32; (w * h * 4) as usize];
    cpu::sharpen_simple(&mut clear, w, h, 3.0, 1.0, 1.0);
    assert!(clear.iter().all(|v| *v == 0.0));
}

#[test]
fn cpu_directional_blur_streaks_along_the_angle() {
    // A white impulse in the middle of a transparent frame.
    let (w, h) = (17u32, 9u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let mid = at(8, 4);
    img[mid..mid + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);

    // Length 0 and mix 0 are both the exact identity.
    let mut l0 = img.clone();
    cpu::blur_directional(&mut l0, w, h, 0.0, 0.0, 1, 1.0);
    assert_eq!(l0, img);
    let mut m0 = img.clone();
    cpu::blur_directional(&mut m0, w, h, 6.0, 45.0, 1, 0.0);
    assert_eq!(m0, img);

    // Angle 0, length 5: the impulse smears along x only — energy
    // appears beside it on its own row, none above or below.
    let mut s = img.clone();
    cpu::blur_directional(&mut s, w, h, 5.0, 0.0, 1, 1.0);
    assert!(s[mid] < 1.0, "peak flattens");
    assert!(
        s[at(7, 4)] > 0.0 && s[at(9, 4)] > 0.0,
        "streak spreads in x"
    );
    assert_eq!(s[at(8, 3)], 0.0, "no bleed upward");
    assert_eq!(s[at(8, 5)], 0.0, "no bleed downward");
    // Box weights conserve energy away from edges (5 interior taps).
    let sum = |v: &[f32]| v.iter().step_by(4).sum::<f32>();
    assert!((sum(&s) - sum(&img)).abs() < 1e-4, "energy conserved");

    // Angle 90 streaks along y instead.
    let mut v = img.clone();
    cpu::blur_directional(&mut v, w, h, 5.0, 90.0, 1, 1.0);
    assert!(
        v[at(8, 3)] > 0.0 && v[at(8, 5)] > 0.0,
        "streak spreads in y"
    );
    assert!(v[at(7, 4)] < 1e-6, "x row stays clean");
}

#[test]
fn cpu_radial_blur_spins_and_zooms_from_centre() {
    // A white impulse 4px right of centre in a transparent square frame
    // (odd dimensions: pixel 8's centre is the exact frame centre, as
    // the RGB split radial test already relies on).
    let (w, h) = (17u32, 17u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let imp = at(12, 8);
    img[imp..imp + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
    let centre = [0.5f32, 0.5f32];

    // Amount 0 and mix 0 are both the exact identity, either type (the
    // same zero-tap-offset reasoning as blur_directional's length 0).
    let mut a0 = img.clone();
    cpu::blur_radial(&mut a0, w, h, centre, 0.0, true, 1, 1.0);
    assert_eq!(a0, img);
    let mut a0z = img.clone();
    cpu::blur_radial(&mut a0z, w, h, centre, 0.0, false, 1, 1.0);
    assert_eq!(a0z, img);
    let mut m0 = img.clone();
    cpu::blur_radial(&mut m0, w, h, centre, 30.0, true, 1, 0.0);
    assert_eq!(m0, img);

    // The exact centre pixel is unmoved even at a huge amount, either
    // type — d = 0 there, so every tap collapses to that pixel itself.
    let mut cs = img.clone();
    cpu::blur_radial(&mut cs, w, h, centre, 60.0, true, 1, 1.0);
    assert_eq!(cs[at(8, 8)], 0.0, "centre picks up no energy (spin)");
    let mut cz = img.clone();
    cpu::blur_radial(&mut cz, w, h, centre, 60.0, false, 1, 1.0);
    assert_eq!(cz[at(8, 8)], 0.0, "centre picks up no energy (zoom)");

    // Zoom steps along the ray through the impulse — here, exactly the
    // row — so energy spreads left/right of it on that same row. Row 8
    // is where the exact proof lives: any output pixel there has a
    // purely horizontal d (centre is also on row 8), so its zoom taps
    // never leave the row. Off-row neighbours (12,7)/(12,9) are not
    // proved zero — bilinear's one-pixel blend radius legitimately
    // bleeds a little across a row boundary near the impulse — so the
    // contrast is asserted as "far less", not "none".
    let mut z = img.clone();
    cpu::blur_radial(&mut z, w, h, centre, 20.0, false, 1, 1.0);
    assert!(z[imp] < 1.0, "peak flattens");
    assert!(
        z[at(11, 8)] > 0.0 && z[at(13, 8)] > 0.0,
        "zoom streak spreads along the ray"
    );
    assert!(
        z[at(12, 7)] < z[at(11, 8)] && z[at(12, 9)] < z[at(11, 8)],
        "zoom bleeds far less off the ray than along it"
    );

    // Spin steps along the perpendicular instead — energy spreads
    // above/below the impulse. The exact proof mirrors the zoom one:
    // row 8's own points have a purely *vertical* spin step there, so
    // they never reach column 12 — no bleed along the ray at all.
    let mut s = img.clone();
    cpu::blur_radial(&mut s, w, h, centre, 20.0, true, 1, 1.0);
    assert!(s[imp] < 1.0, "peak flattens");
    assert!(
        s[at(12, 7)] > 0.0 && s[at(12, 9)] > 0.0,
        "spin streak spreads tangentially"
    );
    assert_eq!(s[at(11, 8)], 0.0, "spin: no bleed along the ray");
    assert_eq!(s[at(13, 8)], 0.0, "spin: no bleed along the ray");
}

#[test]
fn transform_instantiates_and_resolves_with_the_preview_factor() {
    let e = instantiate("transform").unwrap();
    assert_eq!(e.float_at("anchor_x", 0.0), Some(0.0));
    assert_eq!(e.float_at("position_x", 0.0), Some(0.0));
    assert_eq!(e.float_at("scale_x", 0.0), Some(100.0));
    assert_eq!(e.float_at("rotation", 0.0), Some(0.0));
    assert_eq!(e.float_at("opacity", 0.0), Some(100.0));
    // Defaults resolve to the exact identity op.
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Transform {
            anchor: [0.0; 2],
            position: [0.0; 2],
            scale: [1.0; 2],
            rotation_deg: 0.0,
            opacity: 1.0,
            mix: 1.0
        }]
    );

    // px@comp parameters scale by the §2.3 preview factor; percentages
    // and degrees do not.
    let mut e = e;
    for p in &mut e.params {
        match p.id.as_str() {
            "anchor_x" => p.value = EffectValue::Float(Property::fixed(40.0)),
            "position_x" => p.value = EffectValue::Float(Property::fixed(100.0)),
            "scale_x" => p.value = EffectValue::Float(Property::fixed(200.0)),
            "rotation" => p.value = EffectValue::Float(Property::fixed(90.0)),
            _ => {}
        }
    }
    let r = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        500.0,
        0.5,
        &MarkerContext::NONE,
    );
    assert_eq!(
        r,
        vec![Resolved::Transform {
            anchor: [20.0, 0.0],
            position: [50.0, 0.0],
            scale: [2.0, 1.0],
            rotation_deg: 90.0,
            opacity: 1.0,
            mix: 1.0
        }]
    );
}

#[test]
fn glow_instantiates_resolves_and_pins_the_one_sided_threshold() {
    // The K-090 poster child: the Threshold hard range is clamped at
    // zero below and unbounded above — HDR values glow harder.
    let s = schema("glow").unwrap();
    let threshold = s.params.iter().find(|p| p.id == "threshold").unwrap();
    assert!(matches!(
        threshold.kind,
        ParamKind::Float {
            hard: (Some(0.0), None),
            ..
        }
    ));

    let e = instantiate("glow").unwrap();
    // K-135/FX-16: default threshold drops to 0.8, and Radius is now px@comp.
    assert_eq!(e.float_at("threshold", 0.0), Some(0.8));
    assert_eq!(e.float_at("knee", 0.0), Some(0.5));
    assert_eq!(e.float_at("radius", 0.0), Some(24.0));
    assert_eq!(e.float_at("intensity", 0.0), Some(1.0));
    assert_eq!(e.colour_at("tint", 0.0), Some([1.0; 4]));
    // Radius is px@comp scaled by the preview factor: 24 px × a half-res
    // (0.5) factor = 12 raster px; diag_px no longer feeds Radius.
    let r = resolve_stack(&[e], 0.0, 1000.0, 0.5, &MarkerContext::NONE);
    assert_eq!(
        r,
        vec![Resolved::Glow {
            radius_px: 12.0,
            threshold: 0.8,
            knee: 0.5,
            intensity: 1.0,
            tint: [1.0; 4],
            mix: 1.0
        }]
    );
    // The Radius schema is now open above (px@comp, K-135).
    let s = schema("glow").unwrap();
    let radius = s.params.iter().find(|p| p.id == "radius").unwrap();
    assert!(matches!(
        radius.kind,
        ParamKind::Float {
            slider: (0.0, 200.0),
            hard: (Some(0.0), None),
            ..
        }
    ));
}

#[test]
fn glow_bright_gates_eases_and_passes_hdr() {
    // Below the threshold: nothing, knee or not.
    assert_eq!(glow_bright(0.5, 1.0, 0.0), 0.0);
    assert_eq!(glow_bright(0.5, 1.0, 0.5), 0.0);
    assert_eq!(glow_bright(1.0, 1.0, 0.5), 0.0);
    // Knee 0 is the hard subtract.
    assert_eq!(glow_bright(3.0, 1.0, 0.0), 2.0);
    // Inside the knee the onset is eased below the hard hinge.
    let eased = glow_bright(1.25, 1.0, 0.5);
    assert!(eased > 0.0 && eased < 0.25, "eased onset: {eased}");
    // Beyond threshold + knee the smoothstep saturates: hard subtract.
    assert_eq!(glow_bright(3.0, 1.0, 0.5), 2.0);
    // Monotone across the knee (no dips as the smoothstep engages).
    let mut prev = 0.0;
    for i in 0..=40 {
        let x = 0.4 + i as f32 * 0.05;
        let b = glow_bright(x, 1.0, 0.5);
        assert!(b >= prev, "monotone at x={x}");
        prev = b;
    }
}

#[test]
fn cpu_glow_blooms_spreads_alpha_and_keeps_neutral_exact() {
    // An HDR spike on an opaque dark frame, plus a transparent border.
    let (w, h) = (17u32, 9u32);
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 2..w - 2 {
            let i = at(x, y);
            img[i..i + 4].copy_from_slice(&[0.1, 0.1, 0.1, 1.0]);
        }
    }
    let mid = at(8, 4);
    img[mid..mid + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);

    // Intensity 0 is the bit-exact identity (the neutral pin).
    let mut n = img.clone();
    cpu::glow(&mut n, w, h, 4.0, 1.0, 0.5, 0.0, [1.0; 4], 1.0);
    assert_eq!(n, img);

    // Mix 0 is the exact identity whatever the parameters.
    let mut m0 = img.clone();
    cpu::glow(&mut m0, w, h, 4.0, 0.2, 0.1, 2.0, [1.0; 4], 0.0);
    assert_eq!(m0, img);

    // A frame entirely below the threshold gains nothing: the halo is
    // zero everywhere and the add is exact.
    let dim = {
        let mut d = img.clone();
        d[mid..mid + 4].copy_from_slice(&[0.1, 0.1, 0.1, 1.0]);
        d
    };
    let mut quiet = dim.clone();
    cpu::glow(&mut quiet, w, h, 4.0, 1.0, 0.5, 1.0, [1.0; 4], 1.0);
    assert_eq!(quiet, dim);

    // The spike blooms: neighbours gain light, the spike itself gains
    // its own halo back (additive, §2.1: nothing clips).
    let mut g = img.clone();
    cpu::glow(&mut g, w, h, 3.0, 1.0, 0.5, 1.0, [1.0; 4], 1.0);
    assert!(g[at(10, 4)] > img[at(10, 4)], "neighbour catches the halo");
    assert!(g[mid] > img[mid], "the spike gains its own bloom");

    // The halo carries alpha over transparency: with a threshold low
    // enough that opaque coverage passes it, the transparent border
    // next to the footprint gains coverage — glow reads as light there.
    let mut a = img.clone();
    cpu::glow(&mut a, w, h, 3.0, 0.05, 0.0, 1.0, [1.0; 4], 1.0);
    assert!(a[at(1, 4) + 3] > 0.0, "coverage bloomed past the edge");
    assert!(a[at(8, 4) + 3] <= 1.0, "alpha saturates at full coverage");

    // Tint colours the halo, not the underlying image: with a red tint,
    // the transparent border gains red light only.
    let mut t = img.clone();
    cpu::glow(&mut t, w, h, 3.0, 0.05, 0.0, 1.0, [1.0, 0.0, 0.0, 1.0], 1.0);
    assert!(t[at(1, 4)] > 0.0, "red halo over the border");
    assert_eq!(t[at(1, 4) + 1], 0.0, "no green in a red-tinted halo");
}

#[test]
fn shake_noise_is_deterministic_seeded_and_hop_free() {
    // Same inputs → same outputs, exactly (§2.4 determinism).
    for i in 0..50 {
        let x = i as f64 * 0.173;
        assert_eq!(shake_noise(7, 0, x), shake_noise(7, 0, x));
    }
    // Different seeds → different sequences; different channels too.
    assert_ne!(shake_noise(1, 0, 0.37), shake_noise(2, 0, 0.37));
    assert_ne!(shake_noise(1, 0, 0.37), shake_noise(1, 1, 0.37));
    // Bounded to [−1, 1] and actually moving.
    let mut spread = (f64::MAX, f64::MIN);
    for i in 0..500 {
        let v = shake_noise(11, 2, i as f64 * 0.31);
        assert!(v.abs() <= 1.0, "bounded at x={i}: {v}");
        spread = (spread.0.min(v), spread.1.max(v));
    }
    assert!(spread.1 - spread.0 > 0.5, "the wobble wanders: {spread:?}");
    // Hop-free: tiny steps in time give tiny steps in value, across
    // lattice boundaries included (the smoothstep is C¹ there).
    for i in 0..400 {
        let x = i as f64 * 0.01;
        let dv = (shake_noise(3, 1, x + 1e-4) - shake_noise(3, 1, x)).abs();
        assert!(dv < 1e-2, "no hop at x={x}: step {dv}");
    }
}

#[test]
fn shake_instantiates_with_a_per_instance_seed_and_resolves() {
    let e = instantiate("shake").unwrap();
    assert_eq!(e.float_at("amplitude", 0.0), Some(1.5));
    assert_eq!(e.float_at("frequency", 0.0), Some(8.0));
    assert_eq!(e.float_at("rotation", 0.0), Some(1.0));
    // The per-axis twirl group's defaults (multipliers of 1, z pump 0) and
    // the Edges control (default Repeat = code 1) replace the old Zoom
    // pump / Auto-scale pair.
    assert_eq!(e.float_at("x_amp", 0.0), Some(1.0));
    assert_eq!(e.float_at("y_freq", 0.0), Some(1.0));
    assert_eq!(e.float_at("z_amp", 0.0), Some(0.0));
    assert!(matches!(e.param("edge"), Some(EffectValue::Choice(2))));
    assert!(e.param("zoom_pump").is_none());
    assert!(e.param("auto_scale").is_none());
    assert!(matches!(e.param("seed"), Some(EffectValue::Seed(_))));
    // The shake's own motion blur (T18) ships off, with a 0.5 shutter default.
    assert_eq!(e.bool_of("motion_blur"), Some(false));
    assert_eq!(e.float_at("mb_amount", 0.0), Some(0.5));
    // The schema declares two twirl groups over contiguous param runs.
    let schema = schema("shake").unwrap();
    assert_eq!(schema.groups.len(), 2);
    assert_eq!(schema.groups[0].label, "Per-axis wobble");
    assert!(schema.groups[0].collapsed);
    assert_eq!(
        schema.groups[0].params,
        &["x_amp", "x_freq", "y_amp", "y_freq", "z_amp", "z_freq"]
    );
    assert_eq!(schema.groups[1].label, "Motion blur");
    assert!(schema.groups[1].collapsed);
    assert_eq!(schema.groups[1].params, &["motion_blur", "mb_amount"]);

    // Resolving is deterministic: the same instance at the same time
    // yields the identical wobble, twice.
    let a = resolve_stack(
        std::slice::from_ref(&e),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let b = resolve_stack(
        std::slice::from_ref(&e),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(a, b);
    let Resolved::Shake {
        offset_px,
        zoom,
        edge,
        mix,
        ..
    } = a[0]
    else {
        panic!("expected a Shake");
    };
    // 1.5% of a 1000px diagonal = 15px ceiling; the wobble stays
    // within it, z amount 0 leaves zoom at exactly 1, and the default
    // Edges control is Mirror (code 2 — owner, 2026-07-19).
    assert!(offset_px[0].abs() <= 15.0 && offset_px[1].abs() <= 15.0);
    assert_eq!(zoom, 1.0);
    assert_eq!(edge, 2);
    assert_eq!(mix, 1.0);

    // Different frames wobble differently; different seeds too.
    let later = resolve_stack(
        std::slice::from_ref(&e),
        0.9,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_ne!(a, later, "the wobble moves between frames");
    let mut reseeded = e.clone();
    for p in &mut reseeded.params {
        if p.id == "seed" {
            let old = match p.value {
                EffectValue::Seed(s) => s,
                _ => 0,
            };
            p.value = EffectValue::Seed(old.wrapping_add(1));
        }
    }
    let other = resolve_stack(
        std::slice::from_ref(&reseeded),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_ne!(a, other, "a different seed wobbles differently");
}

#[test]
fn cpu_shake_is_identity_at_zero_and_wobbles_through_the_affine() {
    let (w, h) = (17u32, 9u32);
    let img = transform_card(w, h);

    // A neutral shake (zero wobble) is the bit-exact identity: the affine
    // is the identity, whatever the Edges control.
    let neutral = Resolved::Shake {
        offset_px: [0.0, 0.0],
        rotation_deg: 0.0,
        zoom: 1.0,
        edge: 1,
        mix: 1.0,
        mb: None,
    };
    let mut n = img.clone();
    cpu::apply(&mut n, w, h, &neutral);
    assert_eq!(n, img);

    // A pure offset matches the Transform reference fed the same shared
    // affine and the same edge policy — the oracle path is one path.
    let shaken = Resolved::Shake {
        offset_px: [2.0, -1.0],
        rotation_deg: 0.0,
        zoom: 1.0,
        edge: 0,
        mix: 1.0,
        mb: None,
    };
    let mut s = img.clone();
    cpu::apply(&mut s, w, h, &shaken);
    let (anchor, position, scale, rot) = shake_affine(w, h, [2.0, -1.0], 0.0, 1.0);
    let mut t = img.clone();
    cpu::transform(&mut t, w, h, anchor, position, scale, rot, 0, 1.0, 1.0);
    assert_eq!(s, t);
    assert_ne!(s, img, "the wobble actually moves pixels");

    // The Edges control governs the revealed border (P3, K-145). A big
    // offset drags an edge into view: Transparent leaves a fully clear
    // corner; Repeat and Mirror hold coverage there instead.
    let corner_alpha = |v: &[f32]| {
        let at = |x: u32, y: u32| ((y * w + x) * 4 + 3) as usize;
        [
            v[at(0, 0)],
            v[at(w - 1, 0)],
            v[at(0, h - 1)],
            v[at(w - 1, h - 1)],
        ]
    };
    let shake_with = |edge: u32| {
        let mut c = img.clone();
        cpu::apply(
            &mut c,
            w,
            h,
            &Resolved::Shake {
                offset_px: [6.0, 3.0],
                rotation_deg: 0.0,
                zoom: 1.0,
                edge,
                mix: 1.0,
                mb: None,
            },
        );
        c
    };
    let transparent = shake_with(0);
    assert!(
        corner_alpha(&transparent).contains(&0.0),
        "Transparent reveals a clear corner: {:?}",
        corner_alpha(&transparent)
    );
    for edge in [1u32, 2] {
        let held = shake_with(edge);
        assert!(
            corner_alpha(&held).iter().all(|a| *a > 0.0),
            "edge {edge} holds coverage at every corner: {:?}",
            corner_alpha(&held)
        );
    }
}

/// A shake instance with its motion blur enabled at `amount`.
fn shake_with_mb(amount: f64) -> crate::model::EffectInstance {
    let mut e = instantiate("shake").unwrap();
    for p in &mut e.params {
        match p.id.as_str() {
            "motion_blur" => p.value = EffectValue::Bool(true),
            "mb_amount" => p.value = EffectValue::Float(crate::anim::Property::fixed(amount)),
            _ => {}
        }
    }
    e
}

#[test]
fn resolve_shake_motion_blur_samples_the_shutter_and_centres_on_the_frame() {
    // Off (the default) resolves to a single wobble — no sub-frame set.
    let off = instantiate("shake").unwrap();
    let r = resolve_stack(
        std::slice::from_ref(&off),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::Shake { mb, .. } = r[0] else {
        panic!("expected a Shake");
    };
    assert!(mb.is_none(), "motion blur off carries no sub-frames");

    // On: the sub-frame set is present, its centre sample is the frame-time
    // wobble exactly, and the samples actually differ across the shutter.
    let on = shake_with_mb(0.5);
    let r = resolve_stack(
        std::slice::from_ref(&on),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::Shake {
        offset_px,
        rotation_deg,
        zoom,
        mb,
        ..
    } = r[0]
    else {
        panic!("expected a Shake");
    };
    let samples = mb.expect("motion blur on carries sub-frames");
    assert_eq!(samples.len(), SHAKE_MB_SAMPLES);
    let centre = samples[SHAKE_MB_SAMPLES / 2];
    assert_eq!(centre.offset_px, offset_px, "centre sample is the frame");
    assert_eq!(centre.rotation_deg, rotation_deg);
    assert_eq!(centre.zoom, zoom);
    assert_ne!(
        samples[0].offset_px,
        samples[SHAKE_MB_SAMPLES - 1].offset_px,
        "the wobble moves across the shutter"
    );

    // Determinism: same instance, same time, identical sub-frames twice.
    let r2 = resolve_stack(
        std::slice::from_ref(&on),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(r, r2);

    // A zero shutter is treated as no smear (the bit-exact single resample).
    let zero = shake_with_mb(0.0);
    let r = resolve_stack(
        std::slice::from_ref(&zero),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::Shake { mb, .. } = r[0] else {
        panic!("expected a Shake");
    };
    assert!(mb.is_none(), "a zero shutter carries no sub-frames");
}

#[test]
fn cpu_shake_motion_blur_off_is_the_plain_shake_and_on_smears() {
    let (w, h) = (24u32, 16u32);
    let img = transform_card(w, h);

    // A shake carrying a wobble, resolved without motion blur.
    let base = shake_with_mb(0.0); // amount 0 ⇒ mb None ⇒ the plain shake
    let plain = resolve_stack(
        std::slice::from_ref(&base),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::Shake { mb: None, .. } = plain[0] else {
        panic!("expected a plain Shake");
    };
    let mut a = img.clone();
    cpu::apply(&mut a, w, h, &plain[0]);

    // The same shake with motion blur on smears: the averaged result differs
    // from the plain single resample.
    let blurred = resolve_stack(
        std::slice::from_ref(&shake_with_mb(0.8)),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert!(
        matches!(blurred[0], Resolved::Shake { mb: Some(_), .. }),
        "motion blur on carries sub-frames"
    );
    let mut b = img.clone();
    cpu::apply(&mut b, w, h, &blurred[0]);
    assert_ne!(a, b, "motion blur smears the shake");

    // A degenerate sub-frame set — every sample equal to one wobble — averages
    // back to that single resample (to within f32 rounding of the sum ÷ count),
    // pinning the averaging maths against the plain transform reference.
    let one = ShakeSample {
        offset_px: [3.0, -2.0],
        rotation_deg: 5.0,
        zoom: 1.02,
    };
    let flat = Resolved::Shake {
        offset_px: one.offset_px,
        rotation_deg: one.rotation_deg,
        zoom: one.zoom,
        edge: 1,
        mix: 1.0,
        mb: Some([one; SHAKE_MB_SAMPLES]),
    };
    let mut avg = img.clone();
    cpu::apply(&mut avg, w, h, &flat);
    let single = Resolved::Shake {
        offset_px: one.offset_px,
        rotation_deg: one.rotation_deg,
        zoom: one.zoom,
        edge: 1,
        mix: 1.0,
        mb: None,
    };
    let mut one_shot = img.clone();
    cpu::apply(&mut one_shot, w, h, &single);
    let worst = avg
        .iter()
        .zip(&one_shot)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        worst < 1e-4,
        "averaging identical sub-frames is the single resample (worst {worst})"
    );
}

#[test]
fn edges_mode_codes_round_trip() {
    // The enum only names the wire codes the resolved ops and WGSL read.
    for (mode, code) in [
        (EdgesMode::Transparent, 0u32),
        (EdgesMode::Repeat, 1),
        (EdgesMode::Mirror, 2),
    ] {
        assert_eq!(mode.code(), code);
        assert_eq!(EdgesMode::from_code(code), Some(mode));
    }
    assert_eq!(EdgesMode::from_code(3), None);
    assert_eq!(EdgesMode::OPTIONS, &["Transparent", "Repeat", "Mirror"]);
    // The shared blur-family const is the enum's option list.
    assert_eq!(EDGE_OPTIONS, EdgesMode::OPTIONS);
}

#[test]
fn shake_migrates_old_zoom_pump_and_auto_scale_params() {
    // A project saved before FX-11 carries `zoom_pump` and `auto_scale`
    // instead of `z_amp` and `edge`. Resolve reads the old ids as
    // fallbacks so the look migrates sensibly (K-146).
    let mut old = instantiate("shake").unwrap();
    // Rebuild the pre-FX-11 param set by id.
    old.params.retain(|p| {
        matches!(
            p.id.as_str(),
            "amplitude" | "frequency" | "rotation" | "seed" | "mix"
        )
    });
    old.params.push(crate::model::EffectParam {
        id: "zoom_pump".into(),
        value: EffectValue::Float(crate::anim::Property::fixed(10.0)),
        extra: Default::default(),
    });
    old.params.push(crate::model::EffectParam {
        id: "auto_scale".into(),
        value: EffectValue::Bool(false),
        extra: Default::default(),
    });

    let resolved = resolve_stack(
        std::slice::from_ref(&old),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::Shake { zoom, edge, .. } = resolved[0] else {
        panic!("expected a Shake");
    };
    // The old 10% Zoom pump becomes the z (depth) shake, so zoom moves off
    // 1; Auto-scale off migrates to the Transparent edge (code 0).
    assert_ne!(zoom, 1.0, "the old Zoom pump migrated to the z shake");
    assert_eq!(edge, 0, "Auto-scale off migrated to Transparent");

    // Auto-scale on (the old default) migrates to Repeat (code 1).
    for p in &mut old.params {
        if p.id == "auto_scale" {
            p.value = EffectValue::Bool(true);
        }
    }
    let on = resolve_stack(
        std::slice::from_ref(&old),
        0.4,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let Resolved::Shake { edge, .. } = on[0] else {
        panic!("expected a Shake");
    };
    assert_eq!(edge, 1, "Auto-scale on migrated to Repeat");
}

#[test]
fn transform_inverse_is_exact_at_identity_and_none_at_zero_scale() {
    let (m, o) = transform_inverse([0.0; 2], [0.0; 2], [1.0; 2], 0.0).unwrap();
    assert_eq!(m, [1.0, 0.0, -0.0, 1.0]);
    assert_eq!(o, [0.0, 0.0]);
    assert!(transform_inverse([0.0; 2], [0.0; 2], [0.0, 1.0], 0.0).is_none());
    assert!(transform_inverse([0.0; 2], [0.0; 2], [1.0, 0.0], 0.0).is_none());
}

/// A varied premultiplied test card for the transform: gradient, an HDR
/// spike, a half-alpha region and an opaque border pixel.
fn transform_card(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = (x + y) as f32 / (w + h) as f32;
            let a = if y < h / 2 { 1.0 } else { 0.5 };
            img[i] = g * a;
            img[i + 1] = (1.0 - g) * a;
            img[i + 2] = 0.25 * a;
            img[i + 3] = a;
        }
    }
    let spike = ((3 * w + 4) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);
    img
}

#[test]
fn cpu_transform_identity_is_bit_exact() {
    let (w, h) = (13u32, 9u32);
    let img = transform_card(w, h);
    // Identity parameters: the docs/08 §3.5 bit-exact passthrough pin.
    let mut id = img.clone();
    cpu::transform(
        &mut id, w, h, [0.0; 2], [0.0; 2], [1.0; 2], 0.0, 0, 1.0, 1.0,
    );
    assert_eq!(id, img);
    // Mix 0 is the exact identity whatever the parameters.
    let mut m0 = img.clone();
    cpu::transform(
        &mut m0,
        w,
        h,
        [3.0; 2],
        [9.0, 1.0],
        [2.0, 0.5],
        33.0,
        0,
        0.4,
        0.0,
    );
    assert_eq!(m0, img);
}

#[test]
fn cpu_transform_moves_scales_rotates_and_fades() {
    // A white impulse on a transparent frame.
    let (w, h) = (17u32, 9u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let mid = at(8, 4);
    img[mid..mid + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);

    // Position +2 in x (anchor 0): the impulse lands two pixels right,
    // exactly (integer offsets keep bilinear taps on pixel centres).
    let mut t = img.clone();
    cpu::transform(
        &mut t,
        w,
        h,
        [0.0; 2],
        [2.0, 0.0],
        [1.0; 2],
        0.0,
        0,
        1.0,
        1.0,
    );
    assert_eq!(t[at(10, 4)], 1.0, "impulse moved +2x");
    assert_eq!(t[mid], 0.0, "and left its old home");

    // The area revealed beyond the source edge is transparent, not a
    // smeared border: shifting +2 leaves columns 0-1 fully empty.
    for y in 0..h {
        for x in 0..2 {
            assert_eq!(t[at(x, y) + 3], 0.0, "({x},{y}) revealed as clear");
        }
    }

    // Rotation 90° about the frame centre: y-down raster, so the pixel
    // two to the right of centre lands two below it (clockwise).
    let centre = [8.5, 4.5];
    let mut r = img.clone();
    img[at(10, 4)..at(10, 4) + 4].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
    r.copy_from_slice(&img);
    cpu::transform(&mut r, w, h, centre, centre, [1.0; 2], 90.0, 0, 1.0, 1.0);
    assert_eq!(r[mid], 1.0, "the centre pixel stays put");
    assert!(r[at(8, 6) + 1] > 0.999, "+2x lands at +2y");

    // Scale 0 is degenerate: the image collapses to nothing and renders
    // fully transparent — never a division fault (docs/14).
    let mut z = img.clone();
    cpu::transform(&mut z, w, h, centre, centre, [0.0, 0.0], 0.0, 0, 1.0, 1.0);
    assert!(z.iter().all(|v| *v == 0.0), "zero scale collapses to clear");

    // Opacity halves all four channels (premultiplied).
    let mut o = img.clone();
    cpu::transform(&mut o, w, h, [0.0; 2], [0.0; 2], [1.0; 2], 0.0, 0, 0.5, 1.0);
    for c in 0..4 {
        assert_eq!(o[mid + c], 0.5, "channel {c} at half");
    }
}

/// A minimal comp + layer pair for marker-context tests: a comp at the
/// given frame rate carrying `markers`, and an adjustment layer whose
/// start offset is `offset_s` seconds.
fn marker_rig(
    fps: (u32, u32),
    markers: Vec<crate::markers::Marker>,
    offset_s: (i64, i64),
) -> (Composition, Layer) {
    use crate::model::{LayerKind, LinearColour, Switches, TransformGroup};
    use crate::time::{CompTime, Duration, FrameRate, Rational};
    let secs = |n, d| CompTime(Rational::new(n, d).unwrap());
    let comp = Composition {
        id: uuid::Uuid::now_v7(),
        name: "c".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(fps.0, fps.1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers: Vec::new(),
        markers,
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let layer = Layer {
        id: uuid::Uuid::now_v7(),
        name: "l".into(),
        kind: LayerKind::Adjustment,
        in_point: secs(0, 1),
        out_point: secs(10, 1),
        start_offset: secs(offset_s.0, offset_s.1),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: crate::anim::Property::zero(),
        blend: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    (comp, layer)
}

#[test]
fn marker_context_builds_layer_local_ordered_beats() {
    use crate::markers::{Marker, MarkerKind};
    use crate::time::{CompTime, Rational};
    let rat = |n, d| Rational::new(n, d).unwrap();
    // Beats out of order, plus a user and a chapter marker to ignore.
    let user = Marker::user(uuid::Uuid::now_v7(), rat(1, 2));
    let chapter = Marker {
        kind: MarkerKind::Chapter,
        time: CompTime(rat(3, 1)),
        ..Marker::user(uuid::Uuid::now_v7(), rat(3, 1))
    };
    let late = Marker::beat(uuid::Uuid::now_v7(), rat(2, 1), 0.9);
    let early = Marker::beat(uuid::Uuid::now_v7(), rat(1, 1), 0.5);
    let (comp, layer) = marker_rig((30, 1), vec![user, late, chapter, early], (1, 4));
    let ctx = MarkerContext::for_layer(&comp, &layer);
    // Beat kind only, layer-local (comp time − start offset), sorted.
    assert_eq!(ctx.beats, vec![0.75, 1.75]);
    assert_eq!(ctx.fps, 30.0);
    // The local translation matches the resolver's own lt subtraction
    // exactly: a beat at comp second 1 and a frame evaluated there land
    // on the identical f64.
    let lt = 1.0 - layer.start_offset.0.to_f64();
    assert_eq!(ctx.beats[0], lt);
    // The obvious no-marker default (§1.4 graceful fallback).
    assert_eq!(MarkerContext::NONE.beats, Vec::<f64>::new());
    assert_eq!(MarkerContext::NONE.fps, 0.0);
    assert_eq!(MarkerContext::default(), MarkerContext::NONE);
}

#[test]
fn marker_context_window_and_nearest() {
    let ctx = MarkerContext {
        beats: vec![1.0, 2.0, 4.0],
        fps: 30.0,
    };
    // The §1.4 temporal-window view: inclusive both ends.
    assert_eq!(ctx.window(1.0, 2.0), &[1.0, 2.0]);
    assert_eq!(ctx.window(1.5, 3.9), &[2.0]);
    assert_eq!(ctx.window(2.5, 3.5), &[] as &[f64]);
    assert_eq!(
        ctx.window(3.0, 1.0),
        &[] as &[f64],
        "inverted span is empty"
    );
    // The nearest-either-side pair: "before" is at/before the frame.
    assert_eq!(ctx.nearest(2.0), (Some(2.0), Some(4.0)));
    assert_eq!(ctx.nearest(2.5), (Some(2.0), Some(4.0)));
    assert_eq!(ctx.nearest(0.5), (None, Some(1.0)));
    assert_eq!(ctx.nearest(9.0), (Some(4.0), None));
    assert_eq!(MarkerContext::NONE.nearest(1.0), (None, None));
}

/// A context whose beats and rate use exactly representable values, so
/// envelope boundary assertions are exact rather than tolerance games.
fn beat_ctx(beats: &[f64], fps: f64) -> MarkerContext {
    MarkerContext {
        beats: beats.to_vec(),
        fps,
    }
}

#[test]
fn flash_beat_envelope_hard_and_fade_shapes() {
    let ctx = beat_ctx(&[1.0], 4.0);
    // On the beat: full strength, whichever the shape.
    assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, false, 1, 0.0), 1.0);
    assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, true, 1, 0.0), 1.0);
    // One frame in (0.25 s at 4 fps): Hard still full, Fade at the
    // midpoint of a 2-frame duration.
    assert_eq!(flash_beat_envelope(&ctx, 1.25, 2.0, false, 1, 0.0), 1.0);
    assert_eq!(flash_beat_envelope(&ctx, 1.25, 2.0, true, 1, 0.0), 0.5);
    // The span is [0, duration): at exactly two frames both shapes are
    // spent, and well past the duration they stay zero.
    assert_eq!(flash_beat_envelope(&ctx, 1.5, 2.0, false, 1, 0.0), 0.0);
    assert_eq!(flash_beat_envelope(&ctx, 1.5, 2.0, true, 1, 0.0), 0.0);
    assert_eq!(flash_beat_envelope(&ctx, 3.0, 2.0, false, 1, 0.0), 0.0);
    // Before the first trigger there is nothing to decay from.
    assert_eq!(flash_beat_envelope(&ctx, 0.75, 2.0, false, 1, 0.0), 0.0);
    // A fresh beat wins over a spent one (nearest at/before rule).
    let two = beat_ctx(&[1.0, 2.0], 4.0);
    assert_eq!(flash_beat_envelope(&two, 2.0, 2.0, true, 1, 0.0), 1.0);
}

#[test]
fn flash_beat_envelope_phase_shifts_the_triggers() {
    let ctx = beat_ctx(&[1.0], 4.0);
    // Phase +2 frames at 4 fps = +0.5 s: the beat itself no longer
    // fires; the shifted moment does, at full strength.
    assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, false, 1, 2.0), 0.0);
    assert_eq!(flash_beat_envelope(&ctx, 1.5, 2.0, false, 1, 2.0), 1.0);
    // Negative phase leads the beat.
    assert_eq!(flash_beat_envelope(&ctx, 0.5, 2.0, false, 1, -2.0), 1.0);
    assert_eq!(
        flash_beat_envelope(&ctx, 0.75, 2.0, true, 1, -2.0),
        0.5,
        "fade measures from the shifted trigger"
    );
}

#[test]
fn flash_beat_envelope_strobe_skips_to_every_nth() {
    // Beats each second; every 2nd fires indices 0 and 2 (the comp's
    // first beat is index 0).
    let ctx = beat_ctx(&[1.0, 2.0, 3.0, 4.0], 4.0);
    assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, false, 2, 0.0), 1.0);
    assert_eq!(
        flash_beat_envelope(&ctx, 2.0, 2.0, false, 2, 0.0),
        0.0,
        "the skipped beat does not fire"
    );
    assert_eq!(flash_beat_envelope(&ctx, 3.0, 2.0, false, 2, 0.0), 1.0);
    // Nth 1 fires them all; a degenerate 0 clamps to 1.
    assert_eq!(flash_beat_envelope(&ctx, 2.0, 2.0, false, 1, 0.0), 1.0);
    assert_eq!(flash_beat_envelope(&ctx, 2.0, 2.0, false, 0, 0.0), 1.0);
}

#[test]
fn flash_beat_envelope_falls_back_gracefully() {
    // No markers, the NONE context, a zero duration and a zero frame
    // rate all yield exactly nothing (§1.4: MUST work with no markers).
    assert_eq!(
        flash_beat_envelope(&MarkerContext::NONE, 1.0, 2.0, false, 1, 0.0),
        0.0
    );
    assert_eq!(
        flash_beat_envelope(&beat_ctx(&[], 30.0), 1.0, 2.0, true, 1, 0.0),
        0.0
    );
    let ctx = beat_ctx(&[1.0], 4.0);
    assert_eq!(flash_beat_envelope(&ctx, 1.0, 0.0, false, 1, 0.0), 0.0);
    assert_eq!(
        flash_beat_envelope(&beat_ctx(&[1.0], 0.0), 1.0, 2.0, false, 1, 0.0),
        0.0
    );
}

#[test]
fn flash_mode_resolves_manual_trigger_strobe_and_legacy() {
    let ctx = beat_ctx(&[1.0, 2.0, 3.0], 4.0);
    // A fresh instance defaults to Manual and resolves exactly as the
    // pre-mode flash did, markers or none.
    let mut e = instantiate("flash").unwrap();
    assert!(matches!(e.param("mode"), Some(EffectValue::Choice(0))));
    assert_eq!(e.float_at("duration", 0.0), Some(2.0));
    assert!(matches!(e.param("shape"), Some(EffectValue::Choice(0))));
    assert_eq!(e.float_at("every_nth", 0.0), Some(1.0));
    assert_eq!(e.float_at("phase", 0.0), Some(0.0));
    let dark = Resolved::Flash {
        strength: 0.0,
        colour: [1.0; 4],
        mix: 1.0,
    };
    let r = resolve_stack(std::slice::from_ref(&e), 1.0, 1000.0, 1.0, &ctx);
    assert_eq!(r, vec![dark], "Manual ignores markers entirely");

    // Trigger mode lights on the beat and is spent past Duration.
    for p in &mut e.params {
        if p.id == "mode" {
            p.value = EffectValue::Choice(1);
        }
    }
    let lit = Resolved::Flash {
        strength: 1.0,
        colour: [1.0; 4],
        mix: 1.0,
    };
    let r = resolve_stack(std::slice::from_ref(&e), 1.0, 1000.0, 1.0, &ctx);
    assert_eq!(r, vec![lit]);
    let r = resolve_stack(std::slice::from_ref(&e), 1.75, 1000.0, 1.0, &ctx);
    assert_eq!(r, vec![dark], "3 frames past a 2-frame flash");
    // And with no markers at all it resolves dark — never an error
    // (§1.4 graceful fallback).
    let r = resolve_stack(
        std::slice::from_ref(&e),
        1.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(r, vec![dark]);

    // Strobe every 2nd beat: beat index 1 (2 s) does not fire, index 2
    // (3 s) does.
    for p in &mut e.params {
        match p.id.as_str() {
            "mode" => p.value = EffectValue::Choice(2),
            "every_nth" => p.value = EffectValue::Float(Property::fixed(2.0)),
            _ => {}
        }
    }
    let r = resolve_stack(std::slice::from_ref(&e), 2.0, 1000.0, 1.0, &ctx);
    assert_eq!(r, vec![dark]);
    let r = resolve_stack(std::slice::from_ref(&e), 3.0, 1000.0, 1.0, &ctx);
    assert_eq!(r, vec![lit]);

    // A legacy instance (saved before the marker modes existed) has no
    // mode parameter and still resolves Manual: a static Trigger of
    // 0.4 holds a 0.4 flash whatever the markers say.
    let mut legacy = instantiate("flash").unwrap();
    legacy.params.retain(|p| {
        !matches!(
            p.id.as_str(),
            "mode" | "duration" | "shape" | "every_nth" | "phase"
        )
    });
    for p in &mut legacy.params {
        if p.id == "trigger" {
            p.value = EffectValue::Float(Property::fixed(0.4));
        }
    }
    let r = resolve_stack(std::slice::from_ref(&legacy), 1.0, 1000.0, 1.0, &ctx);
    assert_eq!(
        r,
        vec![Resolved::Flash {
            strength: 0.4,
            colour: [1.0; 4],
            mix: 1.0
        }]
    );
}

#[test]
fn marker_window_reports_what_the_envelope_reads() {
    let ctx = beat_ctx(&[1.0, 2.0, 3.0], 4.0);
    // Manual mode — and any effect without marker input — has no
    // window, which is what keeps its frame keys time-free.
    let mut e = instantiate("flash").unwrap();
    assert_eq!(marker_window(&e, 1.5, &ctx), None);
    let blur = instantiate("blur").unwrap();
    assert_eq!(marker_window(&blur, 1.5, &ctx), None);

    // Trigger mode: the nearest trigger either side of the frame.
    for p in &mut e.params {
        if p.id == "mode" {
            p.value = EffectValue::Choice(1);
        }
    }
    assert_eq!(
        marker_window(&e, 1.5, &ctx),
        Some(MarkerWindow {
            fps: 4.0,
            before: Some(1.0),
            after: Some(2.0),
        })
    );
    assert_eq!(
        marker_window(&e, 0.5, &ctx),
        Some(MarkerWindow {
            fps: 4.0,
            before: None,
            after: Some(1.0),
        })
    );

    // Strobe filters first: with every 2nd beat, the frame after beat
    // index 1 still sees indices 0 and 2 as its neighbours — the
    // window is the triggers the envelope actually consumes.
    for p in &mut e.params {
        match p.id.as_str() {
            "mode" => p.value = EffectValue::Choice(2),
            "every_nth" => p.value = EffectValue::Float(Property::fixed(2.0)),
            _ => {}
        }
    }
    assert_eq!(
        marker_window(&e, 2.5, &ctx),
        Some(MarkerWindow {
            fps: 4.0,
            before: Some(1.0),
            after: Some(3.0),
        })
    );
}

#[test]
fn block_hash01_is_deterministic_bounded_and_varies() {
    let a = block_hash01(7, 0, 3, 5, 2);
    let b = block_hash01(7, 0, 3, 5, 2);
    assert_eq!(a, b, "same inputs, same hash");
    assert!((0.0..1.0).contains(&a), "hash lands in [0, 1)");

    // Changing any one input moves the hash (checked, not proved
    // statistically — a collision is possible in principle but
    // vanishingly unlikely for a well-mixed hash, and none of these
    // particular inputs happen to collide).
    assert_ne!(a, block_hash01(8, 0, 3, 5, 2), "seed matters");
    assert_ne!(a, block_hash01(7, 1, 3, 5, 2), "channel matters");
    assert_ne!(a, block_hash01(7, 0, 4, 5, 2), "block x matters");
    assert_ne!(a, block_hash01(7, 0, 3, 6, 2), "block y matters");
    assert_ne!(a, block_hash01(7, 0, 3, 5, 3), "tick matters");
}

#[test]
fn block_glitch_instantiates_and_resolves() {
    let e = instantiate("block_glitch").unwrap();
    assert_eq!(e.float_at("intensity", 0.0), Some(0.35));
    assert!(matches!(e.param("seed"), Some(EffectValue::Seed(_))));
    assert_eq!(e.float_at("block_size", 0.0), Some(24.0));
    assert_eq!(e.float_at("block_jitter", 0.0), Some(25.0));
    assert_eq!(e.float_at("block_amount", 0.0), Some(3.0));
    assert_eq!(e.float_at("channel_offset", 0.0), Some(1.0));
    assert_eq!(e.float_at("slice_repeat", 0.0), Some(20.0));

    // Resolving is deterministic: the same instance at the same time
    // yields the identical result, twice — and the px_scale factor
    // (0.5 here) reaches the px@comp parameters exactly like Transform
    // and Shake's do.
    let a = resolve_stack(
        std::slice::from_ref(&e),
        0.4,
        1000.0,
        0.5,
        &MarkerContext::NONE,
    );
    let b = resolve_stack(
        std::slice::from_ref(&e),
        0.4,
        1000.0,
        0.5,
        &MarkerContext::NONE,
    );
    assert_eq!(a, b);
    let Resolved::BlockGlitch {
        intensity,
        tick,
        block_size_px,
        jitter_frac,
        amount_px,
        chan_px,
        slice_frac,
        mix,
        ..
    } = a[0]
    else {
        panic!("expected a BlockGlitch");
    };
    assert_eq!(intensity, 0.35);
    assert_eq!(tick, 3); // floor(0.4 * GLITCH_TICK_HZ 8) = 3
    assert_eq!(block_size_px, 12.0); // 24 px@comp * px_scale 0.5
    assert_eq!(jitter_frac, 0.25);
    assert_eq!(amount_px, 30.0); // 3% of a 1000px diagonal
    assert_eq!(chan_px, 10.0); // 1% of a 1000px diagonal
    assert_eq!(slice_frac, 0.20);
    assert_eq!(mix, 1.0);

    // A different frame ticks differently (the per-block hash itself
    // only runs inside cpu::block_glitch/the kernel, not here).
    let later = resolve_stack(
        std::slice::from_ref(&e),
        0.9,
        1000.0,
        0.5,
        &MarkerContext::NONE,
    );
    assert_ne!(a, later, "the tick moves between frames");
}

#[test]
fn scanlines_instantiates_and_resolves() {
    let e = instantiate("scanlines").unwrap();
    assert_eq!(e.float_at("intensity", 0.0), Some(0.35));
    assert_eq!(e.float_at("scanline_period", 0.0), Some(3.0));
    // Darkness is gone (FX-13/K-147): Intensity is the single darken dial.
    assert_eq!(e.float_at("scanline_darkness", 0.0), None);
    assert_eq!(e.float_at("scanline_roll", 0.0), Some(0.0));
    assert!(matches!(
        e.param("scanline_interlace"),
        Some(EffectValue::Bool(false))
    ));

    let a = resolve_stack(
        std::slice::from_ref(&e),
        0.4,
        1000.0,
        0.5,
        &MarkerContext::NONE,
    );
    assert_eq!(
        a,
        vec![Resolved::Scanlines {
            intensity: 0.35, // no Darkness param, so the raw Intensity stands
            period_px: 1.5,  // 3 px@comp * px_scale 0.5
            roll_px: 0.0,    // roll speed 0
            interlace: false,
            mix: 1.0,
        }]
    );
}

#[test]
fn scanlines_migrates_old_darkness_into_intensity() {
    // An old project (FX-13/K-147) carried a separate Darkness param
    // (0..100). On load it folds into the single Intensity so the darken is
    // the old Intensity × Darkness product exactly.
    let mut e = instantiate("scanlines").unwrap();
    // Restore the pre-K-147 shape: Intensity 0.5 plus a Darkness of 80%.
    for p in &mut e.params {
        if p.id == "intensity" {
            p.value = EffectValue::Float(Property::fixed(0.5));
        }
    }
    e.params.push(crate::model::EffectParam {
        id: "scanline_darkness".to_owned(),
        value: EffectValue::Float(Property::fixed(80.0)),
        extra: serde_json::Map::new(),
    });
    let a = resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    // 0.5 × 0.80 = 0.40.
    let Resolved::Scanlines { intensity, .. } = a[0] else {
        panic!("expected a scanlines op");
    };
    assert!(
        (intensity - 0.40).abs() < 1e-6,
        "old Darkness folds into Intensity: got {intensity}"
    );
}

#[test]
fn cpu_block_glitch_is_identity_at_zero_intensity() {
    let (w, h) = (17u32, 9u32);
    let img = transform_card(w, h);

    // Intensity 0: every hashed quantity collapses — the early return
    // skips the blend entirely, so this holds for any Mix, unlike the
    // blur family's tap-sum coincidence.
    let mut a = img.clone();
    cpu::block_glitch(&mut a, w, h, 0.0, 7, 3, 6.0, 0.5, 5.0, 2.0, 0.5, 0.4);
    assert_eq!(a, img, "intensity 0 is the exact identity");
}

#[test]
fn cpu_scanlines_is_identity_at_zero_intensity() {
    let (w, h) = (17u32, 9u32);
    let img = transform_card(w, h);
    let mut a = img.clone();
    cpu::scanlines(&mut a, w, h, 0.0, 3.0, 1.0, true, 0.4);
    assert_eq!(a, img, "intensity 0 is the exact identity");
}

#[test]
fn cpu_block_glitch_params_each_move_the_result() {
    // Every hashed quantity at zero is still an exact identity even
    // though block displacement runs (not the early return) — the
    // "scale by zero" branches must themselves be exact.
    let (w, h) = (40u32, 40u32);
    let img = transform_card(w, h);
    let (seed, tick) = (42u32, 5i32);
    let run = |amount: f32, jitter: f32, chan: f32, slice: f32| {
        let mut out = img.clone();
        cpu::block_glitch(
            &mut out, w, h, 1.0, seed, tick, 8.0, jitter, amount, chan, slice, 1.0,
        );
        out
    };
    let zero = run(0.0, 0.0, 0.0, 0.0);
    assert_eq!(
        zero, img,
        "every hashed quantity at zero is the identity too"
    );
    assert_ne!(
        run(6.0, 0.0, 0.0, 0.0),
        zero,
        "displacement amount moves pixels"
    );
    assert_ne!(run(0.0, 0.5, 0.0, 0.0), zero, "grid jitter moves pixels");
    assert_ne!(
        run(0.0, 0.0, 4.0, 0.0),
        zero,
        "channel offset splits colour"
    );
    assert_ne!(run(0.0, 0.0, 0.0, 1.0), zero, "slice repeat folds rows");
}

#[test]
fn cpu_scanlines_darken_a_periodic_band() {
    let (w, h) = (4u32, 12u32);
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for px in img.chunks_exact_mut(4) {
        px.copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
    }
    let red_at = |img: &[f32], y: u32| img[(y * w * 4) as usize];

    // Period 4px, no roll, no interlace: rows 0-1 of every period are
    // bright, rows 2-3 dark — the same shape every period. Intensity 0.5
    // takes the dark rows to half brightness (1 − intensity).
    let mut out = img.clone();
    cpu::scanlines(&mut out, w, h, 0.5, 4.0, 0.0, false, 1.0);
    for y in 0..h {
        let expect = if (y % 4) < 2 { 1.0 } else { 0.5 };
        assert_eq!(red_at(&out, y), expect, "row {y}");
    }

    // Interlace flips which half darkens on odd periods only: period 1
    // (rows 4-7) is dark-then-bright instead of bright-then-dark;
    // period 0 and period 2 (even) are unaffected.
    let mut inter = img.clone();
    cpu::scanlines(&mut inter, w, h, 0.5, 4.0, 0.0, true, 1.0);
    assert_eq!(red_at(&inter, 0), 1.0, "period 0 unaffected");
    assert_eq!(red_at(&inter, 2), 0.5, "period 0 unaffected");
    assert_eq!(red_at(&inter, 4), 0.5, "period 1 flips: dark first");
    assert_eq!(red_at(&inter, 6), 1.0, "period 1 flips: bright second");
    assert_eq!(red_at(&inter, 8), 1.0, "period 2 (even) unflipped again");
    assert_eq!(red_at(&inter, 10), 0.5, "period 2 (even) unflipped again");
}
