use std::sync::Arc;

use super::*;
use crate::anim::{Animation, Property};
use crate::expression::ExpressionContext;
use crate::model::{Composition, EffectInstance, EffectNamespace, EffectValue, Layer};

// These tests are about *parameter resolution*, not about expressions, so they
// call the resolvers without an expression context and get the detached one.
// Shadowing the two entry points here keeps that out of every call below —
// otherwise the same argument would be spelled out ninety times.
fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<Resolved> {
    super::resolve_stack(
        effects,
        lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    )
}

fn resolve_stack_temporal_named(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<(uuid::Uuid, Resolved)> {
    super::resolve_stack_temporal_named(
        effects,
        sample_lt,
        frame_lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    )
}

fn resolve_stack_temporal(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<Resolved> {
    super::resolve_stack_temporal(
        effects,
        sample_lt,
        frame_lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    )
}

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
        markers: Vec::new(),
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
        retime: None,
        interpolation: Default::default(),
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
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

// The render-time indicator (docs/13 §7.1) puts a measured millisecond on the
// row of the effect stack that spent it, and the only thing that can say which
// row an op came from is the walk that resolved it: `resolve_one` drops
// placeholders, unknown names and the orchestration-only effects, so filtering
// the effect list afterwards would misalign the moment a stack held one of
// those. The named walk must therefore stay op-for-op identical to the plain
// one, and carry the id beside each op.
#[test]
fn the_named_resolve_is_the_plain_one_with_the_ids_kept() {
    let blur = instantiate("blur").unwrap();
    let mut off = instantiate("glow").unwrap();
    off.enabled = false;
    // Posterize Time is an orchestration-only effect: it is enabled, built in,
    // and resolves to no op at all — the case a list filter would get wrong.
    let posterize = instantiate("posterize_time").unwrap();
    let glow = instantiate("glow").unwrap();
    let stack = [blur.clone(), off, posterize, glow.clone()];

    let plain = resolve_stack(&stack, 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    let named = resolve_stack_temporal_named(&stack, 0.0, 0.0, 1000.0, 1.0, &MarkerContext::NONE);

    assert_eq!(
        named.iter().map(|(_, op)| *op).collect::<Vec<_>>(),
        plain,
        "the same ops, in the same order"
    );
    assert_eq!(
        named.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![blur.id, glow.id],
        "each op carries the id of the effect that wrote it, disabled and \
         orchestration-only effects skipped"
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

/// A neutral resolved Depth of field with the given per-side radii: every
/// control the aperture and highlight groups added at the value that makes the
/// kernel take its historical path (K-313). Spelled once here because
/// functional-update syntax does not reach inside an enum variant.
fn resolved_dof(near_aperture: f32, far_aperture: f32, focus_point: [f32; 2]) -> Resolved {
    let (blade_normals, apothem2) = crate::fx::aperture_blades(6, 0.0);
    Resolved::Dof {
        focus: 0.5,
        range: 0.1,
        near_aperture,
        far_aperture,
        depth_invert: false,
        blade_normals,
        blade_count: 6,
        apothem2,
        roundness: 1.0,
        rim: 0.0,
        aspect_scale: [1.0, 1.0],
        threshold: 1.0,
        bokeh_power: 1.0,
        repeat_edge: true,
        depth_bound: false,
        depth_channel: 0,
        use_focus_point: false,
        focus_point,
        gamma: 1.0,
        remove_edge_leak: 0.0,
        detect_edge_threshold: 0.1,
        display: 0,
        mix: 1.0,
    }
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
    assert_eq!(r, vec![resolved_dof(4.0, 4.0, [480.0, 270.0])]);
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
    assert_eq!(r, vec![resolved_dof(20.0, 8.0, [960.0, 540.0])]);

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
    assert_eq!(r, vec![resolved_dof(12.0, 12.0, [960.0, 540.0])]);
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
        markers: Vec::new(),
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
        retime: None,
        interpolation: Default::default(),
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
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

// ---------------------------------------------------------------------------
// Lens flare (docs/08 §3.27, docs/impl/lens-flare.md §8, K-256)
// ---------------------------------------------------------------------------

// §8.1 — the in-house FFT: a forward-then-inverse round trip returns the
// input, an 8-point transform matches the direct DFT sum, and Parseval's
// identity holds under the ortho normalisation.
#[test]
fn lens_flare_fft_round_trips_matches_dft_and_conserves_energy() {
    use crate::fx::fft::{fft_inplace, Cx};
    let src: Vec<Cx> = (0..8)
        .map(|i| Cx::new((i as f64 * 0.7).sin(), (i as f64 * 1.3).cos()))
        .collect();

    // Round trip.
    let mut data = src.clone();
    fft_inplace(&mut data, false);
    let spectrum = data.clone();
    fft_inplace(&mut data, true);
    for (a, b) in data.iter().zip(src.iter()) {
        assert!((a.re - b.re).abs() < 1e-12 && (a.im - b.im).abs() < 1e-12);
    }

    // Direct ortho DFT.
    let n = src.len();
    for (k, s) in spectrum.iter().enumerate() {
        let mut sum = Cx::ZERO;
        for (j, x) in src.iter().enumerate() {
            let ang = -std::f64::consts::TAU * k as f64 * j as f64 / n as f64;
            sum = sum + *x * Cx::cis(ang);
        }
        sum = sum.scale(1.0 / (n as f64).sqrt());
        assert!((s.re - sum.re).abs() < 1e-12 && (s.im - sum.im).abs() < 1e-12);
    }

    // Parseval (ortho: energies equal exactly).
    let e_time: f64 = src.iter().map(|z| z.norm_sq()).sum();
    let e_freq: f64 = spectrum.iter().map(|z| z.norm_sq()).sum();
    assert!((e_time - e_freq).abs() < 1e-9);
}

// §8.3 — optics units: the Cauchy fit reproduces n_d exactly and the Abbe
// number within tolerance; refraction matches Snell; Fresnel at normal
// incidence is the textbook ((n1-n2)/(n1+n2))²; the quarter-wave MgF₂
// coating cuts the reflectance, and extra layers cut it further (K-261).
#[test]
fn lens_flare_optics_match_the_textbook() {
    use crate::fx::lens_flare::*;
    let (a, b) = cauchy_from_abbe(1.62, 60.3);
    let n_d = cauchy_ior(a, b, 587.56);
    assert!((n_d - 1.62).abs() < 1e-5, "n_d {n_d}");
    let n_f = cauchy_ior(a, b, 486.13);
    let n_c = cauchy_ior(a, b, 656.27);
    let v = (n_d - 1.0) / (n_f - n_c);
    assert!((v - 60.3).abs() < 0.05, "V {v}");

    // Snell at 45° into n = 1.5 glass: sin(t) = sin(45°)/1.5.
    let i = [(0.5f32).sqrt(), 0.0, (0.5f32).sqrt()];
    let t = refract3(i, [0.0, 0.0, -1.0], 1.0 / 1.5).expect("no TIR at 45°");
    let sin_t = t[0].hypot(t[1]);
    assert!((sin_t - (0.5f32).sqrt() / 1.5).abs() < 1e-6);
    // Total internal reflection from the dense side at a grazing angle.
    let g = [(0.99f32).sqrt(), 0.0, (0.01f32).sqrt()];
    assert!(refract3(g, [0.0, 0.0, -1.0], 1.5).is_none());

    // Normal-incidence Fresnel.
    let r = fresnel_cos(1.0, 1.0, 1.5);
    let expect = ((1.0f32 - 1.5) / (1.0 + 1.5)).powi(2);
    assert!((r - expect).abs() < 1e-4, "{r} vs {expect}");

    // Coatings, on ordinary crown glass (K-356). Note the glass: MgF₂ is
    // very nearly the IDEAL single layer for n ≈ 1.9, because 1.38² = 1.904,
    // so a stack comparison there measures a coincidence rather than a
    // coating. n = 1.5 is the honest case and the common one.
    let plain = fresnel_cos(1.0, 1.0, 1.5);
    let one = surface_reflectance(1.0, 1.0, 1.5, 1.0, 550.0, 1.0);
    let three = surface_reflectance(1.0, 1.0, 1.5, 3.0, 550.0, 1.0);
    assert!(one < plain, "coated {one} should be below bare {plain}");
    assert!(
        three < one,
        "the broadband stack {three} should beat the single layer {one}"
    );

    // **Reflectance varies across the band, and that is the point.** A real
    // multicoat has minima rather than a flat floor, which is what gives
    // ghosts their colour: the stack reflects some wavelengths several times
    // more than others. The old single-number model could not do this.
    let across: Vec<f32> = [430.0f32, 500.0, 550.0, 620.0, 680.0]
        .iter()
        .map(|&nm| surface_reflectance(1.0, 1.0, 1.5, 3.0, nm, 1.0))
        .collect();
    let lo = across.iter().copied().fold(f32::MAX, f32::min);
    let hi = across.iter().copied().fold(0.0f32, f32::max);
    assert!(
        hi > lo * 3.0,
        "a broadband stack must be wavelength-selective: {across:?}"
    );

    // **And it shifts with the angle of incidence**, because the phase
    // thickness carries a cos θ — which is the observed effect that a ghost
    // changes hue as its source moves off axis. Steeply off-axis, the band
    // has moved enough that the design wavelength is no longer the minimum.
    let straight = surface_reflectance(1.0, 1.0, 1.5, 3.0, 550.0, 1.0);
    let oblique = surface_reflectance(0.6, 1.0, 1.5, 3.0, 550.0, 1.0);
    assert!(
        oblique > straight * 1.5,
        "the coating must vary with angle: {oblique} at 53° vs {straight} \
         at normal"
    );

    // The Coating dial at 0 is bare glass regardless of the file layers.
    let off = surface_reflectance(1.0, 1.0, 1.5, 3.0, 550.0, 0.0);
    assert!((off - plain).abs() < 1e-6);

    // A bare stack (0 layers) is exactly the uncoated interface, and the
    // transfer matrix agrees with plain Fresnel there — the degenerate case
    // that proves the chain closes correctly.
    let empty = stack_reflectance(1.0, 1.0, 1.5, &coating_stack(0.0), 550.0);
    assert!(
        (empty - plain).abs() < 1e-5,
        "an empty stack {empty} must equal bare Fresnel {plain}"
    );
}

// §8.4 — the prescription library and pair ranking (K-261, curated to
// twenty K-264): every bundled .lens file parses with a sane surface count,
// focal length and a stop surface; the bake's pair list is deterministic,
// non-empty, and every pair joins two genuine glass interfaces.
#[test]
fn lens_flare_library_parses_and_pairs_rank_deterministically() {
    use crate::fx::lens_flare::*;
    use crate::fx::lens_library::{LENS_LIBRARY, LENS_OPTIONS};
    assert_eq!(LENS_LIBRARY.len(), 20, "the curated library is twenty");
    assert_eq!(LENS_OPTIONS.len(), LENS_LIBRARY.len());
    for (i, entry) in LENS_LIBRARY.iter().enumerate() {
        assert_eq!(LENS_OPTIONS[i], entry.name, "options align with entries");
    }
    // Sorted by name, so the picker reads alphabetically and a saved index
    // is reproducible from the name list alone.
    for pair in LENS_LIBRARY.windows(2) {
        assert!(
            pair[0].name < pair[1].name,
            "{} !< {}",
            pair[0].name,
            pair[1].name
        );
    }
    for entry in LENS_LIBRARY.iter() {
        let lens =
            parse_lens(entry.text).unwrap_or_else(|| panic!("{} failed to parse", entry.name));
        assert!(
            (2.0..2000.0).contains(&lens.focal_mm),
            "{}: focal {}",
            entry.name,
            lens.focal_mm
        );
        assert!(
            lens.surfaces.len() >= 3 && lens.surfaces.len() <= 64,
            "{}: {} surfaces",
            entry.name,
            lens.surfaces.len()
        );
        assert!(
            lens.surfaces.iter().all(|s| s.semi_ap_mm > 0.0),
            "{}: non-positive semi-aperture",
            entry.name
        );
    }

    // Deterministic bake: two runs agree entirely (pairs, sprite, gain).
    let p = default_flare_params();
    let a = bake(&p);
    let b = bake(&p);
    assert_eq!(a.pairs, b.pairs);
    // Bit-identical across runs INCLUDING the K-365 field slices, which are
    // baked in parallel: `collect` restores slice order, so the thread pool
    // cannot reach the pixels.
    assert_eq!(a.starburst, b.starburst);
    assert_eq!(
        a.starburst.len(),
        STARBURST_FIELDS * (STARBURST_RES * STARBURST_RES * 3) as usize,
        "the sprite is the field slices concatenated, slice-major"
    );
    assert_eq!(a.energy_gain, b.energy_gain);
    assert!(!a.pairs.is_empty());
    for pair in &a.pairs {
        assert!(pair[0] < pair[1]);
        assert!((pair[1] as usize) < a.surfaces.len());
    }
}

/// One angular ring of a starburst slice: `bins` samples of the pattern's
/// luma at `radius` (a fraction of the sprite's half-size), taken from the
/// nearest texel and smoothed over ±`SMOOTH` bins so a spike reads as one
/// bump rather than a comb of them.
fn starburst_ring(slice: &[f32], n: usize, radius: f32) -> Vec<f32> {
    const BINS: usize = 720;
    const SMOOTH: isize = 5;
    let c = (n - 1) as f32 / 2.0;
    let r = radius * (n as f32 / 2.0);
    let raw: Vec<f32> = (0..BINS)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / BINS as f32;
            let x = (c + r * a.cos()).round().clamp(0.0, (n - 1) as f32) as usize;
            let y = (c + r * a.sin()).round().clamp(0.0, (n - 1) as f32) as usize;
            let i = (y * n + x) * 3;
            0.2126 * slice[i] + 0.7152 * slice[i + 1] + 0.0722 * slice[i + 2]
        })
        .collect();
    (0..BINS)
        .map(|k| {
            let mut s = 0.0;
            for d in -SMOOTH..=SMOOTH {
                s += raw[(k as isize + d).rem_euclid(BINS as isize) as usize];
            }
            s / (2 * SMOOTH + 1) as f32
        })
        .collect()
}

/// Strict local maxima of a circular ring that stand above `1.5 ×` its mean
/// — the diffraction spikes, counted without caring how bright they are.
fn starburst_spikes(ring: &[f32]) -> usize {
    let n = ring.len();
    let mean = ring.iter().sum::<f32>() / n as f32;
    (0..n)
        .filter(|&k| {
            let v = ring[k];
            v > 1.5 * mean && v > ring[(k + n - 1) % n] && v > ring[(k + 1) % n]
        })
        .count()
}

/// **The starburst still counts the blades** (K-256's physics, re-checked
/// after the K-365 field slices): the sprite is the iris polygon's
/// Fraunhofer diffraction, and a polygon's spikes run perpendicular to its
/// edges — so an EVEN blade count gives N spikes (opposite edges are
/// parallel and share a spike) and an ODD one gives 2N. Slice 0 is the
/// on-axis picture; a bake that lost the polygon, or concatenated its
/// slices in the wrong order, changes this count.
#[test]
fn starburst_slice_zero_counts_the_iris_blades() {
    use crate::fx::lens_flare::*;
    let n = STARBURST_RES as usize;
    for (blades, want) in [(6u32, 6usize), (5, 10)] {
        // Lens 18 is the cheapest bundled prescription. Roundness 0 keeps
        // the polygon a polygon — and the f-stop must be well down from the
        // lens's native 4.5, because `effective_roundness` rounds the iris
        // off near wide open (a real iris's blades barely meet there), and
        // a circle has no blades to count.
        let p = LensFlareParams {
            lens: 18,
            blades,
            roundness: 0.0,
            fstop: 16.0,
            ..default_flare_params()
        };
        let baked = bake(&p);
        let ring = starburst_ring(&baked.starburst[..n * n * 3], n, 0.3);
        assert_eq!(
            starburst_spikes(&ring),
            want,
            "{blades} blades must give {want} spikes"
        );
    }
}

/// **The cat's-eye is real** (K-365): at the sensor-corner field angle the
/// front and rear mechanical stops clip the iris into a sliver, so the last
/// field slice must differ from the on-axis one — and must still carry
/// light, because a starburst that goes black in the corners is a worse
/// picture than one that never changed. If `trace_transmit` ever silently
/// returned 1 everywhere, the first assertion fails; if it ever returned 0
/// everywhere, the second does.
#[test]
fn the_corner_field_slice_is_a_cats_eye_not_the_on_axis_sprite() {
    use crate::fx::lens_flare::*;
    let per = (STARBURST_RES * STARBURST_RES * 3) as usize;
    // Lens 16 is the Master Prime: fast enough that its stops really do
    // clip at the corner field angle. Lens 0 is the bundled APS-C design,
    // which at the full-frame corner passes nothing at all — the case the
    // dead-slice hold exists for.
    for lens in [16u32, 0] {
        let p = LensFlareParams {
            lens,
            ..default_flare_params()
        };
        let baked = bake(&p);
        let on_axis = &baked.starburst[..per];
        let corner = &baked.starburst[(STARBURST_FIELDS - 1) * per..];
        let energy = on_axis.iter().sum::<f32>().max(1e-9);
        let l1: f32 = on_axis
            .iter()
            .zip(corner)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / energy;
        assert!(
            l1 > 0.02,
            "lens {lens}: corner slice differs from on-axis by only {l1}"
        );
        let corner_energy = corner.iter().sum::<f32>();
        assert!(
            corner_energy > 0.1 * energy,
            "lens {lens}: the corner slice kept only {corner_energy} of \
             {energy} — the vignette has killed the sprite, not shaped it"
        );
    }
}

// The `lens_file` override (K-264): a custom .lens text replaces the
// picked lens entirely, its bake key never collides with the library's or
// with a different file's, and an unparsable file degrades to the pick.
#[test]
fn lens_flare_custom_lens_file_overrides_and_degrades() {
    use crate::fx::lens_flare::*;
    use crate::fx::lens_library::LENS_LIBRARY;
    let p = default_flare_params();
    // "Custom file" = the text of a DIFFERENT bundled lens, so the expected
    // result is exactly what picking that lens produces (native f-number
    // aside — a custom file estimates it from geometry).
    let other = crate::fx::lens_flare::LensFlareParams { lens: 0, ..p };
    let via_pick = bake(&other);
    let via_file = bake_with(&p, Some(LENS_LIBRARY[0].text));
    assert_eq!(via_file.surfaces.len(), via_pick.surfaces.len());
    assert_eq!(via_file.pairs, via_pick.pairs, "same glass, same ghosts");
    assert_eq!(via_file.focal_mm, via_pick.focal_mm);
    // The key separates library, custom, and edited-custom.
    let h = lens_text_hash(LENS_LIBRARY[0].text);
    let k_lib = bake_key(&p);
    let k_file = bake_key_with(&p, Some(h));
    let k_edit = bake_key_with(&p, Some(lens_text_hash("name: edited\n")));
    assert_ne!(k_lib, k_file);
    assert_ne!(k_file, k_edit);
    // Unparsable text degrades to the picked lens, bit-for-bit.
    let fallback = bake_with(&p, Some("not a prescription"));
    let picked = bake(&p);
    assert_eq!(fallback.pairs, picked.pairs);
    assert_eq!(fallback.starburst, picked.starburst);
    assert_eq!(fallback.energy_gain, picked.energy_gain);
}

// Px-dimensioned resolved fields rescale when the stack runs on a raster
// other than the one it resolved against (K-266) — the adjustment-layer
// preview bug: the flare's light hit the frame edge at 1500 of a 1920 comp
// because the preview factor was applied to the raster and not the params.
#[test]
fn resolved_px_fields_rescale_for_a_different_raster() {
    use crate::fx::{rescale_px, Resolved};
    let mut ops = vec![
        Resolved::Blur {
            radius_px: 10.0,
            edge: 0,
            mix: 1.0,
        },
        Resolved::LensFlare(crate::fx::lens_flare::LensFlareParams {
            light: [1000.0, 500.0],
            ..default_flare_params()
        }),
    ];
    rescale_px(&mut ops, 0.5);
    match &ops[0] {
        Resolved::Blur { radius_px, mix, .. } => {
            assert_eq!(*radius_px, 5.0, "px fields scale");
            assert_eq!(*mix, 1.0, "unitless fields do not");
        }
        other => panic!("unexpected {other:?}"),
    }
    match &ops[1] {
        Resolved::LensFlare(p) => {
            assert_eq!(p.light, [500.0, 250.0], "the flare's light is px@comp");
            assert_eq!(p.intensity, 1.0);
        }
        other => panic!("unexpected {other:?}"),
    }
    // Factor 1 is exactly a no-op.
    let mut same = vec![Resolved::Blur {
        radius_px: 7.0,
        edge: 0,
        mix: 1.0,
    }];
    rescale_px(&mut same, 1.0);
    match &same[0] {
        Resolved::Blur { radius_px, .. } => assert_eq!(*radius_px, 7.0),
        other => panic!("unexpected {other:?}"),
    }
}

// An anamorphic squeeze (or scale) below 1 asks the combine for flare
// coordinates past the buffer. Up to the 2× padding cap the buffer now
// renders wider and carries real flare there (K-267); past even the
// padded extent there is still NO flare (K-266) — the clamp-addressed tap
// used to repeat the edge row outward as a smear.
#[test]
fn lens_flare_combine_does_not_repeat_the_flare_past_its_buffer() {
    use crate::fx::lens_flare::*;
    let (w, h) = (64u32, 36u32);
    // Squeeze 0.5 sits inside the padding: the frame edge samples the
    // padded buffer's real content, not black.
    let p_half = LensFlareParams {
        anamorphic: 0.5,
        starburst_intensity: 0.0,
        ghost_softness: 0.0,
        ..default_flare_params()
    };
    let baked = bake(&p_half);
    let (rw, rh) = flare_pad_dims(w, h, p_half.anamorphic, p_half.scale);
    assert_eq!((rw, rh), (w * 2, h), "squeeze 0.5 pads to double width");
    let flare = vec![0.5_f32; (rw * rh * 3) as usize];
    let mut out = vec![0.0_f32; (w * h * 4) as usize];
    let lights = manual_light(&p_half, w, h);
    cpu_combine(&mut out, w, h, &p_half, &baked, &flare, w, h, &lights);
    let left_edge: f32 = (0..h).map(|y| out[((y * w) * 4) as usize]).sum();
    assert!(
        left_edge > 0.0,
        "K-267: the padded buffer must reach the squeezed frame edge"
    );
    // Squeeze 0.25 outruns even the 2× padding cap — and past the padded
    // buffer there must be nothing, never a repeated edge row.
    let p_quarter = LensFlareParams {
        anamorphic: 0.25,
        ..p_half
    };
    let (rw, rh) = flare_pad_dims(w, h, p_quarter.anamorphic, p_quarter.scale);
    assert_eq!((rw, rh), (w * 2, h), "the padding caps at 2x");
    let flare = vec![0.5_f32; (rw * rh * 3) as usize];
    let mut out = vec![0.0_f32; (w * h * 4) as usize];
    let lights = manual_light(&p_quarter, w, h);
    cpu_combine(&mut out, w, h, &p_quarter, &baked, &flare, w, h, &lights);
    // squeeze 0.25 maps x=0 to sx = 32 + (0.5-32)/0.25 = -94, u = -94.5/64
    // of the base width plus the 32 px pad offset: still far outside.
    let left_edge: f32 = (0..h).map(|y| out[((y * w) * 4) as usize]).sum();
    assert_eq!(
        left_edge, 0.0,
        "outside the padded buffer there is no flare"
    );
    // The centre still receives it.
    let centre = out[(((h / 2) * w + w / 2) * 4) as usize];
    assert!(centre > 0.0, "the squeezed flare itself still lands");
}

// Area sources (K-267): a practical spanning many tiles weighs as its
// whole lit area — every gated tile's flux lands on its nearest anchor —
// while a one-tile point source reads exactly as before (its own tile's
// brightest pixel through the gate). This was the owner's white-circle
// precomp: detected as one pixel, it flared like a pin-prick.
#[test]
fn lens_flare_detects_area_sources_as_summed_flux() {
    use crate::fx::lens_flare::*;
    let (w, h) = (256u32, 96u32);
    let mut matte = vec![0.0_f32; (w * h * 4) as usize];
    // A white disc spanning several detection tiles…
    let (cx, cy, r) = (48.0_f32, 48.0_f32, 40.0_f32);
    // …and a single-pixel practical far to the right.
    let dot = (16 * w + 240) as usize;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let inside = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) <= r;
            if inside || i / 4 == dot {
                matte[i] = 1.0;
                matte[i + 1] = 1.0;
                matte[i + 2] = 1.0;
                matte[i + 3] = 1.0;
            }
        }
    }
    // Threshold below the sources' luma: K-363's gate is "brighter than",
    // so a white (1.0) source at threshold 1.0 is at the line, not over it.
    let lights = detect_lights(&matte, w, h, 0.5, 0.25, true, [1.0, 1.0, 1.0]);
    assert_eq!(lights.len(), 2, "one disc anchor, one dot anchor");
    // The disc's anchor sits inside the disc, the dot's on the dot.
    let disc = &lights[0];
    let dot_light = &lights[1];
    assert!(
        (disc.pos[0] * w as f32 - cx).abs() < r && (disc.pos[1] * h as f32 - cy).abs() < r,
        "first anchor must sit in the disc: {:?}",
        disc.pos
    );
    assert!(
        (dot_light.pos[0] * w as f32 - 240.5).abs() < 1.0,
        "second anchor must sit on the dot: {:?}",
        dot_light.pos
    );
    // The dot reads as the classic point: white × gate(1.0) — and the disc
    // reads as MANY tiles of that, several times the dot's flux.
    let gate = threshold_gate(1.0, 0.5, 0.25);
    assert!(
        (dot_light.rgb[0] - gate).abs() < 1e-6,
        "{:?}",
        dot_light.rgb
    );
    assert!(
        disc.rgb[0] > dot_light.rgb[0] * 3.0,
        "area flux must dwarf the point: disc {} vs dot {}",
        disc.rgb[0],
        dot_light.rgb[0]
    );
}

// §8.5 (CPU side) — the trace lands rays: at the default light the top
// pairs put finite, weighted rays on the sensor, and stopping the iris
// down clips rays the wide stop passed.
#[test]
fn lens_flare_trace_lands_live_rays_with_sane_weights() {
    use crate::fx::lens_flare::*;
    let p = default_flare_params();
    let baked = bake(&p);
    let dir = light_direction([0.33, 0.30], 9.0 / 16.0, baked.focal_mm);
    let side = 12usize;
    let mut live = 0u32;
    for pair in baked.pairs.iter().take(10) {
        for j in 0..side {
            for i in 0..side {
                let u = (i as f32 / (side - 1) as f32) * 2.0 - 1.0;
                let v = (j as f32 / (side - 1) as f32) * 2.0 - 1.0;
                if u * u + v * v > 1.0 {
                    continue;
                }
                let origin = [u * baked.pupil_mm, v * baked.pupil_mm, baked.start_z_mm];
                if let Some((pos, w)) =
                    trace_splat(&baked, *pair, 550.0, origin, dir, 0.75, 1.0, 0.0)
                {
                    live += 1;
                    assert!(pos[0].is_finite() && pos[1].is_finite());
                    assert!((0.0..=1.0).contains(&w), "weight {w}");
                }
            }
        }
    }
    // Off-axis bundles legitimately lose rays to clips and TIR; the pin is
    // that a solid population still lands.
    assert!(live > 60, "only {live} live rays across the top pairs");

    // Stopping down kills rays that the wide stop passed.
    let (mut wide, mut stopped) = (0u32, 0u32);
    let pair = baked.pairs[0];
    for j in 0..side {
        for i in 0..side {
            let u = (i as f32 / (side - 1) as f32) * 2.0 - 1.0;
            let v = (j as f32 / (side - 1) as f32) * 2.0 - 1.0;
            if u * u + v * v > 1.0 {
                continue;
            }
            let origin = [u * baked.pupil_mm, v * baked.pupil_mm, baked.start_z_mm];
            if trace_splat(&baked, pair, 550.0, origin, dir, 0.75, 1.0, 0.0).is_some() {
                wide += 1;
            }
            let o2 = [origin[0] * 0.2, origin[1] * 0.2, origin[2]];
            if trace_splat(&baked, pair, 550.0, o2, dir, 0.75, 0.2, 0.0).is_some() {
                stopped += 1;
            }
        }
    }
    assert!(wide > 0);
    let _ = stopped; // the scaled spray always fits the scaled stop

    // The iris mask (K-261): centre 1, far outside 0, deterministic, and a
    // hexagon carves more of the unit square away than the circle.
    assert_eq!(pupil_mask(0.0, 0.0, 6, 0.0, 0.0, 0.1), 1.0);
    assert_eq!(pupil_mask(2.0, 0.0, 6, 0.0, 0.0, 0.1), 0.0);
    let probe = |roundness: f32| -> f32 {
        let mut acc = 0.0;
        for j in 0..64 {
            for i in 0..64 {
                let u = (i as f32 / 63.0) * 2.0 - 1.0;
                let v = (j as f32 / 63.0) * 2.0 - 1.0;
                acc += pupil_mask(u, v, 6, 0.0, roundness, 0.0);
            }
        }
        acc
    };
    assert!(
        probe(0.0) < probe(1.0),
        "a hexagon must pass less area than the circle"
    );
}

// §8.7 — neutral points: Intensity 0 and Mix 0 leave the working buffer
// bit-exactly untouched through the combine (the flare buffer is irrelevant
// then), and a fresh instance resolves to the documented defaults.
#[test]
fn lens_flare_neutral_points_and_default_resolve() {
    use crate::fx::lens_flare::*;
    let p = default_flare_params();
    let baked = bake(&p);
    let (w, h) = (8u32, 6u32);
    let src: Vec<f32> = (0..(w * h * 4) as usize)
        .map(|i| (i % 17) as f32 / 16.0)
        .collect();
    let flare = vec![0.5f32; (w * h * 3) as usize];

    let mut zero_intensity = src.clone();
    let pi0 = LensFlareParams {
        intensity: 0.0,
        ..p
    };
    let lights = manual_light(&pi0, w, h);
    cpu_combine(
        &mut zero_intensity,
        w,
        h,
        &pi0,
        &baked,
        &flare,
        w,
        h,
        &lights,
    );
    assert_eq!(zero_intensity, src, "Intensity 0 must be bit-exact");

    let mut zero_mix = src.clone();
    let pm0 = LensFlareParams { mix: 0.0, ..p };
    cpu_combine(&mut zero_mix, w, h, &pm0, &baked, &flare, w, h, &lights);
    assert_eq!(zero_mix, src, "Mix 0 must be bit-exact");

    // A live combine changes pixels (the effect is not a silent no-op).
    let mut live = src.clone();
    cpu_combine(&mut live, w, h, &p, &baked, &flare, w, h, &lights);
    assert_ne!(live, src);

    // Fresh instance -> resolve carries the documented defaults.
    let inst = instantiate("lens_flare").unwrap();
    let ops = resolve_stack(
        std::slice::from_ref(&inst),
        0.0,
        2202.9075,
        1.0,
        &MarkerContext::NONE,
    );
    match ops.as_slice() {
        [Resolved::LensFlare(rp)] => {
            // px@comp defaults at the schema's nominal 1080p (K-260).
            assert!((rp.light[0] - 640.0).abs() < 1e-3);
            assert!((rp.light[1] - 360.0).abs() < 1e-3);
            assert_eq!(rp.intensity, 1.0);
            assert_eq!(rp.lens, 16, "default lens is the Master Prime 50");
            assert_eq!(rp.blades, 8);
            assert_eq!(rp.max_ghosts, 60);
            assert_eq!(rp.quality, 1);
            assert_eq!(rp.mix, 1.0);
        }
        other => panic!("expected one LensFlare op, got {other:?}"),
    }
}

// §8.6 (CPU half) — the reference renderer produces finite energy at the
// defaults and follows the light. The full GPU-vs-CPU frame bound lives in
// the lumit-gpu tests.
#[test]
fn lens_flare_cpu_reference_renders_energy_and_reacts_to_the_light() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        quality: 0,
        max_ghosts: 12,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (96u32, 54u32);
    let a = cpu_flare(&p, &baked, w, h, &manual_light(&p, w, h));
    let energy_a: f32 = a.iter().sum();
    assert!(energy_a > 0.0, "the default flare renders no energy");
    assert!(a.iter().all(|v| v.is_finite()));

    // Moving the light moves the picture.
    let p2 = LensFlareParams {
        light: [67.0, 32.0],
        ..p
    };
    let b = cpu_flare(&p2, &baked, w, h, &manual_light(&p2, w, h));
    assert_ne!(a, b, "the flare must follow the light");
}

// Forward migration (K-258): a built-in instance saved before its schema
// grew a parameter gains it at the default on load — the panel had been
// drawing a dash and set_value refusing the id.
#[test]
fn lens_flare_backfill_restores_missing_params() {
    let mut inst = instantiate("lens_flare").unwrap();
    // Simulate a pre-K-257 save: strip the params that pass added.
    inst.params
        .retain(|p| !matches!(p.id.as_str(), "source_type" | "blend"));
    assert!(inst.params.iter().all(|p| p.id != "source_type"));
    let mut effects = vec![inst];
    backfill_builtin_params(&mut effects);
    let inst = &effects[0];
    for id in ["source_type", "blend"] {
        assert!(
            inst.params.iter().any(|p| p.id == id),
            "{id} must be backfilled"
        );
    }
    // Present values are never touched, and a second pass is a no-op.
    let count = inst.params.len();
    backfill_builtin_params(&mut effects);
    assert_eq!(effects[0].params.len(), count);
}

// The Background → Blend migration (K-289, superseding K-258). A project
// saved with Transparent lands on Add — the same pixels it always rendered —
// and one saved with Black lands on Normal, the flare on opaque black that
// option existed to produce. The dead parameter goes, because the schema no
// longer declares it and the panel cannot draw a row `set_value` refuses.
#[test]
fn lens_flare_background_migrates_to_the_blend_menu() {
    use crate::fx::lens_flare::{BLEND_ADD, BLEND_NORMAL};
    for (saved, want) in [(0u32, BLEND_ADD), (1, BLEND_NORMAL)] {
        let mut inst = instantiate("lens_flare").unwrap();
        inst.params.retain(|p| p.id != "blend");
        inst.params.push(crate::model::EffectParam {
            id: "background".to_owned(),
            value: EffectValue::Choice(saved),
            extra: serde_json::Map::new(),
        });
        let mut effects = vec![inst];
        backfill_builtin_params(&mut effects);
        assert!(
            effects[0].params.iter().all(|p| p.id != "background"),
            "the legacy parameter must be dropped"
        );
        assert!(
            matches!(effects[0].param("blend"), Some(EffectValue::Choice(c)) if *c == want),
            "background {saved} must migrate to blend {want}"
        );
        // Idempotent: loading twice cannot re-migrate or duplicate.
        let count = effects[0].params.len();
        backfill_builtin_params(&mut effects);
        assert_eq!(effects[0].params.len(), count);
        assert!(matches!(effects[0].param("blend"), Some(EffectValue::Choice(c)) if *c == want));
    }
}

// "This layer" (K-288): a fresh Lens flare added to a layer points its Matte
// source at that layer, so switching Source to Matte flares the lights in
// the picture the effect is already on — and on an adjustment layer, the
// composite below. Plain `instantiate` (presets, tests) leaves it unset, the
// labelled no-op it always was, and no other effect's Layer parameter moves.
#[test]
fn lens_flare_matte_defaults_to_the_layer_it_is_added_to() {
    let owner = uuid::Uuid::now_v7();

    let bare = instantiate("lens_flare").unwrap();
    assert_eq!(bare.layer_ref("matte"), None, "a preset stays unset");

    let mut inst = instantiate("lens_flare").unwrap();
    point_self_layer_params_at(&mut inst, owner);
    assert_eq!(inst.layer_ref("matte"), Some(owner));

    // DoF's depth pass is never the picture itself, so it is untouched.
    let mut dof = instantiate("dof").unwrap();
    point_self_layer_params_at(&mut dof, owner);
    assert_eq!(dof.layer_ref("depth"), None);
}

// Blend (K-289, replacing K-258's Background pair): Normal shows the flare
// element alone on opaque black, Add is the historical behaviour bit for
// bit, and every mode keeps the Intensity-0 passthrough exact.
#[test]
fn lens_flare_blend_normal_is_the_element_on_opaque_black() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        blend: BLEND_NORMAL,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (8u32, 6u32);
    let src: Vec<f32> = (0..(w * h * 4) as usize)
        .map(|i| (i % 13) as f32 / 24.0)
        .collect();
    let flare = vec![0.25f32; (w * h * 3) as usize];
    let lights = manual_light(&p, w, h);

    let mut normal = src.clone();
    cpu_combine(&mut normal, w, h, &p, &baked, &flare, w, h, &lights);
    let mut add = src.clone();
    let pa = LensFlareParams {
        blend: BLEND_ADD,
        ..p
    };
    cpu_combine(&mut add, w, h, &pa, &baked, &flare, w, h, &lights);
    for i in 0..(w * h) as usize {
        assert_eq!(normal[i * 4 + 3], 1.0, "alpha must be opaque");
        for c in 0..3 {
            // Add lays the same element over the layer, so Normal is Add
            // minus the layer: the element by itself.
            let element = add[i * 4 + c] - src[i * 4 + c];
            assert!(
                (normal[i * 4 + c] - element).abs() < 1e-6,
                "Normal must show the element alone"
            );
        }
    }

    // Neutral points ignore the blend: bit-exact passthrough.
    let mut neutral = src.clone();
    let p0 = LensFlareParams {
        intensity: 0.0,
        ..p
    };
    cpu_combine(&mut neutral, w, h, &p0, &baked, &flare, w, h, &lights);
    assert_eq!(neutral, src);
}

// The default Blend is Add, and Add is exactly what the effect did before
// the menu existed (K-289): `out = in + flare`, alpha saturating at 1. A
// regression here would silently move every flare anyone has already built.
#[test]
fn lens_flare_add_blend_is_the_historical_combine() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        blend: BLEND_ADD,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (8u32, 6u32);
    let src: Vec<f32> = (0..(w * h * 4) as usize)
        .map(|i| (i % 13) as f32 / 24.0)
        .collect();
    let flare = vec![0.25f32; (w * h * 3) as usize];
    let lights = manual_light(&p, w, h);

    let mut out = src.clone();
    cpu_combine(&mut out, w, h, &p, &baked, &flare, w, h, &lights);
    for i in 0..(w * h) as usize {
        let add: Vec<f32> = (0..3).map(|c| out[i * 4 + c] - src[i * 4 + c]).collect();
        let luma = 0.2126 * add[0] + 0.7152 * add[1] + 0.0722 * add[2];
        assert!(
            (out[i * 4 + 3] - (src[i * 4 + 3] + luma).min(1.0)).abs() < 1e-6,
            "alpha must be the historical saturating sum"
        );
    }

    // And the schema default really is Add.
    let inst = instantiate("lens_flare").unwrap();
    assert!(matches!(
        inst.param("blend"),
        Some(EffectValue::Choice(c)) if *c == BLEND_ADD
    ));
}

// Every Blend option is reachable, and the resolve clamps an index past the
// menu rather than faulting (K-289).
#[test]
fn lens_flare_blend_options_all_resolve() {
    use crate::fx::lens_flare::*;
    let last = BLEND_OPTIONS.len() as u32 - 1;
    for mode in 0..=last + 3 {
        let mut inst = instantiate("lens_flare").unwrap();
        for p in &mut inst.params {
            if p.id == "blend" {
                p.value = EffectValue::Choice(mode);
            }
        }
        let ops = resolve_stack(&[inst], 0.0, 2202.9, 1.0, &MarkerContext::NONE);
        let [Resolved::LensFlare(p)] = ops.as_slice() else {
            panic!("lens_flare must resolve to exactly one op");
        };
        assert_eq!(p.blend, mode.min(last));
    }
}

/// The Lens flare's float parameters read through the expression context like
/// every other effect's. A merge had left the flare's arm on the context-free
/// `float_at`, where `time` evaluates to nothing — so an expression-driven
/// flare silently ignored its expressions while every neighbour honoured
/// theirs.
#[test]
fn lens_flare_params_evaluate_expressions_in_context() {
    let mut inst = instantiate("lens_flare").unwrap();
    for p in &mut inst.params {
        if p.id == "intensity" {
            let mut prop = Property::fixed(1.0);
            prop.animation = Animation::Expression("time".into());
            p.value = EffectValue::Float(prop);
        }
    }
    let context = Arc::new(ExpressionContext {
        comp_time: 3.0,
        ..ExpressionContext::detached()
    });
    let ops = super::resolve_stack(&[inst], 0.0, 2202.9, 1.0, &MarkerContext::NONE, context);
    let [Resolved::LensFlare(p)] = ops.as_slice() else {
        panic!("lens_flare must resolve to exactly one op");
    };
    assert!(
        (p.intensity - 3.0).abs() < 1e-6,
        "intensity must follow the expression through the context: {}",
        p.intensity
    );
}

// The blend table itself (K-289), against the formulas written out by hand.
// The CPU twin is the oracle the WGSL `flare_blend` is pinned to, so it has
// to be right on its own terms first.
#[test]
fn flare_blend_matches_its_formulas() {
    use crate::fx::lens_flare::*;
    let d = [0.30_f32, 0.60, 0.10, 0.80];
    let e = [0.40_f32, 0.20, 0.70, 0.25];
    let close = |got: [f32; 4], want: [f32; 4], what: &str| {
        for c in 0..4 {
            assert!(
                (got[c] - want[c]).abs() < 1e-6,
                "{what} channel {c}: {} vs {}",
                got[c],
                want[c]
            );
        }
    };
    close(
        flare_blend(BLEND_NORMAL, d, e),
        [e[0], e[1], e[2], 1.0],
        "Normal",
    );
    close(
        flare_blend(BLEND_ADD, d, e),
        [0.70, 0.80, 0.80, 1.05],
        "Add",
    );
    close(
        flare_blend(2, d, e),
        [
            d[0] + e[0] - d[0] * e[0],
            d[1] + e[1] - d[1] * e[1],
            d[2] + e[2] - d[2] * e[2],
            d[3] + e[3] - d[3] * e[3],
        ],
        "Screen",
    );
    close(
        flare_blend(3, d, e),
        [d[0] * e[0], d[1] * e[1], d[2] * e[2], d[3] * e[3]],
        "Multiply",
    );
    close(flare_blend(7, d, e), [0.40, 0.60, 0.70, 0.80], "Lighten");
    close(flare_blend(8, d, e), [0.30, 0.20, 0.10, 0.25], "Darken");
    close(flare_blend(9, d, e), [0.10, 0.40, 0.60, 0.55], "Difference");
    close(
        flare_blend(11, d, e),
        [0.0, 0.40, 0.0, 0.55],
        "Subtract clamps at black",
    );
    // Divide by a zero element cannot produce a NaN or an infinity.
    let z = flare_blend(12, d, [0.0; 4]);
    assert!(z.iter().all(|v| v.is_finite()), "Divide must stay finite");
}

// Light tint and Use source colour (K-259): the tint multiplies every mode's
// light, and the toggle chooses whether a detected source's own colour rides
// with it. Manual carries the tint as its whole colour.
#[test]
fn lens_flare_light_tint_and_source_colour_toggle() {
    use crate::fx::lens_flare::*;
    // Manual: the light IS the tint (white by default).
    let p = default_flare_params();
    assert_eq!(manual_light(&p, 96, 54)[0].rgb, [1.0, 1.0, 1.0]);
    let warm = LensFlareParams {
        light_tint: [1.0, 0.5, 0.25],
        ..p
    };
    assert_eq!(manual_light(&warm, 96, 54)[0].rgb, [1.0, 0.5, 0.25]);

    // Matte: one blue-green source, gate fully open.
    let (w, h) = (64u32, 64u32);
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    let i = ((20 * w + 20) * 4) as usize;
    matte[i] = 0.5;
    matte[i + 1] = 2.0;
    matte[i + 2] = 4.0;
    matte[i + 3] = 1.0;

    // Source colour ON, tint white: the light is the source colour.
    let on = detect_lights(&matte, w, h, 0.5, 0.0, true, [1.0; 3]);
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].rgb, [0.5, 2.0, 4.0]);

    // Source colour OFF: white through the tint alone — the "this matte only
    // says where" case.
    let off = detect_lights(&matte, w, h, 0.5, 0.0, false, [1.0; 3]);
    assert_eq!(off[0].rgb, [1.0, 1.0, 1.0]);
    let off_tinted = detect_lights(&matte, w, h, 0.5, 0.0, false, [1.0, 0.5, 0.25]);
    assert_eq!(off_tinted[0].rgb, [1.0, 0.5, 0.25]);
    // …and its position is unchanged by either (only the colour differs).
    assert_eq!(off[0].pos, on[0].pos);

    // Source colour ON with a tint: the two multiply.
    let both = detect_lights(&matte, w, h, 0.5, 0.0, true, [1.0, 0.5, 0.25]);
    assert_eq!(both[0].rgb, [0.5, 1.0, 1.0]);

    // A black tint kills the flare without touching detection.
    let dark = detect_lights(&matte, w, h, 0.5, 0.0, true, [0.0; 3]);
    assert_eq!(dark[0].rgb, [0.0, 0.0, 0.0]);

    // The tint is NOT a bake input: changing it must not re-key the bake
    // (animating it would otherwise rebake every frame).
    assert_eq!(bake_key(&p), bake_key(&warm));
    assert_eq!(
        bake_key(&p),
        bake_key(&LensFlareParams {
            use_source_colour: false,
            ..p
        })
    );
}

// The thin-lens focus shift (K-260): zero at infinity, growing as focus
// nears, never past one focal length. (The K-260 paraxial sensor
// calibration is superseded by K-261: the FlareSim prescriptions carry
// their own measured back-focal chains.)
#[test]
fn lens_flare_focus_shift_follows_the_thin_lens() {
    use crate::fx::lens_flare::focus_shift_mm;
    assert_eq!(focus_shift_mm(0.0, 50.0), 0.0);
    assert!(focus_shift_mm(100.0, 50.0) < 0.03);
    let near = focus_shift_mm(1.0, 50.0);
    assert!((near - 2500.0 / 950.0).abs() < 1e-3, "1 m shift {near}");
    assert!(focus_shift_mm(0.2, 50.0) <= 50.0);
}

// The splat guard (K-366), tested where the old quad bugs lived. Quads
// connected rays across caustic folds, and the sliver/inflate rescue
// machinery (K-261..K-264) existed to survive that; splats never connect
// rays, so what needs pinning now is the deposit itself: flux is conserved
// away from the density cap, a footprint never drops below the anti-alias
// floor, and a fold (near-parallel axes) deposits a finite bright line
// rather than nothing or a spike.
#[test]
fn lens_flare_splats_conserve_flux_and_survive_folds() {
    use crate::fx::lens_flare::*;
    let sum = |buf: &[f32]| -> f32 { buf.iter().sum() };
    let (w, h) = (64u32, 64u32);

    // An ordinary footprint: deposited flux equals the flux put in (the
    // tent integrates to the parallelogram's area, which the divisor is).
    let mut out = vec![0.0_f32; (w * h * 3) as usize];
    splat_ray(
        &mut out,
        w,
        h,
        [32.0, 32.0],
        [4.0, 0.0],
        [0.0, 3.0],
        [7.0, 5.0, 3.0],
        16.0,
    );
    let total = sum(&out);
    assert!(
        (total - 15.0).abs() < 0.15,
        "an uncapped splat deposits its whole flux: {total} vs 15"
    );

    // A caustic fold: axes long but nearly parallel. The deposit must be
    // finite (the density cap) and non-zero (the anti-alias floor) — the
    // two halves of what the quad machinery got wrong in turn.
    let mut fold = vec![0.0_f32; (w * h * 3) as usize];
    splat_ray(
        &mut fold,
        w,
        h,
        [32.0, 32.0],
        [10.0, 0.0],
        [10.0, 1e-4],
        [1.0, 1.0, 1.0],
        16.0,
    );
    let fold_total = sum(&fold);
    assert!(fold_total > 0.0, "a fold still deposits");
    let peak = fold.iter().fold(0.0_f32, |a, &b| a.max(b));
    assert!(
        peak <= 1.0 / (MIN_AREA_FRAC * 16.0) + 1.0,
        "the density cap bounds a fold's brightness: peak {peak}"
    );

    // A sub-pixel footprint deposits over at least a pixel, not nothing.
    let mut tiny = vec![0.0_f32; (w * h * 3) as usize];
    splat_ray(
        &mut tiny,
        w,
        h,
        [32.0, 32.0],
        [0.05, 0.0],
        [0.0, 0.05],
        [1.0, 1.0, 1.0],
        16.0,
    );
    assert!(
        sum(&tiny) > 0.0,
        "the anti-alias floor keeps sub-pixel rays"
    );

    // Off-raster splats are calmly clipped.
    let mut off = vec![0.0_f32; (w * h * 3) as usize];
    splat_ray(
        &mut off,
        w,
        h,
        [-100.0, -100.0],
        [4.0, 0.0],
        [0.0, 4.0],
        [1.0, 1.0, 1.0],
        16.0,
    );
    assert_eq!(sum(&off), 0.0);

    // ray_axes: central differences where neighbours live, the floor when
    // none do, and a dead axis borrowed from the live one at right angles.
    let side = 3usize;
    let mut corners: Vec<Option<lens_flare::Corner>> = vec![None; side * side];
    corners[4] = Some(([10.0, 10.0], 1.0, [1.0; 3]));
    let (a1, a2) = ray_axes(&corners, side, 1, 1);
    assert_eq!(
        (a1, a2),
        ([MIN_SPLAT_AXIS_PX, 0.0], [0.0, MIN_SPLAT_AXIS_PX])
    );
    corners[3] = Some(([6.0, 10.0], 1.0, [1.0; 3]));
    corners[5] = Some(([14.0, 10.0], 1.0, [1.0; 3]));
    let (a1, a2) = ray_axes(&corners, side, 1, 1);
    assert_eq!(a1, [2.0, 0.0], "central difference halved twice: 8/2/2");
    assert!(
        (a2[0] - 0.0).abs() < 1e-6 && (a2[1].abs() - MIN_SPLAT_AXIS_PX).abs() < 1e-6,
        "the dead axis sits at right angles to the live one: {a2:?}"
    );
}

#[test]
fn lens_flare_grid_budget_follows_ghost_size() {
    use crate::fx::lens_flare::*;
    // Monotonic (non-strict) in spread, and never outside the clamp. A
    // tight blob gets the FULL base since K-265 — the half rung starved
    // caustic rims into sunflower teeth on the owner's EF 70-200.
    let tight = pair_grid(64, 0.05);
    let mid = pair_grid(64, 0.3);
    let wide = pair_grid(64, 1.0);
    let huge = pair_grid(64, 4.0);
    assert_eq!(tight, mid, "no half rung: small ghosts keep the base");
    assert!(mid < wide && wide < huge, "{mid} {wide} {huge}");
    assert!(tight >= 8 && huge <= 512);
    // Degenerate inputs stay in range rather than exploding a dispatch.
    assert!((8..=512).contains(&pair_grid(2, 0.0)));
    assert!((8..=512).contains(&pair_grid(512, 99.0)));
    // The Detail dial scales the base through one shared helper (K-265).
    assert_eq!(detail_base(64, 1.0), 64);
    assert_eq!(detail_base(64, 2.0), 128);
    assert_eq!(detail_base(64, 0.25), 16);
    assert_eq!(detail_base(64, 99.0), 256, "dial clamps at 4x");
    // …and the wavelength axis scales with it (K-265): more rays barely
    // touch spectral banding, so the dial must buy bands too.
    assert_eq!(detail_lambda(32, 1.0), 32);
    assert_eq!(detail_lambda(32, 2.0), 64);
    assert_eq!(detail_lambda(32, 4.0), 64, "capped: combos scale linearly");
    assert_eq!(detail_lambda(8, 0.25), 3, "floor keeps colour honest");

    // Every bundled pair carries a finite, non-negative spread.
    let p = default_flare_params();
    let baked = bake(&p);
    assert_eq!(baked.pairs.len(), baked.spreads.len());
    assert!(baked.spreads.iter().all(|s| s.is_finite() && *s >= 0.0));
}

// Frame-time grid probe (K-267): the bake spread is a bounding-box measure
// and misses folds — a pair the same overall size can stretch several-fold
// locally at a corner light, and those cells were the owner's choppy
// polyline edges on the 7Artisans. The probe must see the local stretch
// and raise the grid, the boost must respect its floor and caps, and the
// raw-rows entry the GPU seam uses must agree with the typed one exactly.
#[test]
fn lens_flare_frame_probe_sees_corner_stretch() {
    use crate::fx::lens_flare::*;
    let p = crate::fx::lens_flare::LensFlareParams {
        lens: 0,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let pair_count = baked.pairs.len().min(p.max_ghosts as usize);
    let stop_scale = fstop_scale(baked.native_fstop, p.fstop);
    let shift = focus_shift_mm(p.focus_m, baked.focal_mm);
    let corner = light_direction([0.85, 0.78], 9.0 / 16.0, baked.focal_mm);
    let sp = frame_grid_needs(&baked, pair_count, corner, p.coating, stop_scale, shift);
    assert_eq!(sp.len(), pair_count);
    assert!(sp.iter().all(|s| s.is_finite() && *s >= 1.0));
    // At least one renderable pair must outgrow its bake-floor grid at the
    // Normal tier — the condition the K-267 budget raise exists for.
    let grew = sp
        .iter()
        .zip(&baked.spreads)
        .any(|(need, b)| boost_grid(pair_grid(64, *b), *need) > pair_grid(64, *b));
    assert!(grew, "corner light must raise at least one pair's grid");
    // The boost never lowers the floor, honours its 3x cap, and stays in
    // the dispatchable range.
    assert_eq!(boost_grid(64, 1.0), 64);
    assert_eq!(boost_grid(64, 63.0), 64, "never below the bake floor");
    assert_eq!(boost_grid(64, 100.4), 100);
    assert_eq!(boost_grid(64, 4096.0), 192, "capped at 3x the rung grid");
    assert_eq!(boost_grid(360, 4096.0), 512, "hard 512 dispatch clamp");
    assert_eq!(boost_grid(4, 1.0), 8, "degenerate floor stays sane");
    // The budget plan never lowers a rung, spends at most the headroom,
    // and raises the pair that asked.
    let plan = plan_frame_grids(64, &baked.spreads, &sp);
    assert_eq!(plan.len(), pair_count);
    let mut baseline = 0u64;
    let mut spent = 0u64;
    for (pi, &g) in plan.iter().enumerate() {
        let rung = pair_grid(64, baked.spreads.get(pi).copied().unwrap_or(1.0));
        assert!(g >= rung, "pair {pi}: planned {g} under rung {rung}");
        assert!(g <= 512);
        baseline += u64::from(rung) * u64::from(rung);
        spent += u64::from(g) * u64::from(g);
    }
    assert!(
        spent as f64 <= baseline as f64 * (1.0 + f64::from(FRAME_RAY_HEADROOM)) + 512.0 * 512.0,
        "plan overspends: {spent} vs baseline {baseline}"
    );
    assert!(
        plan.iter()
            .enumerate()
            .any(|(pi, &g)| g > pair_grid(64, baked.spreads.get(pi).copied().unwrap_or(1.0))),
        "the corner frame must actually spend its headroom"
    );
    // The raw-rows entry is the same probe.
    let rows: Vec<[f32; 8]> = baked
        .surfaces
        .iter()
        .map(|s| {
            [
                s.radius_mm,
                s.z_mm,
                s.semi_ap_mm,
                s.cauchy_a,
                s.cauchy_b,
                s.coating_layers,
                s.is_stop,
                0.0,
            ]
        })
        .collect();
    let sp2 = frame_grid_needs_from_rows(
        &rows,
        &baked.pairs,
        baked.sensor_z_mm,
        baked.focal_mm,
        baked.pupil_mm,
        baked.start_z_mm,
        pair_count,
        corner,
        p.coating,
        stop_scale,
        shift,
    );
    assert_eq!(sp, sp2, "seam entry must be bit-identical");
}

/// The documented drop-on defaults, shared by the lens flare tests.
fn default_flare_params() -> crate::fx::lens_flare::LensFlareParams {
    crate::fx::lens_flare::LensFlareParams {
        // Raster pixels (K-260): tests divide by their own raster via
        // manual_light, so any sane point works; this is 0.33/0.30 of 96×54.
        light: [31.7, 16.2],
        // A point source, as the effect has always defaulted to, and no
        // comp lights — Manual mode never reads them.
        source_size: [0.0, 0.0],
        lights: [crate::fx::lens_flare::DEAD_LIGHT; crate::fx::lens_flare::MAX_SOURCES],
        light_count: 0,
        intensity: 1.0,
        lens: 16,
        fstop: 2.8,
        focus_m: 100.0,
        blades: 8,
        aperture_rotation_deg: 0.0,
        roundness: 0.15,
        aperture_softness: 0.05,
        ghost_intensity: 1.0,
        ghost_softness: 0.05,
        max_ghosts: 60,
        dispersion: 1.0,
        coating: 0.75,
        starburst_intensity: 1.0,
        scale: 1.0,
        source: 0,
        threshold: 1.0,
        threshold_softness: 0.25,
        light_tint: [1.0, 1.0, 1.0],
        use_source_colour: true,
        anamorphic: 1.0,
        quality: 1,
        detail: 1.0,
        blend: crate::fx::lens_flare::BLEND_ADD,
        mix: 1.0,
    }
}

// Matte-mode source detection (impl note §6, K-257): the CPU reference finds
// the brightest sources deterministically — brightest first, gated by the
// soft threshold, adjacent maxima suppressed — and the light carries the
// source pixel's colour times its gate weight.
#[test]
fn lens_flare_detects_matte_sources_deterministically() {
    use crate::fx::lens_flare::*;
    let (w, h) = (128u32, 96u32);
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    let mut put = |x: u32, y: u32, rgb: [f32; 3]| {
        let i = ((y * w + x) * 4) as usize;
        matte[i] = rgb[0];
        matte[i + 1] = rgb[1];
        matte[i + 2] = rgb[2];
        matte[i + 3] = 1.0;
    };
    // A bright white source, a dimmer warm one far away, and a neighbour 8 px
    // from the bright one that suppression must fold into it.
    put(20, 24, [4.0, 4.0, 4.0]);
    put(28, 24, [3.0, 3.0, 3.0]);
    put(100, 70, [1.5, 1.0, 0.5]);

    let lights = detect_lights(&matte, w, h, 1.0, 0.0, true, [1.0; 3]);
    assert_eq!(
        lights.len(),
        2,
        "the neighbour must be suppressed: {lights:?}"
    );
    // Brightest first — and since K-355 the light sits at the flux centre of
    // everything folded into it, not on its brightest pixel. Both pixels are
    // one lit region here, so the centre is between them weighted by
    // brightness: (20·4 + 28·3) / 7 = 164/7.
    let cx = 164.0 / 7.0;
    assert!(
        (lights[0].pos[0] - (cx + 0.5) / 128.0).abs() < 1e-6,
        "x {}",
        lights[0].pos[0] * 128.0 - 0.5
    );
    assert!((lights[0].pos[1] - 24.5 / 96.0).abs() < 1e-6);
    // …and its colour is the MEAN of the lit pixels, not the brightest one's:
    // (4 + 3) / 2. One sparkle can no longer define a source's colour.
    assert_eq!(lights[0].rgb, [3.5, 3.5, 3.5]);
    // The warm source keeps its colour.
    assert!((lights[1].pos[0] - 100.5 / 128.0).abs() < 1e-6);
    assert_eq!(lights[1].rgb, [1.5, 1.0, 0.5]);

    // The soft gate scales (K-363: luma 4 against a gate opening 3 → 5
    // lands half-way, 0.5), and a threshold above every source finds none —
    // including one AT a source's luma, which "brighter than" excludes.
    let gated = detect_lights(&matte, w, h, 3.0, 2.0, true, [1.0; 3]);
    assert!(!gated.is_empty());
    assert!(gated[0].rgb[0] < 4.0, "the gate must attenuate: {gated:?}");
    assert!(detect_lights(&matte, w, h, 10.0, 0.0, true, [1.0; 3]).is_empty());
    assert!(
        detect_lights(&matte, w, h, 4.0, 0.0, true, [1.0; 3]).is_empty(),
        "a threshold at the brightest source's own luma finds nothing:          the gate is 'brighter than', not 'at least'"
    );

    // Determinism: two runs agree bit-for-bit.
    assert_eq!(
        lights,
        detect_lights(&matte, w, h, 1.0, 0.0, true, [1.0; 3])
    );

    // The gate itself (K-363, one-sided): closed at and below the threshold,
    // open a softness above it. The two cases the owner asked for by name:
    // at threshold 1 only light brighter than 1 flares, and at threshold 0
    // anything brighter than black does — black itself never.
    assert_eq!(threshold_gate(0.99, 1.0, 0.0), 0.0);
    assert_eq!(
        threshold_gate(1.0, 1.0, 0.0),
        0.0,
        "at the line is not over it"
    );
    assert_eq!(threshold_gate(1.01, 1.0, 0.0), 1.0);
    assert_eq!(
        threshold_gate(1.0, 1.0, 0.5),
        0.0,
        "softness opens the gate above the threshold, never below or at it"
    );
    assert!(threshold_gate(1.25, 1.0, 0.5) > 0.0 && threshold_gate(1.25, 1.0, 0.5) < 1.0);
    assert_eq!(threshold_gate(1.5, 1.0, 0.5), 1.0);
    assert_eq!(threshold_gate(0.0, 0.0, 0.25), 0.0, "black never flares");
    assert!(
        threshold_gate(0.05, 0.0, 0.25) > 0.0,
        "at threshold 0, anything brighter than black flares"
    );
}

/// **A source spanning several tiles is found at the centre of its light, not
/// at one arbitrary pixel of it** (K-354).
///
/// The anchor used to be pinned to the brightest pixel of the brightest tile.
/// For a point source that is exactly right, and this test pins that it still
/// is. For a *practical* — a softbox, a window, a lamp with a visible bulb —
/// it put the light at whichever corner of the source the tile scan happened
/// to reach first, so a flare fired from a large soft source came out of its
/// edge rather than its middle.
///
/// Since K-355 every tile carries its own flux moments rather than one pixel's,
/// so the answer is the source's TRUE centre — and no single pixel, however
/// hot, can move it. That is what stops a flare jumping about inside a
/// practical as sensor noise shuffles which pixel happens to be brightest.
#[test]
fn lens_flare_centres_an_area_source_on_its_light() {
    use crate::fx::lens_flare::*;
    let (w, h) = (128u32, 128u32);
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    // A uniform square filling four whole 32-px tiles: x and y both 32..=95,
    // so its true centre is (63.5, 63.5).
    for y in 32..96u32 {
        for x in 32..96u32 {
            let i = ((y * w + x) * 4) as usize;
            matte[i] = 2.0;
            matte[i + 1] = 2.0;
            matte[i + 2] = 2.0;
            matte[i + 3] = 1.0;
        }
    }
    let lights = detect_lights(&matte, w, h, 1.0, 0.0, true, [1.0; 3]);
    assert_eq!(lights.len(), 1, "one source: {lights:?}");

    // Every lit pixel is weighed, so the answer is the square's exact centre
    // — (32 + 95) / 2 — rather than a corner of it, which is where this used
    // to land.
    let centre = 63.5f32;
    let px = lights[0].pos[0] * w as f32 - 0.5;
    let py = lights[0].pos[1] * h as f32 - 0.5;
    assert!((px - centre).abs() < 1e-3, "x {px}, want {centre}");
    assert!((py - centre).abs() < 1e-3, "y {py}, want {centre}");

    // **The jumping test.** One pixel of the source goes very hot, as sensor
    // sparkle does frame to frame. That pixel now owns the tile's `luma_max`,
    // so before K-355 it would have become the light's position outright — a
    // 30-pixel jump for a source that has not moved. Weighing every pixel
    // leaves the centre where it was, to well under a pixel.
    let mut sparkle = matte.clone();
    let i = ((34 * w + 34) * 4) as usize;
    for c in 0..3 {
        sparkle[i + c] = 40.0;
    }
    let jumped = detect_lights(&sparkle, w, h, 1.0, 0.0, true, [1.0; 3]);
    assert_eq!(jumped.len(), 1);
    let jx = jumped[0].pos[0] * w as f32 - 0.5;
    let jy = jumped[0].pos[1] * h as f32 - 0.5;
    assert!(
        (jx - px).abs() < 1.0 && (jy - py).abs() < 1.0,
        "one hot pixel moved the light from ({px}, {py}) to ({jx}, {jy})"
    );

    // And the source knows how big it is, which is what lets it be sampled
    // across rather than flared as a point (K-355).
    assert!(
        lights[0].extent[0] > 0.1,
        "a source 64 px wide in a 128 px frame must measure a real extent: \
         {:?}",
        lights[0].extent
    );
    assert!(
        area_samples(&lights[0], AREA_SAMPLES_MAX).len() > 1,
        "and must therefore be sampled across, not as one point"
    );

    // A one-pixel source has only itself to average, so point lights are
    // exactly where they always were.
    let mut dot = vec![0.0f32; (w * h * 4) as usize];
    let i = ((70 * w + 20) * 4) as usize;
    dot[i] = 4.0;
    dot[i + 1] = 4.0;
    dot[i + 2] = 4.0;
    dot[i + 3] = 1.0;
    let point = detect_lights(&dot, w, h, 1.0, 0.0, true, [1.0; 3]);
    assert_eq!(point.len(), 1);
    assert!((point[0].pos[0] - 20.5 / w as f32).abs() < 1e-6);
    assert!((point[0].pos[1] - 70.5 / h as f32).abs() < 1e-6);
}

/// **Light layers resolve, and an area light keeps its size** (K-360).
///
/// The whole reason the layer exists is the area kind: a light with a real
/// width and height flares as its own shape through the machinery K-355 built,
/// where a point can only ever be a dot. This pins the resolve — including that
/// only an area light reports extent, whatever the stored numbers say, and that
/// a light switched off is not a light (K-230's rule for every layer).
#[test]
fn lens_flare_light_layers_resolve_with_their_extent() {
    use crate::anim::Property;
    use crate::model::*;
    use crate::time::{CompTime, Duration, FrameRate, Rational};

    let mut comp = Composition {
        id: uuid::Uuid::now_v7(),
        name: "Scene".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(5, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        extra: serde_json::Map::new(),
    };

    let mut light_layer = |kind: LightKind, x: f64, half: f64, visible: bool| {
        let mut l = Layer {
            markers: Vec::new(),
            id: uuid::Uuid::now_v7(),
            name: "Light".into(),
            kind: LayerKind::Light {
                light: Box::new(LightDef {
                    kind,
                    half_size: [Property::fixed(half), Property::fixed(half * 0.5)],
                    ..LightDef::default()
                }),
            },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: TransformGroup {
                position_x: Property::fixed(x),
                position_y: Property::fixed(200.0),
                ..TransformGroup::default()
            },
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            retime: None,
            interpolation: Default::default(),
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        l.switches.visible = visible;
        comp.layers.push(l);
    };

    light_layer(LightKind::Area, 300.0, 80.0, true);
    light_layer(LightKind::Point, 900.0, 80.0, true);
    light_layer(LightKind::Area, 1500.0, 40.0, false);

    let lights = comp.lights_at(1.0);
    assert_eq!(lights.len(), 2, "a light switched off is not a light");

    // Top of the stack first — the order the effects that read lights take
    // them in, so a crowded frame spends its slots on the ones on top.
    assert_eq!(lights[0].position.0, 300.0);
    assert_eq!(lights[0].kind, LightKind::Area);
    assert_eq!(
        lights[0].half_size,
        (80.0, 40.0),
        "an area light reports its real size"
    );

    assert_eq!(lights[1].position.0, 900.0);
    assert_eq!(
        lights[1].half_size,
        (0.0, 0.0),
        "a point light has no extent, whatever the stored numbers say"
    );

    // Outside every span there are no lights at all.
    assert!(comp.lights_at(99.0).is_empty());

    // A default light is white at full intensity — a fresh one should light
    // something rather than land as a black source nobody can see.
    assert_eq!(lights[1].colour, [1.0, 1.0, 1.0]);
}

#[test]
fn zz_debug_cells() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        lens: 0,
        quality: 3,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (tier_base, _, _) = quality_ladder(p.quality);
    let base_side = detail_base(tier_base, p.detail);
    let pc = baked.pairs.len().min(p.max_ghosts as usize);
    let ss = fstop_scale(baked.native_fstop, p.fstop);
    let sh = focus_shift_mm(p.focus_m, baked.focal_mm);
    let (w, h) = (960u32, 540u32);
    let dir = light_direction([0.85, 0.78], h as f32 / w as f32, baked.focal_mm);
    let st = screen_transform(w);
    for (pi, pair) in baked.pairs.iter().take(pc).enumerate() {
        let spread = baked.spreads.get(pi).copied().unwrap_or(1.0);
        let side = pair_grid(base_side, spread) as usize;
        // trace the full grid, find max adjacent-corner distance among
        // rays whose weight is significant, plus landing bbox
        let mut pos = vec![None; side * side];
        for j in 0..side {
            for i in 0..side {
                let u = (i as f32 / (side - 1) as f32) * 2.0 - 1.0;
                let v = (j as f32 / (side - 1) as f32) * 2.0 - 1.0;
                if u * u + v * v > 1.1f32 {
                    continue;
                }
                let o = [
                    u * baked.pupil_mm * ss,
                    v * baked.pupil_mm * ss,
                    baked.start_z_mm,
                ];
                pos[j * side + i] = trace_splat(&baked, *pair, 550.0, o, dir, p.coating, ss, sh);
            }
        }
        let mut dmax = 0.0f32;
        let mut wmax = 0.0f32;
        let (mut bx0, mut bx1, mut by0, mut by1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for j in 0..side {
            for i in 0..side {
                if let Some((a, wa)) = pos[j * side + i] {
                    if wa > 1e-5 {
                        wmax = wmax.max(wa);
                        bx0 = bx0.min(a[0]);
                        bx1 = bx1.max(a[0]);
                        by0 = by0.min(a[1]);
                        by1 = by1.max(a[1]);
                        for (ni, nj) in [(i + 1, j), (i, j + 1)] {
                            if ni < side && nj < side {
                                if let Some((b, wb)) = pos[nj * side + ni] {
                                    if wb > 1e-5 {
                                        let d = (a[0] - b[0]).hypot(a[1] - b[1]);
                                        dmax = dmax.max(d);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if wmax > 1e-4 && dmax * st > 6.0 {
            eprintln!("pair {pi} {:?} spread {spread:.2} side {side} maxcell_px {:.0} bbox_px {:.0}x{:.0} wmax {wmax:.4}",
                pair, dmax * st, (bx1 - bx0) * st, (by1 - by0) * st);
        }
    }
}

// Every piece of schema metadata that names a parameter by string can rot:
// rename the parameter and the group or the enablement rule quietly stops
// matching anything, with no compiler to catch it. This sweeps the whole
// catalogue so a rename fails the build instead of silently un-grouping a
// twirl or un-greying a row.
#[test]
fn every_enablement_rule_names_a_parameter_of_its_kind() {
    for s in BUILTINS {
        let kind_of = |id: &str| s.params.iter().find(|p| p.id == id).map(|p| p.kind);

        for rule in s.enabled_when {
            assert!(
                kind_of(rule.param).is_some(),
                "{}: rule greys `{}`, which it does not declare",
                s.match_name,
                rule.param
            );
            let on = kind_of(rule.on).unwrap_or_else(|| {
                panic!(
                    "{}: rule reads `{}`, which it does not declare",
                    s.match_name, rule.on
                )
            });
            // A rule pointed at the wrong kind of parameter can never fire —
            // `param_enabled` leaves the row live rather than locking it
            // unreachably — so the mistake has to be caught here.
            match rule.cond {
                EnabledCond::BoolIs(_) => assert!(
                    matches!(on, ParamKind::Bool { .. }),
                    "{}: `{}` is read as a Bool but is not one",
                    s.match_name,
                    rule.on
                ),
                EnabledCond::ChoiceIs(i) | EnabledCond::ChoiceIsNot(i) => {
                    let ParamKind::Choice { options, .. } = on else {
                        panic!(
                            "{}: `{}` is read as a Choice but is not one",
                            s.match_name, rule.on
                        );
                    };
                    assert!(
                        (i as usize) < options.len(),
                        "{}: rule names option {i} of `{}`, which has {}",
                        s.match_name,
                        rule.on,
                        options.len()
                    );
                }
                EnabledCond::LayerSet => assert!(
                    matches!(on, ParamKind::Layer { .. }),
                    "{}: `{}` is read as a Layer reference but is not one",
                    s.match_name,
                    rule.on
                ),
            }
            assert_ne!(
                rule.param, rule.on,
                "{}: `{}` cannot gate itself",
                s.match_name, rule.param
            );
        }

        // K-145 requires a group's members to be a contiguous run of `params`,
        // because the twirl renders in place where its first member sits — a
        // gap would swallow whatever sat in it.
        for g in s.groups {
            let mut positions = g.params.iter().map(|id| {
                s.params
                    .iter()
                    .position(|p| p.id == *id)
                    .unwrap_or_else(|| {
                        panic!(
                            "{}: group `{}` names `{id}`, which it does not declare",
                            s.match_name, g.label
                        )
                    })
            });
            let first = positions
                .next()
                .unwrap_or_else(|| panic!("{}: group `{}` is empty", s.match_name, g.label));
            let mut prev = first;
            for pos in positions {
                assert_eq!(
                    pos,
                    prev + 1,
                    "{}: group `{}` is not a contiguous run of params",
                    s.match_name,
                    g.label
                );
                prev = pos;
            }
        }
    }
}

// Depth of field's folded parameter surface (K-313): the aperture, highlight
// and depth-map controls landed *inside* the shipped effect rather than beside
// it as a second one, so the surface itself is the thing under test — the order
// rows appear in, which twirl each sits behind, and above all the factory
// defaults, because every one of those has to be the value that leaves the
// effect rendering what it always rendered.
#[test]
fn dof_declares_the_folded_aperture_surface() {
    let s = schema("dof").unwrap();
    assert_eq!(s.label, "Depth of field");
    // There is no second effect: the aperture folded into this one.
    assert!(
        schema("bokeh").is_none(),
        "the aperture belongs to Depth of field, not to a second effect"
    );

    let ids: Vec<&str> = s.params.iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        vec![
            // Where focus IS: the layer, the number, the switch that takes it
            // over, and the point that takes over from it — adjacent, because a
            // toggle three twirls away from the row it governs reads as
            // unrelated to it.
            "depth",
            "focus",
            "use_focus_point",
            "focus_point_x",
            "focus_point_y",
            "range",
            "aperture",
            "near_aperture",
            "far_aperture",
            // Iris.
            "blades",
            "roundness",
            "rotation",
            "aspect",
            "rim",
            // Highlights.
            "threshold",
            "exposure",
            // Depth map: how the pass is READ.
            "depth_channel",
            "depth_invert",
            "gamma",
            "remove_edge_leak",
            "detect_edge_threshold",
            // Back out of the twirls.
            "repeat_edge_pixels",
            "display",
            "mix",
        ],
        "row order is what the panel draws"
    );

    let kind = |id: &str| s.params.iter().find(|p| p.id == id).unwrap().kind;
    let float_default = |id: &str| match kind(id) {
        ParamKind::Float { default, .. } => default,
        other => panic!("{id} is {other:?}, not a Float"),
    };

    // **Every added control is neutral at its default.** This is the fold's
    // whole licence: a project saved before any of them renders bit-identically,
    // which `the_default_aperture_is_the_historical_disc_bit_for_bit` in the
    // lumit-gpu tests pins on the pixels. Here it is pinned on the schema, so a
    // default drifting fails the build rather than silently re-rendering
    // everyone's work.
    assert_eq!(float_default("roundness"), 1.0, "1 is the circle");
    assert_eq!(float_default("aspect"), 0.0);
    assert_eq!(float_default("rim"), 0.0);
    assert_eq!(float_default("exposure"), 0.0, "0 is the plain mean");
    assert_eq!(float_default("gamma"), 0.0, "0 is a multiplier of 1");
    assert_eq!(float_default("remove_edge_leak"), 0.0);
    assert_eq!(float_default("detect_edge_threshold"), 0.10);
    assert_eq!(float_default("threshold"), 1.0, "scene white");
    // The historical rows keep their historical defaults.
    assert_eq!(float_default("focus"), 0.5);
    assert_eq!(float_default("range"), 0.1);
    assert_eq!(float_default("aperture"), 8.0);
    assert!(matches!(
        kind("repeat_edge_pixels"),
        ParamKind::Bool { default: true },
    ));
    assert!(matches!(
        kind("use_focus_point"),
        ParamKind::Bool { default: false },
    ));
    assert!(matches!(
        kind("rotation"),
        ParamKind::Angle { default: 0.0, .. }
    ));
    // Luminance: right for a grey depth map whatever channels it was written
    // to, and the shortlist has no entry that cannot explain itself.
    assert!(matches!(
        kind("depth_channel"),
        ParamKind::Choice {
            options: CHANNEL_OPTIONS,
            default: 0,
            ..
        }
    ));
    assert_eq!(CHANNEL_OPTIONS[0], "Luminance");

    // Roundness reaches concave — five blades at −1 is a star — which is why it
    // is not a 0..1 curvature.
    assert!(matches!(
        kind("roundness"),
        ParamKind::Float {
            hard: (Some(-1.0), Some(1.0)),
            ..
        }
    ));
    // The blade count is bounded by the kernel's uniform array in both
    // directions: below 3 there is no polygon, above MAX_BLADES there is no room
    // in the uniform. An Int, so a keyframe steps rather than growing half a
    // blade.
    assert!(matches!(
        kind("blades"),
        ParamKind::Int {
            default: 6,
            hard: (Some(3), Some(m)),
            ..
        } if m == MAX_BLADES as i64
    ));
    // The focus point is an `_x`/`_y` Float pair, which is the panel's point row
    // (docs/07 §6.1) — there is no Point schema kind and this is why one is not
    // needed. px@comp, open on both sides (K-260).
    for id in ["focus_point_x", "focus_point_y"] {
        assert!(matches!(
            kind(id),
            ParamKind::Float {
                hard: (None, None),
                ..
            }
        ));
    }
    assert!(matches!(
        kind("depth"),
        ParamKind::Layer {
            self_default: false
        }
    ));

    // The twirls, all closed by default: the rows above them are the effect and
    // these are how it is shaped.
    let labels: Vec<&str> = s.groups.iter().map(|g| g.label).collect();
    assert_eq!(labels, vec!["Iris", "Highlights", "Depth map"]);
    assert!(s.groups.iter().all(|g| g.collapsed));
    // Display sits OUTSIDE the twirls: a diagnostic is not part of the depth
    // plumbing, and tucking it away is the opposite of what it is for.
    assert!(s
        .groups
        .iter()
        .all(|g| !g.params.contains(&"display") && !g.params.contains(&"mix")));
    assert!(matches!(
        kind("display"),
        ParamKind::Choice {
            options: &["Rendered", "Depth map", "Focus map"],
            default: 0,
            ..
        }
    ));
}

// The greyed rows. `param_enabled` is the authority on the question and the
// panel draws from it, so the semantics are pinned here rather than left to the
// Dart side to rediscover.
#[test]
fn dof_greys_the_rows_its_switches_take_over() {
    let mut e = instantiate("dof").unwrap();

    // A fresh instance: no depth layer, so everything that reads one is greyed,
    // and Use focus point is off, so the point is greyed and the distance is
    // live.
    assert!(param_enabled(&e, "focus"));
    for id in [
        "depth_channel",
        "use_focus_point",
        "remove_edge_leak",
        "detect_edge_threshold",
    ] {
        assert!(
            !param_enabled(&e, id),
            "{id} needs a depth pass to mean anything"
        );
    }
    assert!(!param_enabled(&e, "focus_point_x"));
    assert!(!param_enabled(&e, "focus_point_y"));
    // Everything without a rule against it stays live, which is most rows.
    for id in ["aperture", "blades", "exposure", "roundness", "mix"] {
        assert!(param_enabled(&e, id), "{id} has no rule and must stay live");
    }

    // Picking a depth layer gives the depth rows their subject.
    set_layer(&mut e, "depth", Some(uuid::Uuid::now_v7()));
    assert!(param_enabled(&e, "depth_channel"));
    assert!(param_enabled(&e, "use_focus_point"));
    assert!(param_enabled(&e, "remove_edge_leak"));

    // Tick Use focus point and the two swap: the point decides, the number does
    // not.
    set_bool(&mut e, "use_focus_point", true);
    assert!(!param_enabled(&e, "focus"));
    assert!(param_enabled(&e, "focus_point_x"));
    assert!(param_enabled(&e, "focus_point_y"));
    set_bool(&mut e, "use_focus_point", false);
    assert!(param_enabled(&e, "focus"));
    assert!(!param_enabled(&e, "focus_point_x"));

    // Clearing the layer greys the depth rows again — a dangling reference reads
    // as unset.
    set_layer(&mut e, "depth", None);
    assert!(!param_enabled(&e, "depth_channel"));

    // An instance that predates the deciding parameter must not lock a row it
    // can never unlock: the rule cannot be judged, so it greys nothing. This is
    // the `backfill_builtin_params` trap from the other side.
    let mut old = instantiate("dof").unwrap();
    old.params.retain(|p| p.id != "use_focus_point");
    assert!(param_enabled(&old, "focus"));

    // An effect with no built-in schema at all (an OFX or placeholder instance)
    // has no rules, so nothing is greyed.
    let mut foreign = instantiate("dof").unwrap();
    foreign.effect.match_name = "not_a_builtin".to_owned();
    assert!(param_enabled(&foreign, "focus"));
}

// A fresh Depth of field focuses on the middle of the frame, the way a fresh
// Transform rotates about the middle (T23). The schema cannot know the raster,
// so the apply site fills it in; landing focus in the top-left corner would be
// exactly the §1.2 failure the raster-aware constructor exists to prevent.
#[test]
fn a_fresh_dof_focuses_on_the_middle_of_the_frame() {
    let e = instantiate_for_raster("dof", 1920.0, 1440.0).unwrap();
    assert_eq!(e.float_at("focus_point_x", 0.0), Some(960.0));
    assert_eq!(e.float_at("focus_point_y", 0.0), Some(720.0));

    // Plain `instantiate` keeps the pure schema default (nominal 1080p), which
    // is what presets and tests want.
    let pure = instantiate("dof").unwrap();
    assert_eq!(pure.float_at("focus_point_x", 0.0), Some(960.0));
    assert_eq!(pure.float_at("focus_point_y", 0.0), Some(540.0));
}

// The fold's contract in the resolve step: a saved instance that predates every
// added control resolves to exactly the op the effect always produced, with each
// new field at the value the kernel branches around.
#[test]
fn a_legacy_dof_resolves_to_the_neutral_aperture() {
    let mut legacy = instantiate("dof").unwrap();
    // Strip everything the fold added, as a project saved before it would be.
    legacy.params.retain(|p| {
        !matches!(
            p.id.as_str(),
            "blades"
                | "roundness"
                | "rotation"
                | "aspect"
                | "rim"
                | "threshold"
                | "exposure"
                | "depth_channel"
                | "use_focus_point"
                | "focus_point_x"
                | "focus_point_y"
                | "gamma"
                | "remove_edge_leak"
                | "detect_edge_threshold"
                | "repeat_edge_pixels"
        )
    });
    let r = resolve_stack(
        std::slice::from_ref(&legacy),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(r, vec![resolved_dof(8.0, 8.0, [0.0, 0.0])]);
}

// **The fold's load-bearing promise** (K-313): at the shipped defaults the
// gather computes exactly the box-weighted disc average this effect computed
// before it grew an aperture, a tonal mean or a weighting — to the bit, not to a
// tolerance.
//
// That is the whole licence for folding the Bokeh control surface *into* Depth
// of field rather than shipping it beside it as a second effect, so it is pinned
// on the arithmetic rather than asserted in a comment. The reference below is
// the historical kernel written out longhand; both sides are f32, so the
// comparison is exact equality and not a ULP bound. Any drift in the branches
// that skip the weighting, the split or the polygon test fails here.
#[test]
fn the_default_aperture_is_the_historical_disc_bit_for_bit() {
    let (w, h) = (24u32, 18u32);
    let (wi, hi) = (w as i32, h as i32);
    let n = (w * h) as usize;

    // A picture with real structure, and highlights above the 1.0 threshold so
    // the tonal branch would show if it were taken.
    let mut img = vec![0.0f32; n * 4];
    let mut depth = vec![0.0f32; n * 4];
    for y in 0..hi {
        for x in 0..wi {
            let i = (y * wi + x) as usize;
            let t = (x as f32 * 0.37 + y as f32 * 0.11).sin() * 0.5 + 0.5;
            img[i * 4] = t * 3.0;
            img[i * 4 + 1] = 1.0 - t;
            img[i * 4 + 2] = t * t;
            img[i * 4 + 3] = 1.0;
            // A left-to-right ramp, so the circle of confusion sweeps its whole
            // range across the frame.
            depth[i * 4] = x as f32 / (wi - 1) as f32;
            depth[i * 4 + 3] = 1.0;
        }
    }

    let (focus, range, near, far, mix) = (0.5f32, 0.1f32, 6.0f32, 6.0f32, 1.0f32);
    let (blade_normals, apothem2) = crate::fx::aperture_blades(6, 0.0);
    let p = cpu::DofParams {
        focus,
        range,
        near_aperture: near,
        far_aperture: far,
        blade_normals,
        blade_count: 6,
        apothem2,
        roundness: 1.0,
        rim: 0.0,
        aspect_scale: [1.0, 1.0],
        threshold: 1.0,
        bokeh_power: 1.0,
        repeat_edge: true,
        // Red explicitly: this test pins the GATHER, and the depth below is
        // written to red alone. Which channel is read by default is a different
        // question, asked in `dof_declares_the_folded_aperture_surface`.
        depth_channel: 2,
        depth_invert: false,
        use_focus_point: false,
        focus_point: [0.0, 0.0],
        gamma: 1.0,
        remove_edge_leak: 0.0,
        detect_edge_threshold: 0.1,
        display: 0,
        mix,
    };
    let mut got = img.clone();
    cpu::dof(&mut got, Some(&depth), w, h, &p);

    // The historical kernel, longhand: smoothstep ramp, per-side aperture,
    // box-weighted integer disc, edges clamped, `o*(1-mix) + v*mix`.
    let mut want = img.clone();
    for y in 0..hi {
        for x in 0..wi {
            let pi = (y * wi + x) as usize;
            let d = depth[pi * 4];
            let dist = (d - focus).abs();
            let denom = (1.0f32 - range).max(1e-4);
            let e = ((dist - range) / denom).clamp(0.0, 1.0);
            let s = e * e * (3.0 - 2.0 * e);
            let ap = if d < focus { near } else { far };
            let coc = ap * s;
            let coc2 = coc * coc;
            let ri = coc.ceil() as i32;
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let r2 = (dx * dx + dy * dy) as f32;
                    if r2 <= coc2 {
                        let sx = (x + dx).clamp(0, wi - 1);
                        let sy = (y + dy).clamp(0, hi - 1);
                        let si = ((sy * wi + sx) * 4) as usize;
                        for c in 0..4 {
                            acc[c] += img[si + c];
                        }
                        wsum += 1.0;
                    }
                }
            }
            for c in 0..4 {
                let v = acc[c] / wsum;
                want[pi * 4 + c] = img[pi * 4 + c] * (1.0 - mix) + v * mix;
            }
        }
    }

    assert_eq!(
        got, want,
        "the shipped defaults must reproduce the historical disc bit for bit"
    );

    // And each control on its own really does change the picture, so the
    // equality above is a property of the neutrals rather than of a gather that
    // ignores them.
    for changed in [
        cpu::DofParams {
            roundness: 0.0,
            ..p
        },
        cpu::DofParams { rim: 0.7, ..p },
        cpu::DofParams {
            threshold: 0.5,
            bokeh_power: 4.0,
            ..p
        },
        cpu::DofParams {
            aspect_scale: [1.0, 2.0],
            roundness: 0.0,
            ..p
        },
    ] {
        let mut other = img.clone();
        cpu::dof(&mut other, Some(&depth), w, h, &changed);
        assert_ne!(other, want, "a shaped aperture must change the picture");
    }
}

fn set_bool(e: &mut EffectInstance, id: &str, v: bool) {
    for p in &mut e.params {
        if p.id == id {
            p.value = EffectValue::Bool(v);
        }
    }
}

fn set_layer(e: &mut EffectInstance, id: &str, v: Option<uuid::Uuid>) {
    for p in &mut e.params {
        if p.id == id {
            p.value = EffectValue::Layer(v);
        }
    }
}

// The aperture's two load-bearing geometric claims, because the kernel's scan
// box depends on both and neither is obvious from the formula.
//
// **It stays inscribed in the circle at every setting.** The gather scans a
// `ceil(coc)` box and tests each integer offset; that box is only a correct
// bound if no accepted tap lies outside the circle of radius `coc`. Roundness
// reaching below zero and Deform squeezing an axis both had to preserve that,
// and a change that broke it would not fail the oracle — both paths would
// simply miss the same taps — so it is pinned here instead.
//
// **Negative Roundness really is a star.** The vertices stay on the circle while
// the edge midpoints pull in, which is what makes the shape a star rather than
// just a smaller polygon.
#[test]
fn the_dof_aperture_stays_inside_its_circle() {
    let coc = 12.0f32;
    let coc2 = coc * coc;
    let ri = coc.ceil() as i32;

    for sides in [3u32, 5, 6, 8] {
        let (blade_normals, apothem2) = aperture_blades(sides, 17.0);
        for roundness in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            for deform in [[1.0f32, 1.0], [2.0, 1.0], [1.0, 3.0]] {
                let p = cpu::DofParams {
                    focus: 0.5,
                    range: 0.0,
                    near_aperture: coc,
                    far_aperture: coc,
                    blade_normals,
                    blade_count: sides,
                    apothem2,
                    roundness,
                    rim: 0.0,
                    aspect_scale: deform,
                    threshold: 0.0,
                    bokeh_power: 1.0,
                    repeat_edge: true,
                    depth_channel: 5,
                    depth_invert: false,
                    use_focus_point: false,
                    focus_point: [0.0, 0.0],
                    gamma: 1.0,
                    remove_edge_leak: 0.0,
                    detect_edge_threshold: 0.1,
                    display: 0,
                    mix: 1.0,
                };
                let mut accepted = 0;
                for dy in -ri..=ri {
                    for dx in -ri..=ri {
                        if cpu::dof_tap_inside(dx as f32, dy as f32, coc2, &p) {
                            accepted += 1;
                            let r2 = (dx * dx + dy * dy) as f32;
                            assert!(
                                r2 <= coc2 + 1e-3,
                                "n{sides} roundness {roundness} deform {deform:?}: \
                                 tap ({dx},{dy}) is outside the circle of confusion, \
                                 so ceil(coc) no longer bounds the gather"
                            );
                        }
                    }
                }
                // The centre tap is always in, which is what keeps the running
                // weight non-zero at any radius.
                assert!(accepted > 0);
                assert!(cpu::dof_tap_inside(0.0, 0.0, coc2, &p));
            }
        }
    }

    // The star property, on a hexagon with a vertex placed on the +x axis so the
    // two directions are exactly where they are expected. `aperture_blades`
    // puts an edge normal at `rotation`, so rotating by half a step (30° for
    // six sides) moves a vertex there instead.
    let (blade_normals, apothem2) = aperture_blades(6, 30.0);
    let star = |roundness: f32| cpu::DofParams {
        focus: 0.5,
        range: 0.0,
        near_aperture: coc,
        far_aperture: coc,
        blade_normals,
        blade_count: 6,
        apothem2,
        roundness,
        rim: 0.0,
        aspect_scale: [1.0, 1.0],
        threshold: 0.0,
        bokeh_power: 1.0,
        repeat_edge: true,
        depth_channel: 5,
        depth_invert: false,
        use_focus_point: false,
        focus_point: [0.0, 0.0],
        gamma: 1.0,
        remove_edge_leak: 0.0,
        detect_edge_threshold: 0.1,
        display: 0,
        mix: 1.0,
    };
    // How far the aperture reaches along a ray, by bisection on the inside test.
    let reach = |p: &cpu::DofParams, ux: f32, uy: f32| {
        let (mut lo, mut hi) = (0.0f32, coc * 1.5);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if cpu::dof_tap_inside(ux * mid, uy * mid, coc2, p) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    // Which direction happens to be a vertex depends on the rotation phase, so
    // the extremes are found rather than assumed: the farthest direction is a
    // vertex and the nearest is an edge midpoint, whatever the phase.
    let extremes = |p: &cpu::DofParams| {
        let (mut lo, mut hi) = (f32::MAX, 0.0f32);
        for i in 0..360 {
            let a = (i as f32).to_radians();
            let r = reach(p, a.cos(), a.sin());
            lo = lo.min(r);
            hi = hi.max(r);
        }
        (lo, hi)
    };

    let (poly_min, poly_max) = extremes(&star(0.0));
    let (star_min, star_max) = extremes(&star(-1.0));
    let (round_min, round_max) = extremes(&star(1.0));

    // The vertices sit on the circle at any roundness: along a vertex both terms
    // carry the same k²r², so the test collapses to r ≤ coc whatever the
    // coefficient. This is what keeps the aperture inscribed rather than merely
    // small.
    assert!(
        (poly_max - coc).abs() < 0.05 && (star_max - coc).abs() < 0.05,
        "a vertex must reach the circle at any roundness: {poly_max} vs {star_max}"
    );
    // A plain polygon's nearest point is its apothem, k·coc.
    let apothem = coc * apothem2.sqrt();
    assert!(
        (poly_min - apothem).abs() < 0.05,
        "roundness 0 must be the inscribed polygon: {poly_min} vs {apothem}"
    );
    // Negative roundness pulls the edge midpoints in past the apothem, which is
    // the difference between a star and a smaller hexagon.
    assert!(
        star_min < poly_min * 0.95,
        "negative roundness must pinch the edge midpoints in ({star_min} vs {poly_min})"
    );

    // Roundness 1 is the circle, which is why the reference panel needs no
    // Circle entry: every direction reaches the same distance.
    assert!(
        (round_max - round_min).abs() < 0.05 && (round_max - coc).abs() < 0.05,
        "roundness 1 must be the circle: {round_min} to {round_max}"
    );
}

// The defocus ramp is continuous in depth — a value nearer the focus depth is
// never blurred more than one further from it, and there are no steps.
//
// **The regression this guards.** Resolution was read as a band count that
// quantised this ramp into (by default) seven levels. On a real depth pass —
// nearly all the content in a narrow band, one near object well outside it —
// that put the entire band in level zero and the object in level five, so focus
// was all-or-nothing and no shape of ramp could recover it. The guess is
// withdrawn (docs/08 §3.27); this is what stops it coming back by accident.
#[test]
fn the_defocus_ramp_is_continuous_in_depth() {
    let focus = 0.78f32;
    // However many bands are asked for, the ramp does not step.
    for bands in [1.0f32, 6.0, 32.0] {
        let mut seen: Vec<f32> = Vec::new();
        for i in 0..=200 {
            let d = i as f32 / 200.0;
            let r = cpu::dof_falloff(d, focus, 0.0, 1.0);
            if !seen.iter().any(|s| (s - r).abs() < 1e-7) {
                seen.push(r);
            }
        }
        assert!(
            seen.len() > 100,
            "the ramp must be continuous, got {} distinct levels at bands {bands}",
            seen.len()
        );
    }

    // The narrow band a real depth pass occupies must separate rather than move
    // as one — the complaint that found this.
    let scene = [0.70f32, 0.74, 0.78, 0.82, 0.86];
    let mut levels: Vec<f32> = scene
        .iter()
        .map(|d| cpu::dof_falloff(*d, focus, 0.0, 1.0))
        .collect();
    levels.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    assert!(
        levels.len() >= 3,
        "a narrow depth band must not collapse to one blur amount: {levels:?}"
    );

    // In focus is exactly zero, and the ramp still reaches full blur.
    assert_eq!(cpu::dof_falloff(focus, focus, 0.0, 1.0), 0.0);
    assert_eq!(cpu::dof_falloff(1.0, 0.0, 0.0, 1.0), 1.0);
}

#[test]
fn profile_moves_the_focus_transition_where_the_content_is() {
    // A scene band around 0.75 and a near object at 0.05 — the shape a real
    // depth pass has.
    let focus = 0.78;
    let scene = [0.70f32, 0.75, 0.80, 0.85];
    let near = 0.05f32;

    // At the neutral falloff the scene is essentially untouched and the near
    // object is essentially gone: the two ends, nothing between them.
    let plain_scene: Vec<f32> = scene.iter().map(|d| ramp_at(*d, focus, 1.0)).collect();
    let plain_near = ramp_at(near, focus, 1.0);
    assert!(
        plain_scene.iter().all(|s| *s < 0.05),
        "the scene band should barely blur at the neutral falloff: {plain_scene:?}"
    );
    assert!(
        plain_near > 0.8,
        "the near object is already all but gone: {plain_near}"
    );

    // Tightening it (a positive Profile) brings the transition into the scene
    // band itself, so the scene now separates front-to-back instead of moving
    // as one.
    let tight = (2.0f32 * 0.8).exp2();
    let tight_scene: Vec<f32> = scene.iter().map(|d| ramp_at(*d, focus, tight)).collect();
    let spread = tight_scene.iter().cloned().fold(0.0f32, f32::max)
        - tight_scene.iter().cloned().fold(1.0f32, f32::min);
    let plain_spread = plain_scene.iter().cloned().fold(0.0f32, f32::max)
        - plain_scene.iter().cloned().fold(1.0f32, f32::min);
    assert!(
        spread > plain_spread * 2.0,
        "a tighter profile must separate the scene band, got {tight_scene:?}"
    );

    // Loosening it (a negative Profile) softens the near object instead of
    // obliterating it — the far extreme is no longer automatically full blur.
    let loose = (-2.0f32).exp2();
    let loose_near = ramp_at(near, focus, loose);
    assert!(
        loose_near < plain_near * 0.5,
        "a looser profile must soften rather than obliterate: {loose_near} vs {plain_near}"
    );
    assert!(loose_near > 0.0, "and it must still blur something");

    // The ramp is monotone in depth distance at every falloff — a further thing
    // is never sharper than a nearer one.
    for falloff in [0.25f32, 1.0, 4.0] {
        let mut prev = -1.0f32;
        for i in 0..=100 {
            let d = focus + i as f32 / 100.0;
            let s = ramp_at(d.min(1.0), focus, falloff);
            assert!(
                s >= prev - 1e-6,
                "the ramp must not go back down at {falloff}"
            );
            prev = s;
        }
    }
}

fn ramp_at(d: f32, focus: f32, falloff: f32) -> f32 {
    cpu::dof_falloff(d, focus, 0.0, falloff)
}

// Profile must reach far enough for a depth pass whose content is squeezed into
// a fraction of its range — which is what a real one looks like.
//
// **The case that set the range.** A linear depth channel off game footage put
// the sky at 1.0 and compressed an entire room into 0.0–0.2, so the depth
// differences that matter were a tenth of the range. At the original ±1 the
// control could compress the falloff only fourfold, and focus stayed
// all-or-nothing however it was set: the room moved as one block whichever end
// it was focused on.
#[test]
fn profile_reaches_a_depth_pass_squeezed_into_a_fifth_of_its_range() {
    // The distribution measured off the owner's footage through the Focus map:
    // the room in the bottom fifth, the ceiling pinned at the far end.
    let room = [0.02f32, 0.06, 0.10, 0.14, 0.18];
    let ceiling = 1.0f32;
    let focus = 0.10f32;

    let spread = |falloff: f32| {
        let levels: Vec<f32> = room
            .iter()
            .map(|d| cpu::dof_falloff(*d, focus, 0.0, falloff))
            .collect();
        levels.iter().cloned().fold(0.0f32, f32::max)
            - levels.iter().cloned().fold(1.0f32, f32::min)
    };

    // At the neutral falloff the whole room is effectively one blur amount —
    // the complaint.
    assert!(
        spread(1.0) < 0.05,
        "the neutral falloff cannot separate this pass: {}",
        spread(1.0)
    );
    // The old ceiling (fourfold) still could not.
    assert!(
        spread(4.0) < 0.4,
        "fourfold was not enough, which is why the range widened: {}",
        spread(4.0)
    );
    // 64× — Profile 6 on the current scale, the middle of the slider — spreads
    // the room across most of the range, which is what makes the control usable
    // on real depth.
    assert!(
        spread(64.0) > 0.9,
        "the widened range must separate the room: {}",
        spread(64.0)
    );

    // And the far extreme is still all but fully blurred at every setting —
    // widening the reach must not cost the background its defocus.
    for falloff in [1.0f32, 4.0, 64.0] {
        let far = cpu::dof_falloff(ceiling, focus, 0.0, falloff);
        assert!(
            far > 0.95,
            "the background must stay defocused at {falloff}: {far}"
        );
    }

    // The schema must actually offer the range the maths needs.
    let s = schema("dof").unwrap();
    let profile = s.params.iter().find(|p| p.id == "gamma").unwrap();
    assert!(matches!(
        profile.kind,
        ParamKind::Float {
            hard: (Some(-10.0), Some(10.0)),
            ..
        }
    ));
    // And the scale must put that 64× in the middle of the slider, not at its
    // end: one doubling per unit means Profile 6.
    assert!(((6.0f32).exp2() - 64.0).abs() < 1e-3);
}

/// K-321: an instance may carry the user's own name. `None` — every older
/// project — serialises to nothing at all, so documents without the feature
/// are byte-for-byte unchanged, and a named instance round-trips exactly.
#[test]
fn custom_name_roundtrips_and_defaults_to_none() {
    let e = instantiate("blur").unwrap();
    assert_eq!(e.custom_name, None);
    let bare = serde_json::to_string(&e).unwrap();
    assert!(
        !bare.contains("custom_name"),
        "an unnamed instance writes no field, so older files are unchanged"
    );
    let back: EffectInstance = serde_json::from_str(&bare).unwrap();
    assert_eq!(
        back.custom_name, None,
        "a file without the field reads None"
    );

    let mut named = e;
    named.custom_name = Some("Blur the sign".into());
    let json = serde_json::to_string(&named).unwrap();
    let back: EffectInstance = serde_json::from_str(&json).unwrap();
    assert_eq!(back.custom_name.as_deref(), Some("Blur the sign"));
}

/// TEMPORARY perf probe (not part of the suite): times a bake per library
/// lens so the 23-second report can be reproduced or ruled out. Run with
/// `cargo test -p lumit-core --release bake_timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn bake_timing_probe() {
    for lens in 0..crate::fx::lens_library::LENS_LIBRARY.len() as u32 {
        let p = crate::fx::lens_flare::LensFlareParams {
            lens,
            ..default_flare_params()
        };
        let t = std::time::Instant::now();
        let baked = crate::fx::lens_flare::bake(&p);
        eprintln!(
            "lens {lens:2} ({}): {:6.1} ms, {} surfaces, {} pairs",
            crate::fx::lens_flare::lens_entry(lens).name,
            t.elapsed().as_secs_f64() * 1000.0,
            baked.surfaces.len(),
            baked.pairs.len(),
        );
    }
}

/// **Spectral radiometry preserves exposure and actually resolves the
/// coating** (K-364, entry A2). Two halves:
///
/// The bands' sub-weights sum to what `lambda_weights` gave each whole band
/// — XYZ→RGB is linear, so splitting the CIE integral must split the RGB
/// weight exactly. If this drifts, every flare changes brightness on a
/// change that promised colour accuracy only.
///
/// And a coated ghost's band-integrated energy must differ from the old
/// band-centre sample — the whole point: a 7-layer stack's reflectance
/// oscillates inside one band, and one sample per band cannot see it.
#[test]
fn spectral_bands_preserve_exposure_and_resolve_the_coating() {
    use crate::fx::lens_flare::*;
    for count in [3u32, 8, 16] {
        let old = lambda_weights(count, 1.0);
        let new = spectral_bands(count, 1.0);
        assert_eq!(old.len(), new.len());
        for (k, (o, n)) in old.iter().zip(&new).enumerate() {
            assert!(
                (o.0 - n.traced_nm).abs() < 1e-4,
                "geometry ladder unchanged"
            );
            // In XYZ the subs sum exactly to the band mean; in RGB the
            // out-of-gamut clamp now applies per sub-sample rather than per
            // band, and Σ max(xᵢ, 0) ≥ max(Σ xᵢ, 0) — so every channel is
            // AT LEAST the old weight (violet bands clamp G and R), and a
            // band no clamp touches is exact. "Strictly less thrown away"
            // is the property; never-dimmer is its testable face.
            let mut any_exact = false;
            for c in 0..3 {
                let sum: f32 = n.sub_rgb.iter().map(|s| s[c]).sum();
                assert!(
                    sum + 1e-3 >= o.1[c],
                    "band {k} channel {c}: spectral must never be dimmer                      ({sum} vs {})",
                    o.1[c]
                );
                if (sum - o.1[c]).abs() < 2e-3 {
                    any_exact = true;
                }
            }
            assert!(
                any_exact,
                "band {k}: at least one channel must match the old weight                  exactly — every channel drifting means the normalisation                  changed, not the clamp"
            );
        }
    }

    // A coated lens, one ghost, one off-axis ray: spectral vs band-centre.
    let p = LensFlareParams {
        lens: 16, // Zeiss Master Prime: modern multi-layer coatings
        ..default_flare_params()
    };
    let baked = bake(&p);
    let bands = spectral_bands(3, 1.0);
    let old_weights = lambda_weights(3, 1.0);
    let dir = light_direction([0.3, 0.3], 0.5625, baked.focal_mm);
    let mut spectral_differs = false;
    let mut compared = 0u32;
    // Several ghosts and pupil points: any one surviving ray on a coated
    // path is enough to show the band centre under-resolves the stack.
    for pair in baked.pairs.iter().take(8) {
        for frac in [0.1_f32, 0.3, 0.5] {
            let origin = [
                baked.pupil_mm * frac,
                baked.pupil_mm * frac * 0.5,
                baked.start_z_mm,
            ];
            for (band, old) in bands.iter().zip(&old_weights) {
                let Some((_, _, rgb)) =
                    trace_splat_spectral(&baked, *pair, band, origin, dir, 1.0, 1.0, 0.0)
                else {
                    continue;
                };
                let Some((_, w)) =
                    trace_splat(&baked, *pair, band.traced_nm, origin, dir, 1.0, 1.0, 0.0)
                else {
                    continue;
                };
                if w <= 1e-9 {
                    continue;
                }
                compared += 1;
                for (new_c, old_w) in rgb.iter().zip(old.1) {
                    let old_c = old_w * w;
                    if old_c > 1e-8 && (new_c - old_c).abs() / old_c > 0.02 {
                        spectral_differs = true;
                    }
                }
            }
        }
    }
    assert!(compared > 0, "no ray survived; the probe geometry is wrong");
    assert!(
        spectral_differs,
        "on a multi-coated lens the band-integrated energy must differ from \
         the band-centre sample — otherwise A2 resolved nothing"
    );

    // Determinism: the same band twice is the same bits.
    let probe_origin = [baked.pupil_mm * 0.3, 0.0, baked.start_z_mm];
    let a = trace_splat_spectral(
        &baked,
        baked.pairs[0],
        &bands[1],
        probe_origin,
        dir,
        0.7,
        1.0,
        0.0,
    );
    let b = trace_splat_spectral(
        &baked,
        baked.pairs[0],
        &bands[1],
        probe_origin,
        dir,
        0.7,
        1.0,
        0.0,
    );
    assert_eq!(a, b);
}
