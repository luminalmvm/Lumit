use std::sync::Arc;

use super::*;
use crate::anim::{Animation, Property};
use crate::expression::ExpressionContext;
use crate::model::{Composition, EffectInstance, EffectNamespace, EffectValue, Layer};
use crate::time::Rational;

// These tests are about *parameter resolution*, not about expressions, so they
// call the resolvers without an expression context and get the detached one.
// Shadowing the two entry points here keeps that out of every call below —
// otherwise the same argument would be spelled out ninety times. They also
// reduce the resolved stack to its `Shape`: which effects ran, in what order,
// with what numbers, which is what the ordering assertions below compare. A test
// about one effect's own numbers reads them back through `resolve_migrated`.

/// One resolved op as the ordering assertions read it: its match name and the
/// bag it resolved to.
type ShapedOp = (&'static str, Vec<(ParamId, Value)>);

/// A whole resolved stack in that form. Comparing this compares the *whole*
/// stack — which the old `Vec<Resolved>` stopped doing the moment the last
/// effect's numbers moved into the arena and left one variant behind it.
type Shape = Vec<ShapedOp>;

fn shape(stack: &ResolvedStack) -> Shape {
    stack
        .iter()
        .map(|fx| {
            (
                fx.def.schema().match_name,
                fx.params.iter().collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Shape {
    shape(&super::resolve_stack(
        effects,
        lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    ))
}

fn resolve_stack_temporal(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Shape {
    shape(&super::resolve_stack_temporal(
        effects,
        sample_lt,
        frame_lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    ))
}

/// Resolve a one-effect stack whose effect has moved to the registry, and read
/// its bag back through the effect's own typed reader
/// (docs/impl/effect-registry.md §3).
///
/// The assertions these tests used to make — "100 % resolves to a factor of 1"
/// — are now assertions about the effect's `packed`, because that is where the
/// conversion moved; the resolve step's job is to put the *authored* number in
/// the bag, which this checks on the way past.
fn resolve_migrated<T: EffectMetadata>(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> T {
    let ops = super::resolve_stack(
        effects,
        lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    );
    assert_eq!(ops.len(), 1, "expected exactly one resolved op");
    T::read(ops.get(0).expect("the migrated op").params)
}

/// The resolved bag of a one-effect migrated stack, in push order — for the
/// assertions that are about the *bag* rather than about the declared struct: a
/// derived value, or how many entries an effect resolves to at all.
fn resolve_bag(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<(ParamId, Value)> {
    let ops = super::resolve_stack(
        effects,
        lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    );
    assert_eq!(ops.len(), 1, "expected exactly one resolved op");
    ops.get(0).expect("the migrated op").params.iter().collect()
}

/// What a Shake instance hands its dispatch at `lt`: the wobble (or the whole
/// motion-blur set) the old `Resolved::Shake` variant carried, now the declared
/// rows and the resolve-time derivation read back through the
/// effect's own `packed`.
fn shake_packed(e: &EffectInstance, lt: f64, diag_px: f32) -> effects::shake::Shaken {
    shake_packed_scaled(e, lt, diag_px, 1.0)
}

/// [`shake_packed`] with the §2.3 preview factor in play.
fn shake_packed_scaled(
    e: &EffectInstance,
    lt: f64,
    diag_px: f32,
    px_scale: f32,
) -> effects::shake::Shaken {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        lt,
        diag_px,
        px_scale,
        &MarkerContext::NONE,
    );
    let p = Params::new(&bag);
    effects::shake::Shake::read(p).packed(effects::shake::Shake::derived_of(p))
}

/// A resolved stack holding one shake whose wobble is exactly `wobble` (and, for
/// the motion-blur cases, exactly `mb`) — the hand-built bag the CPU-reference
/// tests need, since a *resolved* wobble is whatever the noise says.
///
/// The trick is the effect's own arithmetic: `packed` builds each offset as
/// `amplitude · axis amount · noise`, so amplitudes of exactly 1 make the
/// unit-free noise vector *be* the wobble, and `zoom = 1 + z · noise` makes the
/// z component `zoom - 1` (an exact f32 subtraction near 1, so it round-trips).
fn shake_stack(
    wobble: ShakeSample,
    edge: u32,
    mix_pct: f32,
    mb: Option<[ShakeSample; SHAKE_MB_SAMPLES]>,
) -> ResolvedStack {
    let noise = |s: ShakeSample| {
        Value::Vec4([s.offset_px[0], s.offset_px[1], s.rotation_deg, s.zoom - 1.0])
    };
    let mut ops = ResolvedStack::new();
    ops.begin(&effects::shake::ShakeDef, Uuid::now_v7());
    for (id, v) in [
        (effects::shake::Shake::AMPLITUDE, 1.0),
        (effects::shake::Shake::X_AMP, 1.0),
        (effects::shake::Shake::Y_AMP, 1.0),
        (effects::shake::Shake::ROTATION, 1.0),
        (effects::shake::Shake::MIX, mix_pct),
    ] {
        ops.push(id, Value::Float(v));
    }
    ops.push(effects::shake::Shake::DERIVED_Z_AMP, Value::Float(1.0));
    ops.push(effects::shake::Shake::DERIVED_EDGE, Value::Choice(edge));
    ops.push(effects::shake::Shake::DERIVED_NOISE, noise(wobble));
    if let Some(samples) = mb {
        for (id, s) in effects::shake::Shake::DERIVED_MB_NOISE
            .iter()
            .zip(samples.iter())
        {
            ops.push(*id, noise(*s));
        }
    }
    ops
}

/// What a Flash instance hands its kernel at `lt`: the `(strength, colour, mix)`
/// the old `Resolved::Flash` variant carried, now the resolve-time derivation
/// read back through the effect's own `packed`.
fn flash_packed(e: &EffectInstance, lt: f64, markers: &MarkerContext) -> (f32, [f32; 4], f32) {
    let bag = resolve_bag(std::slice::from_ref(e), lt, 1000.0, 1.0, markers);
    let p = Params::new(&bag);
    effects::flash::Flash::read(p).packed(effects::flash::Flash::strength_of(p))
}

/// What a Lens flare op hands the bake and the kernels: the
/// [`LensFlareParams`](crate::fx::lens_flare::LensFlareParams) bundle the old
/// `Resolved::LensFlare` variant carried, read back out of the resolved arena
/// through the effect's own `packed` — Lights mode's sources included, since they
/// are a resolve-time derivation rather than a row.
fn flare_packed(ops: &super::ResolvedStack) -> crate::fx::lens_flare::LensFlareParams {
    let fx = ops.get(0).expect("the flare op");
    let (lights, count) = effects::lens_flare::LensFlare::lights_of(fx.params);
    effects::lens_flare::LensFlare::read(fx.params).packed(lights, count)
}

/// What a Scanlines instance hands its kernel: the fields the old
/// `Resolved::Scanlines` variant carried, with the folded intensity and the roll
/// offset coming out of the resolve-time derivation.
fn scanlines_packed(
    e: &EffectInstance,
    lt: f64,
    diag_px: f32,
    px_scale: f32,
) -> (f32, f32, f32, bool, f32) {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        lt,
        diag_px,
        px_scale,
        &MarkerContext::NONE,
    );
    let p = Params::new(&bag);
    let (i, r) = effects::scanlines::Scanlines::derived_of(p);
    effects::scanlines::Scanlines::read(p).packed(i, r)
}

/// What a Depth of field instance hands its kernel: the [`cpu::DofParams`] the
/// old `Resolved::Dof` variant carried, with the floored blade count coming out
/// of the resolve-time derivation and `depth_bound` — the fact a Layer
/// row cannot put in the bag — supplied by the caller, as the render supplies it
/// from the aux slot.
fn dof_packed(e: &EffectInstance, px_scale: f32, depth_bound: bool) -> cpu::DofParams {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        px_scale,
        &MarkerContext::NONE,
    );
    let p = Params::new(&bag);
    effects::dof::Dof::read(p).packed(depth_bound, effects::dof::Dof::blades_of(p))
}

/// What a Datamosh instance hands its kernel at `lt`: the fields the old
/// `Resolved::Datamosh` variant carried, with the reset ramp and the migrated
/// reach coming out of the resolve-time derivation.
fn datamosh_packed(e: &EffectInstance, lt: f64) -> (f32, f32, f32, i32, f32) {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        lt,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let p = Params::new(&bag);
    let (ramp, reach) = effects::datamosh::Datamosh::derived_of(p);
    effects::datamosh::Datamosh::read(p).packed(ramp, reach)
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
        graph: Default::default(),
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
        pan: crate::anim::Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects,
        styles: Vec::new(),
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

    // A Posterize on a plain (footage) layer holds ONLY that layer's own
    // sampling — the reach is implied by the carrier, so a non-adjustment
    // carrier never holds the layers beneath it.
    let on_footage = vec![footage(vec![post.clone()]), footage(vec![])];
    let stf = posterize_sample_times(&on_footage, 0.37);
    assert!((stf[0] - 0.3).abs() < 1e-9, "the posterised footage snaps");
    assert!(
        (stf[1] - 0.37).abs() < 1e-9,
        "a layer below a plain-layer Posterize stays live"
    );

    // No live Posterize → every layer stays at the live playhead.
    let st = posterize_sample_times(&[footage(vec![]), footage(vec![])], 0.37);
    assert!(st.iter().all(|&s| (s - 0.37).abs() < 1e-9));
}

// accumulation_shutter_offsets (docs/08 §3.26): the decode planner's per-layer
// list of shutter moments — the piece that makes accumulation motion blur
// sample *footage* between frames, not only transforms. An adjustment's
// offsets reach every layer beneath it and none above; two stacked ones give
// the layers under both the union; a bypassed, hidden or out-of-span
// adjustment gives nothing.
#[test]
fn accumulation_shutter_offsets_cover_the_layers_beneath() {
    use crate::model::{LayerKind, Switches, TransformGroup};
    use crate::time::{CompTime, Rational};
    let secs = |n: i64, d: i64| CompTime(Rational::new(n, d).unwrap());
    let layer = |kind: LayerKind, effects: Vec<EffectInstance>| Layer {
        graph: Default::default(),
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
        pan: crate::anim::Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects,
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let footage = || {
        layer(
            LayerKind::Solid {
                def: uuid::Uuid::now_v7(),
            },
            vec![],
        )
    };
    let mb = |samples: f64| {
        let mut e = instantiate("accumulation_mb").unwrap();
        for p in &mut e.params {
            if p.id == "samples" {
                p.value = EffectValue::Float(Property::fixed(samples));
            }
        }
        e
    };
    let offsets_of = |samples: f64| {
        stack_accumulation_mb(&[mb(samples)], true, 0.0)
            .unwrap()
            .sample_offsets()
    };

    // One adjustment over two layers: both beneath get its moments, the
    // adjustment itself and a layer above get none.
    let layers = vec![
        footage(),
        layer(LayerKind::Adjustment, vec![mb(4.0)]),
        footage(),
        footage(),
    ];
    let got = accumulation_shutter_offsets(&layers, 0.5);
    assert!(got[0].is_empty(), "a layer above is not covered");
    assert!(got[1].is_empty(), "the adjustment does not cover itself");
    assert_eq!(got[2], offsets_of(4.0));
    assert_eq!(got[3], offsets_of(4.0));

    // Two stacked: the layer between gets the outer's moments, the layer
    // under both gets the union, sorted, with nothing twice.
    let layers = vec![
        layer(LayerKind::Adjustment, vec![mb(4.0)]),
        footage(),
        layer(LayerKind::Adjustment, vec![mb(2.0)]),
        footage(),
    ];
    let got = accumulation_shutter_offsets(&layers, 0.5);
    assert_eq!(got[1], offsets_of(4.0));
    let mut union = offsets_of(4.0);
    union.extend(offsets_of(2.0));
    union.sort_by(f64::total_cmp);
    union.dedup_by(|a, b| a.to_bits() == b.to_bits());
    assert_eq!(got[3], union);
    assert!(
        got[3].len() > got[1].len(),
        "the inner adjustment adds its own moments"
    );

    // Hidden, out of span, or with effects bypassed: nothing reaches below.
    let mut hidden = layer(LayerKind::Adjustment, vec![mb(4.0)]);
    hidden.switches.visible = false;
    let mut early = layer(LayerKind::Adjustment, vec![mb(4.0)]);
    early.out_point = secs(1, 4);
    let mut bypassed = layer(LayerKind::Adjustment, vec![mb(4.0)]);
    bypassed.switches.fx = false;
    for off in [hidden, early, bypassed] {
        let got = accumulation_shutter_offsets(&[off, footage()], 0.5);
        assert!(got[1].is_empty());
    }
    // A plain layer carrying the effect wants its own moments, for itself
    // alone: it averages its own clip, and nothing beneath it is covered.
    let got = accumulation_shutter_offsets(
        &[
            layer(
                LayerKind::Solid {
                    def: uuid::Uuid::now_v7(),
                },
                vec![mb(4.0)],
            ),
            footage(),
        ],
        0.5,
    );
    assert_eq!(got[0], offsets_of(4.0), "the carrier samples itself");
    assert!(got[1].is_empty(), "a plain carrier covers nothing beneath");
}

#[test]
fn resolve_stack_evaluates_converts_and_skips_dead_effects() {
    let mut e = instantiate("blur").unwrap();
    // 30 px@comp at a px_scale of 1 is 30 raster px.
    let b = resolve_migrated::<effects::blur::Blur>(
        &[e.clone()],
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(b.packed(), (30.0, 1, 1.0));
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
    // A blur whose radius ramps 0→1000 px@comp over one second, so a held time
    // and a frame time resolve to visibly different radii.
    let key = |time: Rational, value: f64| Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let ramp = Property {
        animation: Animation::Keyframed(vec![
            key(Rational::ZERO, 0.0),
            key(Rational::new(1, 1).unwrap(), 1000.0),
        ]),
        extra: serde_json::Map::new(),
    };
    let mut e = instantiate("blur").unwrap();
    for p in &mut e.params {
        if p.id == "radius" {
            p.value = EffectValue::Float(ramp.clone());
        }
    }
    // Blur has moved to the registry, so its radius comes back out of the
    // arena through its own typed reader rather than out of a variant.
    fn radius_of(e: &EffectInstance) -> f32 {
        let ops = super::resolve_stack_temporal(
            std::slice::from_ref(e),
            0.2,
            0.8,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        let fx = ops.get(0).expect("the blur op");
        effects::blur::Blur::read(fx.params).radius
    }
    // Sample time 0.2 (radius 200 px@comp at a px_scale of 1), frame time 0.8
    // (800 px). With the flag ON (the default) the effect samples the held
    // time; with it OFF it holds at the frame time.
    assert!((radius_of(&e) - 200.0).abs() < 0.01);
    e.sample_temporally = false;
    assert!((radius_of(&e) - 800.0).abs() < 0.01);
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

/// A neutral Depth of field bundle with the given per-side radii: every control
/// the aperture and highlight groups added at the value that makes the kernel
/// take its historical path. Spelled once here because a twenty-field
/// struct is not something to write out twice.
fn neutral_dof(near_aperture: f32, far_aperture: f32, focus_point: [f32; 2]) -> cpu::DofParams {
    let (blade_normals, apothem2) = crate::fx::aperture_blades(6, 0.0);
    cpu::DofParams {
        focus: 0.5,
        range: 0.1,
        near_aperture,
        far_aperture,
        blade_normals,
        blade_count: 6,
        apothem2,
        roundness: 1.0,
        rim: 0.0,
        aspect_scale: [1.0, 1.0],
        threshold: 1.0,
        bokeh_power: 1.0,
        repeat_edge: true,
        depth_channel: 0,
        depth_invert: false,
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
    assert_eq!(
        dof_packed(&e, 1.0, false),
        neutral_dof(20.0, 8.0, [960.0, 540.0])
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
    assert_eq!(
        dof_packed(&legacy, 1.0, false),
        neutral_dof(12.0, 12.0, [960.0, 540.0])
    );
}

/// The times **one box** asks its input at (node-graph-comp.md §5.2), one
/// family at a time. `dt` is half a second here, so a frame offset and a
/// second are never the same number by accident.
#[test]
fn input_times_are_the_boxs_own_time_and_the_frames_it_reads() {
    let dt = 0.5;
    let times = |match_name: &str| input_times(&instantiate(match_name).unwrap(), 4.0, dt);

    // A plain effect reads the frame in hand and nothing else.
    assert_eq!(times("blur"), vec![4.0]);

    // Echo reaches back over its declared window, its own time first.
    let echo = times("echo");
    assert_eq!(echo[0], 4.0);
    assert_eq!(echo.len(), 17, "the frame itself and sixteen behind it");
    assert_eq!(echo[1], 3.5);
    assert_eq!(echo[16], 4.0 - 16.0 * dt);

    // Motion blur measures against the frame after, Datamosh against the one
    // before.
    assert_eq!(times("motion_blur"), vec![4.0, 4.5]);
    assert_eq!(times("datamosh"), vec![4.0, 3.5]);

    // Posterize time holds: the grid time alone, and no second entry, because
    // what it shows is that frame and not a blend of two.
    let mut post = instantiate("posterize_time").unwrap();
    set_float(&mut post, "rate", 2.0);
    assert_eq!(input_times(&post, 4.3, dt), vec![4.0]);

    // Accumulation motion blur takes its own time and every shutter moment.
    let acc = instantiate("accumulation_mb").unwrap();
    let want = crate::fx::stack_accumulation_mb(std::slice::from_ref(&acc), true, 4.0)
        .expect("the effect resolves")
        .sample_offsets();
    let got = input_times(&acc, 4.0, dt);
    assert_eq!(got.len(), want.len() + 1);
    assert_eq!(got[0], 4.0);
    assert_eq!(got[1], 4.0 + want[0] * dt);

    // A Time offset shows another second, and that second alone.
    let mut shift = instantiate("time_offset").unwrap();
    set_float(&mut shift, "offset", -1.25);
    assert_eq!(input_times(&shift, 4.0, dt), vec![2.75]);

    // A bypassed box asks for the frame in hand, whatever it is.
    let mut off = instantiate("echo").unwrap();
    off.enabled = false;
    assert_eq!(input_times(&off, 4.0, dt), vec![4.0]);
}

/// A Node graph effect's demands on the picture it is handed join the stack's
/// own window, so the layer is rendered at the frames the graph reads (§5.2).
#[test]
fn a_node_graph_effects_demands_join_the_layers_window() {
    use crate::comp_graph::{CompGraph, GraphEdge, GraphInput, GraphNode, InputKind};
    use crate::graph::{INPUT_PORT, OUTPUT_PORT};
    use crate::model::{Document, ProjectItem};

    let (mut comp, mut layer) = marker_rig((25, 1), Vec::new(), (0, 1));
    let provided = GraphNode::Input {
        id: uuid::Uuid::now_v7(),
        input: GraphInput {
            id: "plate".into(),
            label: "Plate".into(),
            kind: InputKind::Picture,
            default: [0.0; 4],
            min: 0.0,
            max: 1.0,
            unit: Unit::Raw,
            preview: None,
        },
    };
    // An Echo box on the graph's own picture Input: the host layer has to be
    // rendered at every frame it reads.
    let echo = GraphNode::Fx(instantiate("echo").unwrap());
    let out = GraphNode::Output {
        id: uuid::Uuid::now_v7(),
    };
    let wire = |from: &GraphNode, from_port: &str, to: &GraphNode, to_port: &str| GraphEdge {
        from: from.id(),
        from_port: from_port.to_owned(),
        to: to.id(),
        to_port: to_port.to_owned(),
    };
    let graph = CompGraph {
        edges: vec![
            wire(&provided, OUTPUT_PORT.id, &echo, INPUT_PORT.id),
            wire(&echo, OUTPUT_PORT.id, &out, INPUT_PORT.id),
        ],
        nodes: vec![provided, echo, out],
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    };
    let graph_comp_id = uuid::Uuid::now_v7();
    let mut graph_comp = comp.clone();
    graph_comp.id = graph_comp_id;
    graph_comp.graph = Some(graph.clone());
    comp.id = uuid::Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(graph_comp));

    let mut inst = instantiate("node_graph").unwrap();
    crate::fx::effects::node_graph::bind(&mut inst, graph_comp_id, &graph);
    layer.effects = vec![inst];

    let dt = 0.04;
    let window = layer_temporal_window(&doc, &layer, 1.0, dt);
    assert_eq!(
        window,
        (-16..=0).collect::<Vec<i32>>(),
        "the graph's Echo reads the layer's own sixteen frames back"
    );

    // A layer whose graph reads nothing else keeps the window it had.
    layer.effects = vec![instantiate("blur").unwrap()];
    assert_eq!(layer_temporal_window(&doc, &layer, 1.0, dt), vec![0]);
}

#[test]
fn motion_blur_and_datamosh_together_ask_for_both_measurements() {
    // A layer used to carry one flow field and the first consumer in
    // stack order took it, leaving the other silently doing nothing. The two
    // want opposite measurements — forward to the next frame, back to the
    // previous — so the stack asks for both, and stack order does not decide
    // who gets served.
    let mb = instantiate("motion_blur").unwrap();
    let dm = instantiate("datamosh").unwrap();
    assert_eq!(
        stack_flow_neighbours(&[mb.clone(), dm.clone()], true),
        vec![-1, 1]
    );
    assert_eq!(
        stack_flow_neighbours(&[dm.clone(), mb.clone()], true),
        vec![-1, 1],
        "the list must not depend on stack order"
    );
    // Two of the same effect still measure once: sorted and deduplicated.
    assert_eq!(stack_flow_neighbours(&[mb.clone(), mb], true), vec![1]);
    // The fx switch still turns the whole thing off.
    assert!(stack_flow_neighbours(&[dm], false).is_empty());
}

#[test]
fn datamosh_intensity_ceiling_is_open_and_displacement_migrates() {
    // FX-14: the Intensity hard cap is lifted, so a typed
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
    assert_eq!(datamosh_packed(&e, 0.0), (2.5, 9.0, 0.6, 9, 1.0));

    // An old project carries `streak_length`, not `displacement`: the
    // resolve reads it as the reach fallback, so the loaded look is unchanged.
    let mut legacy = instantiate("datamosh").unwrap();
    for p in &mut legacy.params {
        if p.id == "displacement" {
            p.id = "streak_length".to_string();
            p.value = EffectValue::Float(Property::fixed(7.0));
        }
    }
    assert_eq!(datamosh_packed(&legacy, 0.0), (1.0, 7.0, 0.6, 7, 1.0));
}

/// The centrepiece of the current reconstruction (docs/impl/optical-flow.md §4.5
/// item 3), and a straight reversal of v1: **an unconfident pixel inside moving
/// footage must still be blurred.** v1 scaled the streak by confidence, so a
/// pixel the flow could not vouch for collapsed to no blur at all and read as a
/// frozen speck in the middle of a smeared frame — worse to look at than a blur
/// pointing slightly wrong. Here it borrows its neighbourhood's dominant motion
/// at a tempered length.
///
/// The frame is one moving field at a real speed, split down the middle into a
/// confident half and a wholly unconfident one, over noise so that any blurring
/// is measurable as a drop in local variance.
#[test]
fn cpu_motion_blur_unconfident_pixels_borrow_the_neighbourhood_rather_than_freezing() {
    let (w, h) = (64u32, 16u32);
    let n = (w * h) as usize;
    // Deterministic noise: a still frame would blur to nothing measurable.
    let mut img = vec![0.0f32; n * 4];
    for i in 0..n {
        let v = ((i * 2654435761) % 251) as f32 / 250.0;
        img[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, 1.0]);
    }
    // 24 px/frame to the right everywhere — one motion, so the neighbourhood
    // genuinely has something to lend.
    let u = vec![24.0f32; n];
    let v = vec![0.0f32; n];
    // Left half fully trusted, right half not trusted at all.
    let conf: Vec<f32> = (0..n)
        .map(|i| if (i as u32 % w) < w / 2 { 1.0 } else { 0.0 })
        .collect();

    let mut out = img.clone();
    cpu::motion_blur(
        &mut out,
        w,
        h,
        &u,
        &v,
        &conf,
        0.5,
        32,
        1.0,
        MbView::Rendered,
        MbQuality::Normal,
    );

    // Mean absolute difference from the input, over a column well inside each
    // half (away from the seam, where the two behaviours meet and mix).
    let moved = |x0: u32, x1: u32| {
        let mut sum = 0.0f64;
        let mut count = 0u32;
        for y in 0..h {
            for x in x0..x1 {
                let i = ((y * w + x) * 4) as usize;
                sum += f64::from((out[i] - img[i]).abs());
                count += 1;
            }
        }
        sum / f64::from(count)
    };
    let confident = moved(4, 24);
    let unconfident = moved(40, 60);

    assert!(
        confident > 0.05,
        "the confident half must blur at all: {confident}"
    );
    // The point of the test: not "less blur", but blur of the same order —
    // an unconfident region that reads as a sharp hole is the defect.
    assert!(
        unconfident > confident * 0.4,
        "unconfident pixels must still be visibly blurred, not frozen: \
         {unconfident} against {confident} in the trusted half"
    );
    // Tempered, though — a borrowed vector is a guess, and asserting the full
    // length would be claiming knowledge the measurement does not have.
    assert!(
        unconfident < confident,
        "the borrowed streak must be tempered below the trusted one: \
         {unconfident} against {confident}"
    );
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
        MbQuality::Normal,
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

/// The default channel tints — red / green / blue — that reproduce the
/// classic R-outward / B-inward / G-anchor split (P2).
const RGB_TINTS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

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
fn cpu_spectral_split_disperses_and_preserves_uniform() {
    let (w, h) = (17u32, 9u32);
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;

    // The default red/green/blue picker gradient (A1): red at the −1 end,
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
    assert_eq!(
        flash_packed(&e, 0.0, &MarkerContext::NONE),
        (0.0, [1.0; 4], 1.0)
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
fn exposure_instantiates_resolves_and_gains_light() {
    let e = instantiate("exposure").unwrap();
    assert_eq!(e.float_at("stops", 0.0), Some(0.0));
    // 0 stops packs to a neutral factor of 1.0.
    let v: effects::exposure::Exposure =
        resolve_migrated(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(v.packed(), (1.0, 1.0));
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
    // Temperature 0 packs to neutral gains of exactly 1.0 each.
    let v: effects::temperature::Temperature = resolve_migrated(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(v.packed(), (1.0, 1.0, 1.0));
    // The range widens to ±150 slider / ±200 hard, with the stronger
    // ±0.75·k gain. +100 packs to gains (1.75, 0.25): red boosted, blue
    // cut hard. −100 is the mirror (0.25, 1.75). The effect owns the gain
    // formula (`Temperature::gains`), so both render paths read one copy.
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
    let warm: effects::temperature::Temperature =
        resolve_migrated(&[warm], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(warm.packed(), (1.75, 0.25, 1.0));
    // At the +200 hard extreme the blue gain would be 1 − 1.5 = −0.5; the
    // pack floors it at 0 (never a negative channel), red at 2.5.
    let mut hot = e.clone();
    for p in &mut hot.params {
        if p.id == "temperature" {
            p.value = EffectValue::Float(Property::fixed(200.0));
        }
    }
    let hot: effects::temperature::Temperature =
        resolve_migrated(&[hot], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(hot.packed(), (2.5, 0.0, 1.0));
    let mut cool = e;
    for p in &mut cool.params {
        if p.id == "temperature" {
            p.value = EffectValue::Float(Property::fixed(-100.0));
        }
    }
    let cool: effects::temperature::Temperature =
        resolve_migrated(&[cool], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(cool.packed(), (0.25, 1.75, 1.0));
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
    let v: effects::invert::Invert = resolve_migrated(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(v.packed(), 1.0);

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
    // Defaults pack to black→black, white→white (a greyscale mapping).
    let v: effects::tint::Tint = resolve_migrated(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(v.packed(), ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0));

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
    // Preserve luminance is on by default.
    assert_eq!(
        e.param("preserve_luminance"),
        Some(&EffectValue::Bool(true))
    );
    // 0° packs to the identity matrix.
    let v: effects::hue_shift::HueShift = resolve_migrated(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        v.packed(),
        ([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], 1.0)
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
fn contrast_is_neutral_at_100_and_pivots_about_mid_grey() {
    let e = instantiate("contrast").unwrap();
    assert_eq!(e.float_at("contrast", 0.0), Some(100.0));
    // 100 % packs to a neutral factor of 1.0.
    let v: effects::contrast::Contrast =
        resolve_migrated(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(v.packed(), (1.0, 1.0));

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
    // Default 1.0 packs to a neutral gamma.
    let v: effects::gamma::Gamma = resolve_migrated(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(v.packed(), (1.0, 1.0));

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

    // Softness > 1 is a legal, wider feather (not clamped to 1). At
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

    // A neutral balance is the bit-exact identity (the
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

    // Saturation 1 is the bit-exact identity (whole-effect short-circuit).
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

    // Amount 0 is the bit-exact identity (whole-effect short-circuit).
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
        pre_blur: 0.0,
        shrink_grow: 0.0,
        softness: 0.0,
        despot_black: 0.0,
        despot_white: 0.0,
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
    // The old mode-driven blur is now three single-purpose effects.
    // Gaussian (match_name "blur") resolves at its Radius, fixed Repeat edge.
    let gaussian = instantiate("blur").unwrap();
    assert!(gaussian.param("mode").is_none(), "the mode control is gone");
    let b = resolve_migrated::<effects::blur::Blur>(
        std::slice::from_ref(&gaussian),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    // 30 px@comp at a px_scale of 1 = 30px.
    assert_eq!(b.packed(), (30.0, 1, 1.0));

    // Directional blur reads Length/Angle (200 px@comp), fixed Repeat.
    let dir = instantiate("directional_blur").unwrap();
    assert_eq!(dir.float_at("length", 0.0), Some(200.0));
    let d = resolve_migrated::<effects::directional_blur::DirectionalBlur>(
        std::slice::from_ref(&dir),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(d.packed(), (200.0, 0.0, 1, 1.0));

    // Radial blur reads Centre/Amount/Type/Edges: Centre is px@comp and
    // resolves like every other pixel row (px_scale 1 here), Amount
    // 150 px@comp = 150px, Type defaults to Spin, Edges to Repeat.
    let mut radial = instantiate("radial_blur").unwrap();
    for p in &mut radial.params {
        match p.id.as_str() {
            "centre_x" => p.value = EffectValue::Float(Property::fixed(300.0)),
            "centre_y" => p.value = EffectValue::Float(Property::fixed(700.0)),
            _ => {}
        }
    }
    let rb = resolve_migrated::<effects::radial_blur::RadialBlur>(
        std::slice::from_ref(&radial),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(rb.packed(), ([300.0, 700.0], 150.0, true, 1, 1.0));

    // The Type choice flips Spin/Zoom; Edges is honoured (Mirror = 2).
    for p in &mut radial.params {
        match p.id.as_str() {
            "radial_type" => p.value = EffectValue::Choice(1),
            "edge" => p.value = EffectValue::Choice(2),
            _ => {}
        }
    }
    let rb = resolve_migrated::<effects::radial_blur::RadialBlur>(
        std::slice::from_ref(&radial),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let (_, _, spin, edge, _) = rb.packed();
    assert!(!spin, "Type 1 is Zoom");
    assert_eq!(edge, 2, "Edges Mirror is honoured");
    // An out-of-range Choice clamps to Mirror rather than falling back, exactly
    // as the old arm's `(*c).min(2)` did.
    for p in &mut radial.params {
        if p.id == "edge" {
            p.value = EffectValue::Choice(9);
        }
    }
    let rb = resolve_migrated::<effects::radial_blur::RadialBlur>(
        std::slice::from_ref(&radial),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(rb.packed().3, 2);

    // A project saved with the old combined blur (a "blur" instance carrying
    // mode/length/angle/edge) loads as Gaussian at its Radius — the leftover
    // params are simply ignored — existing projects load as Gaussian.
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
    let b = resolve_migrated::<effects::blur::Blur>(
        std::slice::from_ref(&legacy),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    // Fixed Repeat, not the stored edge.
    assert_eq!(b.packed(), (30.0, 1, 1.0));
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
    // px@comp, resolved onto this 17x17 raster: pixel 8's centre.
    let centre = [8.5f32, 8.5f32];

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
    cpu::glow(&mut n, w, h, 4.0, 1.0, 0.5, 0.0, [1.0; 4], 1.0, &[]);
    assert_eq!(n, img);

    // Mix 0 is the exact identity whatever the parameters.
    let mut m0 = img.clone();
    cpu::glow(&mut m0, w, h, 4.0, 0.2, 0.1, 2.0, [1.0; 4], 0.0, &[]);
    assert_eq!(m0, img);

    // A frame entirely below the threshold gains nothing: the halo is
    // zero everywhere and the add is exact.
    let dim = {
        let mut d = img.clone();
        d[mid..mid + 4].copy_from_slice(&[0.1, 0.1, 0.1, 1.0]);
        d
    };
    let mut quiet = dim.clone();
    cpu::glow(&mut quiet, w, h, 4.0, 1.0, 0.5, 1.0, [1.0; 4], 1.0, &[]);
    assert_eq!(quiet, dim);

    // The spike blooms: neighbours gain light, the spike itself gains
    // its own halo back (additive, §2.1: nothing clips).
    let mut g = img.clone();
    cpu::glow(&mut g, w, h, 3.0, 1.0, 0.5, 1.0, [1.0; 4], 1.0, &[]);
    assert!(g[at(10, 4)] > img[at(10, 4)], "neighbour catches the halo");
    assert!(g[mid] > img[mid], "the spike gains its own bloom");

    // The halo carries alpha over transparency: with a threshold low
    // enough that opaque coverage passes it, the transparent border
    // next to the footprint gains coverage — glow reads as light there.
    let mut a = img.clone();
    cpu::glow(&mut a, w, h, 3.0, 0.05, 0.0, 1.0, [1.0; 4], 1.0, &[]);
    assert!(a[at(1, 4) + 3] > 0.0, "coverage bloomed past the edge");
    assert!(a[at(8, 4) + 3] <= 1.0, "alpha saturates at full coverage");

    // Tint colours the halo, not the underlying image: with a red tint,
    // the transparent border gains red light only.
    let mut t = img.clone();
    cpu::glow(
        &mut t,
        w,
        h,
        3.0,
        0.05,
        0.0,
        1.0,
        [1.0, 0.0, 0.0, 1.0],
        1.0,
        &[],
    );
    assert!(t[at(1, 4)] > 0.0, "red halo over the border");
    assert_eq!(t[at(1, 4) + 1], 0.0, "no green in a red-tinted halo");
}

/// **Falloff** (docs/08 §3.3): one bright pixel on black, so the halo is the
/// profile. At every Falloff it has to be round and carry on past the Radius,
/// a gaussian at 0 and an exponential above it. The glow used to stop dead at
/// the Radius in a square, which is what a bright glow showed.
#[test]
fn cpu_glow_halo_is_round_and_has_no_edge() {
    let (w, h) = (129u32, 129u32);
    let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let mid = at(64, 64);
    img[mid..mid + 4].copy_from_slice(&[100.0, 100.0, 100.0, 1.0]);
    let bloom = |falloff: f32| {
        let mut out = img.clone();
        cpu::glow_shaped(
            &mut out,
            w,
            h,
            &cpu::GlowHalo {
                radius_px: 16.0,
                falloff,
                chromatic_px: 0.0,
                fringe_angle_deg: 0.0,
                fringe_tints: cpu::HALO_FRINGE_TINTS,
                fringe_wavelength: false,
                fringe_samples: 16,
            },
            0.0,
            0.0,
            1.0,
            [1.0; 4],
            1.0,
            &[],
        );
        out
    };

    // A 3-4-5 triangle puts a point on the axis and one off it at the same
    // distance, so a square halo shows up as two different values.
    let round = |halo: &[f32], name: &str| {
        for (r, off) in [(20u32, (12, 16)), (35, (21, 28))] {
            let axis = halo[at(64 + r, 64)];
            let diagonal = halo[at(64 + off.0, 64 + off.1)];
            assert!(
                (axis / diagonal - 1.0).abs() < 0.1,
                "{name} round at {r} px: {axis} on the axis, {diagonal} off it"
            );
        }
    };

    // Zero is the gaussian, σ half the Radius, still going past twice it.
    let gaussian = bloom(0.0);
    let mut plain = img.clone();
    cpu::glow(&mut plain, w, h, 16.0, 0.0, 0.0, 1.0, [1.0; 4], 1.0, &[]);
    assert_eq!(gaussian, plain, "falloff 0 is the plain glow");
    round(&gaussian, "gaussian");
    assert!(
        gaussian[at(99, 64)] > 0.0,
        "the gaussian carries on past the Radius"
    );
    let (near, far) = (gaussian[at(76, 64)], gaussian[at(88, 64)]);
    let sigma = ((24.0f32 * 24.0 - 12.0 * 12.0) / 2.0 / (near / far).ln()).sqrt();
    assert!((sigma / 8.0 - 1.0).abs() < 0.05, "σ {sigma}, expected 8");

    let exp = bloom(1e-3);
    round(&exp, "exponential");
    // It falls by the same factor every pixel, at the core's decay length.
    let (near, far) = (exp[at(84, 64)], exp[at(99, 64)]);
    assert!(far > 0.0, "the light carries on past twice the Radius");
    let lambda = 16.0 * cpu::GLOW_CORE;
    let measured = 15.0 / (near / far).ln();
    assert!(
        (measured / lambda - 1.0).abs() < 0.1,
        "decay length {measured}, expected {lambda}"
    );

    // A higher Falloff sends more of the light further out.
    let wide = bloom(2.0);
    assert!(
        wide[at(124, 64)] > 4.0 * exp[at(124, 64)],
        "falloff 2 reaches further"
    );
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
fn cpu_shake_is_identity_at_zero_and_wobbles_through_the_affine() {
    let (w, h) = (17u32, 9u32);
    let img = transform_card(w, h);

    // A neutral shake (zero wobble) is the bit-exact identity: the affine
    // is the identity, whatever the Edges control.
    let neutral = shake_stack(ShakeSample::IDENTITY, 1, 100.0, None);
    let mut n = img.clone();
    cpu::apply_stack(&mut n, w, h, &neutral);
    assert_eq!(n, img);

    // A pure offset matches the Transform reference fed the same shared
    // affine and the same edge policy — the oracle path is one path.
    let shaken = shake_stack(
        ShakeSample {
            offset_px: [2.0, -1.0],
            ..ShakeSample::IDENTITY
        },
        0,
        100.0,
        None,
    );
    let mut s = img.clone();
    cpu::apply_stack(&mut s, w, h, &shaken);
    let (anchor, position, scale, rot) = shake_affine(w, h, [2.0, -1.0], 0.0, 1.0);
    let mut t = img.clone();
    cpu::transform(
        &mut t, w, h, anchor, position, scale, rot, NO_SKEW, 0, 1.0, 1.0,
    );
    assert_eq!(s, t);
    assert_ne!(s, img, "the wobble actually moves pixels");

    // The Edges control governs the revealed border (P3). A big
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
        cpu::apply_stack(
            &mut c,
            w,
            h,
            &shake_stack(
                ShakeSample {
                    offset_px: [6.0, 3.0],
                    ..ShakeSample::IDENTITY
                },
                edge,
                100.0,
                None,
            ),
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
fn cpu_shake_motion_blur_off_is_the_plain_shake_and_on_smears() {
    let (w, h) = (24u32, 16u32);
    let img = transform_card(w, h);

    // A shake carrying a wobble, resolved without motion blur.
    let resolved = |e: &EffectInstance| {
        super::resolve_stack(
            std::slice::from_ref(e),
            0.4,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        )
    };
    let base = shake_with_mb(0.0); // amount 0 ⇒ no sub-frames ⇒ the plain shake
    let plain = resolved(&base);
    assert!(
        matches!(
            shake_packed(&base, 0.4, 1000.0),
            effects::shake::Shaken::Plain { .. }
        ),
        "expected a plain shake"
    );
    let mut a = img.clone();
    cpu::apply_stack(&mut a, w, h, &plain);

    // The same shake with motion blur on smears: the averaged result differs
    // from the plain single resample.
    let smeared = shake_with_mb(0.8);
    assert!(
        matches!(
            shake_packed(&smeared, 0.4, 1000.0),
            effects::shake::Shaken::Blurred { .. }
        ),
        "motion blur on carries sub-frames"
    );
    let mut b = img.clone();
    cpu::apply_stack(&mut b, w, h, &resolved(&smeared));
    assert_ne!(a, b, "motion blur smears the shake");

    // A degenerate sub-frame set — every sample equal to one wobble — averages
    // back to that single resample (to within f32 rounding of the sum ÷ count),
    // pinning the averaging maths against the plain transform reference.
    let one = ShakeSample {
        offset_px: [3.0, -2.0],
        rotation_deg: 5.0,
        zoom: 1.02,
    };
    let mut avg = img.clone();
    cpu::apply_stack(
        &mut avg,
        w,
        h,
        &shake_stack(one, 1, 100.0, Some([one; SHAKE_MB_SAMPLES])),
    );
    let mut one_shot = img.clone();
    cpu::apply_stack(&mut one_shot, w, h, &shake_stack(one, 1, 100.0, None));
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
fn shake_migrates_old_zoom_pump_and_auto_scale_params() {
    // A project saved before FX-11 carries `zoom_pump` and `auto_scale`
    // instead of `z_amp` and `edge`. Resolve reads the old ids as
    // fallbacks so the look migrates sensibly.
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

    // Both folds are resolve-time work: the old ids are not schema rows,
    // so they cannot come out of the bag on their own.
    let effects::shake::Shaken::Plain { wobble, edge, .. } = shake_packed(&old, 0.4, 1000.0) else {
        panic!("a pre-FX-11 shake has no motion blur");
    };
    // The old 10% Zoom pump becomes the z (depth) shake, so zoom moves off
    // 1; Auto-scale off migrates to the Transparent edge (code 0).
    assert_ne!(
        wobble.zoom, 1.0,
        "the old Zoom pump migrated to the z shake"
    );
    assert_eq!(edge, 0, "Auto-scale off migrated to Transparent");

    // Auto-scale on (the old default) migrates to Repeat (code 1).
    for p in &mut old.params {
        if p.id == "auto_scale" {
            p.value = EffectValue::Bool(true);
        }
    }
    let effects::shake::Shaken::Plain { edge, .. } = shake_packed(&old, 0.4, 1000.0) else {
        panic!("a pre-FX-11 shake has no motion blur");
    };
    assert_eq!(edge, 1, "Auto-scale on migrated to Repeat");
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
        NO_SKEW,
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
    cpu::transform(
        &mut r, w, h, centre, centre, [1.0; 2], 90.0, NO_SKEW, 0, 1.0, 1.0,
    );
    assert_eq!(r[mid], 1.0, "the centre pixel stays put");
    assert!(r[at(8, 6) + 1] > 0.999, "+2x lands at +2y");

    // Scale 0 is degenerate: the image collapses to nothing and renders
    // fully transparent — never a division fault (docs/14).
    let mut z = img.clone();
    cpu::transform(
        &mut z,
        w,
        h,
        centre,
        centre,
        [0.0, 0.0],
        0.0,
        NO_SKEW,
        0,
        1.0,
        1.0,
    );
    assert!(z.iter().all(|v| *v == 0.0), "zero scale collapses to clear");

    // Opacity halves all four channels (premultiplied).
    let mut o = img.clone();
    cpu::transform(
        &mut o, w, h, [0.0; 2], [0.0; 2], [1.0; 2], 0.0, NO_SKEW, 0, 0.5, 1.0,
    );
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
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
        graph: Default::default(),
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
        pan: crate::anim::Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
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
    let lt = crate::time::layer_time(1.0, layer.start_offset.0);
    assert_eq!(ctx.beats[0], lt);
    // The obvious no-marker default (§1.4 graceful fallback).
    assert_eq!(MarkerContext::NONE.beats, Vec::<f64>::new());
    assert_eq!(MarkerContext::NONE.fps, 0.0);
    assert_eq!(MarkerContext::default(), MarkerContext::NONE);
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
    // The old arm's two outcomes, transcribed: strength is the envelope
    // times Intensity (100 % → 1), clamped, and colour and mix come
    // straight from the declared rows.
    let dark = (0.0, [1.0; 4], 1.0);
    let lit = (1.0, [1.0; 4], 1.0);
    assert_eq!(
        flash_packed(&e, 1.0, &ctx),
        dark,
        "Manual ignores markers entirely"
    );

    // Trigger mode lights on the beat and is spent past Duration.
    for p in &mut e.params {
        if p.id == "mode" {
            p.value = EffectValue::Choice(1);
        }
    }
    assert_eq!(flash_packed(&e, 1.0, &ctx), lit);
    assert_eq!(
        flash_packed(&e, 1.0, &ctx).0,
        (flash_beat_envelope(&ctx, 1.0, 2.0, false, 1, 0.0) * 1.0).clamp(0.0, 1.0) as f32,
        "the derived strength is the old arm's envelope × intensity"
    );
    assert_eq!(
        flash_packed(&e, 1.75, &ctx),
        dark,
        "3 frames past a 2-frame flash"
    );
    // And with no markers at all it resolves dark — never an error
    // (§1.4 graceful fallback).
    assert_eq!(flash_packed(&e, 1.0, &MarkerContext::NONE), dark);

    // Strobe every 2nd beat: beat index 1 (2 s) does not fire, index 2
    // (3 s) does.
    for p in &mut e.params {
        match p.id.as_str() {
            "mode" => p.value = EffectValue::Choice(2),
            "every_nth" => p.value = EffectValue::Float(Property::fixed(2.0)),
            _ => {}
        }
    }
    assert_eq!(flash_packed(&e, 2.0, &ctx), dark);
    assert_eq!(flash_packed(&e, 3.0, &ctx), lit);
    assert_eq!(
        flash_packed(&e, 3.0, &ctx).0,
        flash_beat_envelope(&ctx, 3.0, 2.0, false, 2, 0.0) as f32,
        "Strobe thins the beat list to every Nth before the envelope"
    );

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
    assert_eq!(flash_packed(&legacy, 1.0, &ctx), (0.4, [1.0; 4], 1.0));
}

/// An orchestration-only effect resolves to **nothing**: no op, no bag, and no
/// id in the render-time indicator's list. It changes what time the layers it
/// covers render at, which the frame walk reads straight off the instance
/// ([`stack_posterize`], [`stack_accumulation_mb`]) — there is no per-pixel pass
/// to order among the others, which is exactly what `resolve_one` returning
/// `None` meant for it before it was declared.
#[test]
fn an_orchestration_only_effect_resolves_to_no_op_at_all() {
    for name in ["posterize_time", "accumulation_mb"] {
        let def = BUILTIN_DEFS.get(name).expect("declared");
        assert!(!def.is_image_op(), "{name} draws nothing");
        let e = instantiate(name).unwrap_or_else(|| panic!("{name} does not instantiate"));
        let (ids, ops) = super::resolve_stack_temporal_named(
            std::slice::from_ref(&e),
            super::ResolvedDrivers::NONE,
            0.0,
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        assert!(ids.is_empty(), "{name} claimed a slot in the indicator");
        assert!(ops.is_empty(), "{name} resolved to an op");
    }

    // And it is still the effect the frame walk finds: declaring it changed
    // where its schema lives, not what reads it.
    let mut post = instantiate("posterize_time").unwrap();
    for p in &mut post.params {
        if p.id == "rate" {
            p.value = EffectValue::Float(Property::fixed(4.0));
        }
    }
    assert!(
        super::stack_posterize(std::slice::from_ref(&post), true, 0.0).is_some(),
        "the held-time detector still reads the instance"
    );
}

/// A derived id shares the bag with the declared ones, so it is covered
/// by the same rule: two ids hashing alike would silently make two controls one.
/// Checked on what actually resolves rather than on the schema alone, because
/// that is where the two kinds of id meet.
#[test]
fn no_resolved_bag_carries_one_id_twice() {
    for def in BUILTIN_DEFS.iter() {
        let name = def.schema().match_name;
        // An orchestration-only effect resolves to no op and so to no bag —
        // there is nothing here for it to carry twice.
        if !def.is_image_op() {
            continue;
        }
        let e = instantiate(name).unwrap_or_else(|| panic!("{name} does not instantiate"));
        let bag = resolve_bag(
            std::slice::from_ref(&e),
            1.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        let mut seen: Vec<ParamId> = Vec::new();
        for (id, _) in &bag {
            assert!(!seen.contains(id), "{name} resolves one id twice");
            seen.push(*id);
        }
    }
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
fn scanlines_migrates_old_darkness_into_intensity() {
    // An old project (FX-13) carried a separate Darkness param
    // (0..100). On load it folds into the single Intensity so the darken is
    // the old Intensity × Darkness product exactly.
    let mut e = instantiate("scanlines").unwrap();
    // Restore the old shape: Intensity 0.5 plus a Darkness of 80%.
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
    // 0.5 × 0.80 = 0.40. The fold reads a parameter that is not a schema row at
    // all, which is why it happens in the resolve-time hook rather than
    // coming out of the bag with the declared ones.
    let intensity = scanlines_packed(&e, 0.0, 1000.0, 1.0).0;
    assert!(
        (intensity - 0.40).abs() < 1e-6,
        "old Darkness folds into Intensity: got {intensity}"
    );
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
// Lens flare (docs/08 §3.27, docs/impl/lens-flare.md §8)
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
// coating cuts the reflectance, and extra layers cut it further.
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

    // Coatings, on ordinary crown glass. Note the glass: MgF₂ is
    // very nearly the IDEAL single layer for n ≈ 1.9, because 1.38² = 1.904,
    // so a stack comparison there measures a coincidence rather than a
    // coating. n = 1.5 is the honest case and the common one.
    let plain = fresnel_cos(1.0, 1.0, 1.5);
    let one = surface_reflectance(1.0, 1.0, 1.5, 0.0, 1.0, 550.0, 1.0);
    let three = surface_reflectance(1.0, 1.0, 1.5, 0.0, 3.0, 550.0, 1.0);
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
        .map(|&nm| surface_reflectance(1.0, 1.0, 1.5, 0.0, 3.0, nm, 1.0))
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
    let straight = surface_reflectance(1.0, 1.0, 1.5, 0.0, 3.0, 550.0, 1.0);
    let oblique = surface_reflectance(0.6, 1.0, 1.5, 0.0, 3.0, 550.0, 1.0);
    assert!(
        oblique > straight * 1.5,
        "the coating must vary with angle: {oblique} at 53° vs {straight} \
         at normal"
    );

    // The Coating dial at 0 is bare glass regardless of the file layers.
    let off = surface_reflectance(1.0, 1.0, 1.5, 0.0, 3.0, 550.0, 0.0);
    assert!((off - plain).abs() < 1e-6);

    // A bare stack (0 layers) is exactly the uncoated interface, and the
    // transfer matrix agrees with plain Fresnel there — the degenerate case
    // that proves the chain closes correctly.
    let empty = stack_reflectance(1.0, 1.0, 1.5, &coating_design(0, 0.0), 550.0);
    assert!(
        (empty - plain).abs() < 1e-5,
        "an empty stack {empty} must equal bare Fresnel {plain}"
    );
}

// §8.4 — the prescription library and pair ranking (curated to
// twenty): every bundled .lens file parses with a sane surface count,
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
    // Bit-identical across runs INCLUDING the field slices, which are
    // baked in parallel: `collect` restores slice order, so the thread pool
    // cannot reach the pixels.
    assert_eq!(a.starburst, b.starburst);
    assert_eq!(
        a.starburst.len(),
        STARBURST_FIELDS * (STARBURST_RES * STARBURST_RES * 3) as usize,
        "the sprite is the field slices concatenated, slice-major"
    );
    assert_eq!(a.energy_gain, b.energy_gain);
    // The ring masks are baked in parallel the same way, and the slice
    // each path picks comes off the spreads — both must be bit-equal too, or
    // two identical projects would draw different ghost edges.
    assert!(!a.pairs.is_empty());
    for path in &a.pairs {
        assert!(path[0] < path[1]);
        assert!((path[1] as usize) < a.surfaces.len());
        // Four-bounce paths carry the same walk one leg further in:
        // the third bounce is past the second, the fourth before the third.
        if path[2] != NO_BOUNCE {
            assert!(path[0] < path[2] && path[3] < path[2]);
            assert!((path[2] as usize) < a.surfaces.len());
            assert_ne!(path[3], NO_BOUNCE);
        } else {
            assert_eq!(path[3], NO_BOUNCE);
        }
    }
}

/// **An animated aperture reuses its bakes**: two f-stops inside one
/// step key the same *and* bake bit-identically, while a step apart they key
/// differently — and the frame's own stop scale stays continuous, so the
/// ghosts still shrink smoothly as the iris closes.
///
/// The equality is not a nicety: the bake cache hands a stored bake to
/// anything whose key matches, so two f-stops that share a key and would bake
/// differently would draw each other's optics. Keeping both sides of that
/// promise in one test is the point.
#[test]
fn lens_flare_bakes_are_shared_across_one_step_of_aperture() {
    use crate::fx::lens_flare::*;
    let seed = default_flare_params();
    // The middle of a step, so a nudge either way stays inside it.
    let base = bake_params(&LensFlareParams { fstop: 2.8, ..seed }).fstop;
    // A quarter of a step away — the same bake by construction.
    let nudged = base * (FSTOP_BAKE_STEP_STOPS * 0.25 * 0.5).exp2();
    let a = LensFlareParams {
        fstop: base,
        ..seed
    };
    let b = LensFlareParams { fstop: nudged, ..a };
    assert_ne!(a.fstop, b.fstop, "the two frames hold different f-stops");
    assert_eq!(
        bake_key(&a),
        bake_key(&b),
        "and ask the cache for the same optics"
    );
    let (ba, bb) = (bake(&a), bake(&b));
    assert_eq!(ba.starburst, bb.starburst, "which must be the same sprite");
    assert_eq!(ba.energy_gain, bb.energy_gain, "and the same exposure");
    assert_eq!(ba.pairs, bb.pairs);

    // A whole step is a different bake, so the aperture is still followed —
    // in steps of about 1.7%, not in leaps.
    let stepped = LensFlareParams {
        fstop: base * (FSTOP_BAKE_STEP_STOPS * 0.5).exp2(),
        ..a
    };
    assert_ne!(
        bake_key(&a),
        bake_key(&stepped),
        "a step apart is a different iris"
    );

    // What the frame itself computes is untouched: the ghost trace's stop
    // scale reads the raw dial, so a slow ramp moves the ghosts every frame
    // rather than every step.
    assert_ne!(
        fstop_scale(ba.native_fstop, a.fstop),
        fstop_scale(bb.native_fstop, b.fstop),
        "the per-frame stop scale is not quantised"
    );

    // The other three continuous iris dials snap the same way.
    let rotated = LensFlareParams {
        aperture_rotation_deg: a.aperture_rotation_deg + APERTURE_ROTATION_BAKE_STEP_DEG * 0.25,
        ..a
    };
    assert_eq!(bake_key(&a), bake_key(&rotated));
    let rounder = LensFlareParams {
        roundness: 0.5,
        ..a
    };
    let rounder_nudged = LensFlareParams {
        roundness: 0.5 + APERTURE_BAKE_STEP * 0.25,
        ..a
    };
    assert_eq!(bake_key(&rounder), bake_key(&rounder_nudged));
    let softer = LensFlareParams {
        aperture_softness: 0.25 + APERTURE_BAKE_STEP * 0.25,
        ..a
    };
    assert_ne!(
        bake_key(&softer),
        bake_key(&LensFlareParams {
            aperture_softness: 0.25 + APERTURE_BAKE_STEP * 1.25,
            ..a
        }),
        "a step of softness is still a different iris"
    );
}

/// **The auto-exposure gain belongs to the lens, not to the iris**:
/// the probe is shot at the prescription's native stop, so two working
/// f-stops on one lens close the loop to bit-identically the same gain — and
/// the frame that is stopped down is honestly dimmer for it, as a real lens
/// is.
///
/// Reading the working stop made the gain roughly `(f/native)²`, which
/// cancelled the stop-down entirely (the same brightness at f/16 as wide
/// open) and put the exposure under the snapped half of the bake, so a slow
/// aperture ramp stepped the whole flare's brightness at every step boundary.
#[test]
fn lens_flare_auto_exposure_reads_the_native_stop() {
    use crate::fx::lens_flare::*;
    let seed = default_flare_params();
    let wide = LensFlareParams { fstop: 2.0, ..seed };
    let stopped = LensFlareParams {
        fstop: 11.0,
        ..seed
    };
    let (bw, bs) = (bake(&wide), bake(&stopped));
    assert!(
        bw.native_fstop < wide.fstop,
        "both stops must be below the lens's maximum aperture ({}) for this \
         to measure anything",
        bw.native_fstop
    );
    assert_eq!(
        bw.energy_gain, bs.energy_gain,
        "the exposure gain must not move with the working aperture"
    );

    // And the honest half: the light the iris passes falls with the square of
    // the stop scale, and nothing puts it back any more.
    let (w, h) = (96u32, 54u32);
    let energy = |p: &LensFlareParams, b: &FlareBaked| -> f32 {
        cpu_flare(p, b, w, h, &manual_light(p, w, h)).iter().sum()
    };
    let (open, shut) = (energy(&wide, &bw), energy(&stopped, &bs));
    assert!(open > 0.0, "the reference must render something to measure");
    assert!(
        shut < open * 0.9,
        "stopping down must dim the flare: {open} wide open, {shut} stopped down"
    );
}

/// One splat through the full deposit — pyramid, then resolve — into a
/// flat `w × h × 3` buffer, which is the shape every kernel test below reads.
/// Splats small enough for level 0 land bit-exactly as they always did (the
/// level-0 resolve is the identity); a splat past [`DEPOSIT_SPAN_PX`] takes
/// the coarser path the production frame takes.
#[allow(clippy::too_many_arguments)]
fn splat_flat(
    out: &mut [f32],
    w: u32,
    h: u32,
    centre: [f32; 2],
    a1: [f32; 2],
    a2: [f32; 2],
    rgb: [f32; 3],
    cell_area: f32,
) {
    use crate::fx::lens_flare::{splat_ray, DepositLevels};
    let mut levels = DepositLevels::new(w, h);
    splat_ray(&mut levels, centre, a1, a2, rgb, cell_area);
    levels.resolve(out);
}

/// **The splat reconstruction is a partition of unity**.
///
/// A uniform sheet of rays on a regular grid, all with the same weight and the
/// same footprint, must reconstruct a **flat** field — that is what "the ghost
/// is smooth" means, and it is the property the old tent lacked. It reached one
/// half-axis while the rays sit a full step apart, so neighbouring tents met
/// exactly where both had fallen to zero: a lattice of separate pyramids with
/// a seam of zero along every cell boundary, which is a woven grid of dark
/// lines printed over every ghost. Energy was conserved throughout, which is
/// why every flux test passed while the artefact was plainly on screen.
///
/// This asserts both halves: the interior is flat, **and** the flux is still
/// exactly what was put in.
#[test]
fn lens_flare_splats_reconstruct_a_flat_sheet_and_keep_their_flux() {
    const W: u32 = 128;
    const H: u32 = 128;
    // Ray spacing in pixels, and the half-axes that go with it: a1 and a2 are
    // HALF a step, which is what `ray_axes` hands over.
    const STEP: f32 = 8.0;
    let a1 = [STEP * 0.5, 0.0];
    let a2 = [0.0, STEP * 0.5];
    let cell_area = STEP * STEP;
    let flux = 3.0_f32;

    let mut out = vec![0.0_f32; (W * H * 3) as usize];
    // A lattice well inside the buffer, so no tent is clipped by an edge and
    // the flux check is exact.
    let (n, origin) = (9_usize, 32.0_f32);
    for j in 0..n {
        for i in 0..n {
            splat_flat(
                &mut out,
                W,
                H,
                [origin + i as f32 * STEP, origin + j as f32 * STEP],
                a1,
                a2,
                [flux, flux, flux],
                cell_area,
            );
        }
    }

    // **Flux.** Every ray's deposit lands inside the buffer, so the total is
    // exactly what went in.
    let total: f64 = out.iter().step_by(3).map(|v| f64::from(*v)).sum();
    let want = f64::from(flux) * (n * n) as f64;
    assert!(
        (total - want).abs() / want < 1e-4,
        "flux must be conserved: {total} vs {want}"
    );

    // **Flatness.** Inside the lattice — a step in from its outermost rays, so
    // every sample sees a full set of neighbours — the field must be constant.
    // The value is the flux of one ray spread over one cell.
    let expect = flux / (STEP * STEP);
    let (lo, hi) = (origin + STEP, origin + (n - 2) as f32 * STEP);
    let mut worst = 0.0_f32;
    let mut ripple_min = f32::MAX;
    let mut ripple_max = 0.0_f32;
    for y in (lo as u32)..(hi as u32) {
        for x in (lo as u32)..(hi as u32) {
            let v = out[((y * W + x) * 3) as usize];
            worst = worst.max((v - expect).abs() / expect);
            ripple_min = ripple_min.min(v);
            ripple_max = ripple_max.max(v);
        }
    }
    assert!(
        worst < 0.02,
        "the interior must be flat: worst deviation {:.1}% (min {ripple_min}, \
         max {ripple_max}, expected {expect})",
        100.0 * worst
    );
    // Said the other way round, because this is the number that was wrong: the
    // peak-to-trough ripple across the sheet. The old tent reached zero at
    // every cell boundary, which is 100%.
    let ripple = (ripple_max - ripple_min) / ripple_max.max(1e-9);
    assert!(
        ripple < 0.05,
        "peak-to-trough ripple across a uniform sheet must be nothing: {:.1}%",
        100.0 * ripple
    );

    // And the same for a sheared footprint, since a ghost's cells are rarely
    // axis-aligned: the tent's frame is the parallelogram's, not the pixel's.
    let sh1 = [STEP * 0.5, STEP * 0.25];
    let sh2 = [-STEP * 0.2, STEP * 0.5];
    let mut sheared = vec![0.0_f32; (W * H * 3) as usize];
    for j in 0..n {
        for i in 0..n {
            let (fi, fj) = (i as f32, j as f32);
            splat_flat(
                &mut sheared,
                W,
                H,
                [
                    origin + fi * 2.0 * sh1[0] + fj * 2.0 * sh2[0],
                    origin + fi * 2.0 * sh1[1] + fj * 2.0 * sh2[1],
                ],
                sh1,
                sh2,
                [flux, flux, flux],
                cell_area,
            );
        }
    }
    let det = (sh1[0] * sh2[1] - sh1[1] * sh2[0]).abs() * 4.0;
    let expect_sh = flux / det;
    let mut worst_sh = 0.0_f32;
    for j in 2..(n - 2) {
        for i in 2..(n - 2) {
            let (fi, fj) = (i as f32, j as f32);
            let x = (origin + fi * 2.0 * sh1[0] + fj * 2.0 * sh2[0]).round() as u32;
            let y = (origin + fi * 2.0 * sh1[1] + fj * 2.0 * sh2[1]).round() as u32;
            let v = sheared[((y * W + x) * 3) as usize];
            worst_sh = worst_sh.max((v - expect_sh).abs() / expect_sh);
        }
    }
    assert!(
        worst_sh < 0.05,
        "a sheared sheet must reconstruct flat too: worst {:.1}%",
        100.0 * worst_sh
    );
}

/// **Ghost edges are Fresnel, and only their edges are**.
///
/// The rim carries the knife-edge diffraction profile a real defocused
/// aperture casts; the interior of a ghost is left exactly as flat as the
/// plain iris mask. That second half is the regression: the earlier propagated
/// masks ran at Fresnel numbers of 2 to 64, two to three orders below what a
/// real ghost has, and at those the near field is a whole-aperture pattern —
/// the bundled default measured 2.4× the flat mask's interior on the bottom
/// rung, 4.7× at the very centre — so every frame-filling ghost painted a
/// broad concentric interference pattern across the picture.
#[test]
fn lens_flare_ghost_edges_ring_without_shading_their_interiors() {
    use crate::fx::lens_flare::*;
    let p = default_flare_params();
    let baked = bake(&p);
    let rot = p.aperture_rotation_deg.to_radians();
    let roundness = effective_roundness(p.roundness, p.fstop, baked.native_fstop);

    // The derivation, at the sizes real ghosts come in: a 5%-of-frame ghost
    // and a frame-filling wash are both hundreds to thousands, never the
    // handful the propagated ladder could reach.
    let tight = ghost_fresnel_number(0.05, 2.8);
    let wash = ghost_fresnel_number(1.0, 2.8);
    assert!(
        (300.0..500.0).contains(&tight),
        "a 5% ghost at f/2.8 is a few hundred, got {tight}"
    );
    assert!(
        (6000.0..8000.0).contains(&wash),
        "a frame-filling ghost at f/2.8 is thousands, got {wash}"
    );
    assert!(wash > tight, "a bigger ghost has the higher Fresnel number");
    // Stopping down shrinks the ghost and the pupil together, so the fringes
    // coarsen as F ∝ scale²; and a degenerate stop rings not at all.
    assert!(ghost_fresnel_number(0.05 * 0.35, 8.0) < tight);
    assert_eq!(ghost_fresnel_number(0.0, 2.8), 0.0);
    assert_eq!(ghost_fresnel_number(0.05, 0.0), 0.0);

    // The knife-edge profile itself: 1 deep inside, ¼ on the edge, a first
    // fringe above 1 just inside it, nothing far outside.
    // Deep inside it settles on 1, approached through a fringe train whose
    // amplitude decays as 1/(πv) — so "flat" is a limit, and the tolerance
    // has to be that decay rather than zero.
    for v in [40.0_f32, 80.0, 160.0] {
        let ripple = (knife_edge_intensity(v) - 1.0).abs();
        assert!(
            ripple < 2.5 / (std::f32::consts::PI * v),
            "at v {v} the profile is {ripple} off 1"
        );
    }
    assert!((knife_edge_intensity(0.0) - 0.25).abs() < 0.01);
    assert!(knife_edge_intensity(-6.0) < 0.02);
    let first = knife_edge_intensity(1.217);
    assert!(
        (1.3..1.45).contains(&first),
        "the first fringe peaks near 1.37, got {first}"
    );
    // Monotone it is not — which is the whole point, and what the analytic
    // mask can never be.
    let profile: Vec<f32> = (0..400)
        .map(|i| knife_edge_intensity(i as f32 * 0.05 - 2.0))
        .collect();
    let peaks = profile
        .windows(3)
        .filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > 1.0)
        .count();
    assert!(peaks >= 3, "the fringe train must have several peaks");

    // **The interior is flat.** Along a radial line through the inner 60% of
    // the pupil, a ringed mask must not deviate from the plain one by more
    // than a whisper — the check the earlier masks could not have passed.
    let f = ghost_fresnel_number(1.0, p.fstop);
    let grid = 2.0 / 63.0;
    let mut worst = 0.0_f32;
    for i in 0..38 {
        let u = i as f32 / 63.0 * 0.98;
        let ringed = ghost_mask(
            u,
            0.0,
            p.blades,
            rot,
            roundness,
            p.aperture_softness,
            f,
            grid,
        );
        let plain = pupil_mask(u, 0.0, p.blades, rot, roundness, p.aperture_softness);
        worst = worst.max((ringed - plain).abs());
    }
    assert!(
        worst < 0.02,
        "the ghost interior must not be shaded by its rim: worst |Δ| {worst}"
    );

    // At zero the mask IS the plain one, byte for byte — the path every
    // unmeasurable ghost takes.
    for i in 0..64 {
        let u = i as f32 / 63.0 * 1.2 - 0.6;
        assert_eq!(
            ghost_mask(
                u,
                0.3,
                p.blades,
                rot,
                roundness,
                p.aperture_softness,
                0.0,
                grid
            ),
            pupil_mask(u, 0.3, p.blades, rot, roundness, p.aperture_softness),
        );
    }

    // **The rim does ring** where the grid is fine enough to carry it: with
    // softness off and a dense grid, the profile overshoots the plateau just
    // inside the edge.
    let fine = 0.002;
    let sharp: Vec<f32> = (0..600)
        .map(|i| {
            let u = i as f32 / 599.0;
            ghost_mask(u, 0.0, p.blades, rot, roundness, 0.0, tight, fine)
        })
        .collect();
    let plateau: f32 = sharp[..100].iter().sum::<f32>() / 100.0;
    assert!(
        sharp.iter().any(|&v| v > plateau * 1.15),
        "the rim must overshoot its own plateau ({plateau}); peak {}",
        sharp.iter().fold(0.0_f32, |m, &v| m.max(v))
    );
    // …and the fringes GROW towards the rim rather than washing over the
    // ghost. The train does reach inwards — a Fresnel edge is not a local
    // effect and pretending otherwise would be the same lie in the other
    // direction — but its envelope decays as 2/(πv), so the outer tenth of
    // the radius must ripple several times harder than the inner half. This
    // is the shape the earlier masks' bottom rungs had exactly backwards:
    // theirs peaked at the CENTRE.
    let ripple = |lo: usize, hi: usize| {
        sharp[lo..hi]
            .iter()
            .fold(0.0_f32, |m, &v| m.max((v - plateau).abs()))
    };
    let (inner, rim) = (ripple(0, 300), ripple(540, 600));
    assert!(
        rim > inner * 3.0,
        "the fringes must gather at the rim: inner {inner}, rim {rim}"
    );
    assert!(
        inner < 0.1,
        "the inner half must stay within a tenth of its plateau, got {inner}"
    );

    // **Fringes nobody can sample are averaged, not aliased.** A grid far
    // coarser than the fringe spacing gets the plain mask back; that is the
    // band limit, and it is what stops an aliased fringe train from beating
    // across the whole ghost.
    let coarse = 2.0 / 15.0;
    for i in 0..64 {
        let u = i as f32 / 63.0 * 1.1;
        let ringed = ghost_mask(u, 0.0, p.blades, rot, roundness, 0.0, wash, coarse);
        let plain = pupil_mask(u, 0.0, p.blades, rot, roundness, 0.0);
        assert!(
            (ringed - plain).abs() < 1e-6,
            "unresolvable fringes must average to the plain edge at u {u}"
        );
    }

    // The Fresnel integrals themselves, against their limits and their known
    // value at the first fringe.
    // Both tend to ½, oscillating in with amplitude ~1/(πv).
    let (c, s) = fresnel_cs(100.0);
    assert!((c - 0.5).abs() < 0.005 && (s - 0.5).abs() < 0.005);
    let (cn, sn) = fresnel_cs(-1.5);
    let (cp, sp) = fresnel_cs(1.5);
    assert!((cn + cp).abs() < 1e-6 && (sn + sp).abs() < 1e-6, "odd in v");
    let (c1, s1) = fresnel_cs(1.0);
    assert!((c1 - 0.7799).abs() < 3e-3, "C(1) = 0.7799, got {c1}");
    assert!((s1 - 0.4383).abs() < 3e-3, "S(1) = 0.4383, got {s1}");

    // Every ranked path can ring now — the closed form costs the same as the
    // polygon, so there is no budget to fall off the end of.
    assert!(!baked.spreads.is_empty());
    assert!(baked
        .spreads
        .iter()
        .all(|&s| ghost_fresnel_number(s, baked.native_fstop) > 0.0));
}

/// **A coating is per glass element, and different coatings make differently
/// coloured ghosts**.
///
/// A real flare shows a blue ghost beside a purple one beside an amber one,
/// because a lens's elements are not all coated alike and what a coated
/// surface reflects is the complement of what its coating suppresses. The
/// palette is that choice, per element; this pins the mapping from element to
/// surface, the colour separation the palette actually produces, and that
/// leaving every row alone is byte-for-byte the picture before it existed.
#[test]
fn lens_flare_coatings_are_per_element_and_colour_the_ghosts() {
    use crate::fx::lens_flare::*;
    let p = default_flare_params();

    // The element mapping, on a lens whose own header states the answer: the
    // Tessar is four elements over eight surfaces.
    let tessar = parse_lens(include_str!(
        "../../lens_files/Zeiss_100mm_F4.5_Tessar.lens"
    ))
    .expect("the bundled Tessar parses");
    assert_eq!(element_count(&tessar.surfaces), 4);
    let elements = surface_elements(&tessar.surfaces);
    assert_eq!(elements.len(), tessar.surfaces.len());
    // Element 0 is the front piece of glass: its own row opens it, the row
    // after closes it. The aperture stop belongs to no element.
    assert_eq!(elements[0], 0);
    assert_eq!(elements[1], 0);
    assert_eq!(elements[2], 1);
    assert_eq!(elements[3], 1);
    assert_eq!(elements[4], -1, "the stop bounds no glass");
    // The cemented pair: the join goes to the earlier element, which is the
    // documented rule.
    assert_eq!(elements[5], 2);
    assert_eq!(elements[6], 3);
    assert_eq!(elements[7], 3);
    // Elements are numbered front to back, contiguously, with no gaps.
    let seen: Vec<i32> = elements.iter().copied().filter(|&e| e >= 0).collect();
    assert!(seen.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1));

    // Every bundled lens reports a sane element count, and the library
    // spans the range the schema's twenty rows have to cover.
    let counts = library_element_counts();
    assert_eq!(counts.len(), 20);
    assert!(counts
        .iter()
        .all(|&c| (3..=MAX_COATING_ELEMENTS as u32).contains(&c)));
    let (lo, hi) = (
        *counts.iter().min().expect("a library"),
        *counts.iter().max().expect("a library"),
    );
    assert!(lo <= 5 && hi >= 16, "counts run {lo}..{hi}");
    // The row thresholds: every lens has a first element, and the deepest
    // rows belong to the few big zooms alone.
    assert_eq!(lenses_with_at_least(1).len(), 20);
    assert!(!lenses_with_at_least(hi).is_empty());
    assert!(lenses_with_at_least(MAX_COATING_ELEMENTS as u32 + 1).is_empty());

    // **Stamping.** An element's choice reaches both of its surfaces, and a
    // surface belonging to no element is left as the file describes it.
    let mut surfaces = tessar.surfaces.clone();
    let mut choices = [COATING_AS_FILE; MAX_COATING_ELEMENTS];
    choices[1] = 4;
    apply_element_coatings(&mut surfaces, &choices);
    assert_eq!(surfaces[2].coating_design, 4.0);
    assert_eq!(surfaces[3].coating_design, 4.0);
    assert_eq!(surfaces[0].coating_design, COATING_AS_FILE as f32);
    assert_eq!(surfaces[4].coating_design, COATING_AS_FILE as f32);
    // An out-of-range palette index is clamped rather than indexing nothing.
    let mut wild = tessar.surfaces.clone();
    let mut mad = [COATING_AS_FILE; MAX_COATING_ELEMENTS];
    mad[0] = 9999;
    apply_element_coatings(&mut wild, &mad);
    assert_eq!(wild[0].coating_design, (COATING_DESIGNS - 1) as f32);

    // **The palette really does separate colours.** Each design's residual
    // reflection is measured at normal incidence across the visible band and
    // reduced to the wavelength it reflects most. Real coatings differ in
    // where their minimum sits, so the peaks must land in different parts of
    // the spectrum — that is the whole mechanism behind a blue ghost sitting
    // beside an amber one.
    let peak_nm = |choice: u32| -> f32 {
        let mut best = (0.0_f32, 550.0_f32);
        let mut nm = 420.0_f32;
        while nm <= 680.0 {
            let r = surface_reflectance(1.0, 1.0, 1.5, choice as f32, 1.0, nm, 1.0);
            if r > best.0 {
                best = (r, nm);
            }
            nm += 2.0;
        }
        best.1
    };
    let blue = peak_nm(3);
    let green = peak_nm(4);
    let amber = peak_nm(5);
    assert!(
        blue < 500.0,
        "the blue-residual design must reflect short, peaks at {blue}"
    );
    assert!(
        amber > 600.0,
        "the amber-residual design must reflect long, peaks at {amber}"
    );
    assert!(
        (blue - amber).abs() > 120.0,
        "two designs must be plainly different colours: {blue} vs {amber}"
    );
    let _ = green;

    // Uncoated is brighter than any coating, at every wavelength tried.
    for nm in [450.0_f32, 550.0, 650.0] {
        let bare = surface_reflectance(1.0, 1.0, 1.5, 1.0, 1.0, nm, 1.0);
        for design in 2..COATING_DESIGNS {
            let coated = surface_reflectance(1.0, 1.0, 1.5, design as f32, 1.0, nm, 1.0);
            assert!(
                coated < bare,
                "design {design} at {nm} nm reflects {coated}, more than bare {bare}"
            );
        }
    }

    // The Coating dial still governs everything: at 0 every design is bare
    // glass, whatever the element rows say.
    let plain = fresnel_cos(1.0, 1.0, 1.5);
    for design in 0..COATING_DESIGNS {
        let off = surface_reflectance(1.0, 1.0, 1.5, design as f32, 3.0, 550.0, 0.0);
        assert!((off - plain).abs() < 1e-6, "design {design} at Coating 0");
    }

    // **An untouched panel changes nothing.** Every row at "As the lens file"
    // must bake byte-for-byte the surfaces the prescription describes.
    let mut untouched = tessar.surfaces.clone();
    apply_element_coatings(&mut untouched, &[COATING_AS_FILE; MAX_COATING_ELEMENTS]);
    for (a, b) in untouched.iter().zip(&tessar.surfaces) {
        assert_eq!(a.coating_layers, b.coating_layers);
        assert_eq!(a.coating_design, COATING_AS_FILE as f32);
    }

    // …and it is a BAKE input, so changing one rebakes rather than quietly
    // serving the previous lens's optics.
    let base = bake_key(&p);
    let mut changed = p;
    changed.coating_elements[0] = 5;
    assert_ne!(base, bake_key(&changed), "an element coating must rebake");
    let mut deeper = p;
    deeper.coating_elements[MAX_COATING_ELEMENTS - 1] = 2;
    assert_ne!(base, bake_key(&deeper), "the last row counts too");

    // And the bake really does answer differently: uncoating the front
    // element brightens the ghosts it takes part in.
    let mut uncoated_front = p;
    uncoated_front.coating_elements[0] = 1;
    let a = bake(&p);
    let b = bake(&uncoated_front);
    assert_eq!(a.surfaces.len(), b.surfaces.len());
    assert_ne!(
        a.reflectance, b.reflectance,
        "the reflectance table must follow the element rows"
    );
}

/// **Four-bounce ghosts** (entry C1): the path model walks, the
/// enumeration stays bounded, and old uncoated glass shows the doubled
/// ghosts modern coatings suppress.
#[test]
fn lens_flare_four_bounce_ghosts_rank_and_render() {
    use crate::fx::lens_flare::*;
    // The walk: a known-bright two-bounce path lands, and its sentinel form
    // is what it always was.
    let p = default_flare_params();
    let baked = bake(&p);
    let dir = light_direction([0.33, 0.30], 9.0 / 16.0, baked.focal_mm);
    let two = baked.pairs[0];
    let origin = [baked.pupil_mm * 0.3, 0.0, baked.start_z_mm];
    let hit = trace_splat(
        &baked,
        [two[0], two[1], NO_BOUNCE, NO_BOUNCE],
        550.0,
        origin,
        dir,
        0.75,
        1.0,
        0.0,
    );
    let Some((pos, w)) = hit else {
        panic!("the brightest ranked path traced nothing at the default light")
    };
    assert!(pos[0].is_finite() && pos[1].is_finite() && w.is_finite());
    // Bit-equal twice: the walk carries no state between calls.
    assert_eq!(
        hit,
        trace_splat(
            &baked,
            [two[0], two[1], NO_BOUNCE, NO_BOUNCE],
            550.0,
            origin,
            dir,
            0.75,
            1.0,
            0.0
        )
    );

    // Vintage glass shows its double ghosts. The Biotar is a 1927 design and
    // every surface of it is bare or single-coated, so a four-bounce path
    // keeps ~10⁻⁶ of the light rather than the ~10⁻¹⁰ a modern stack leaves.
    //
    // **The ranking reality, measured.** On every bundled lens the whole
    // two-bounce family outranks the whole four-bounce one — four extra
    // Fresnel factors are simply worth more than any geometry — so what
    // decides whether a four-bounce ghost renders is not whether it beats a
    // pair but whether the pairs run out first. The Biotar has 11 surfaces
    // and 45 surviving pairs, so its four-bounce paths start at rank 45 and
    // over a hundred of them fall inside the rendered 200. The 24-surface
    // Master Prime has 252 pairs, and its four-bounce paths never get a
    // look in — which is the physically right answer for modern multicoated
    // glass, and the assertion below pins it.
    let vintage = LensFlareParams {
        lens: 17, // Zeiss Biotar 50mm F1.4
        ..default_flare_params()
    };
    let vb = bake(&vintage);
    let four_in_view = vb
        .pairs
        .iter()
        .take(MAX_RENDERED_PAIRS)
        .filter(|p| p[2] != NO_BOUNCE)
        .count();
    assert!(
        four_in_view > 0,
        "the Biotar renders no four-bounce ghost at all: {} paths ranked",
        vb.pairs.len()
    );
    // …and they are honest ghosts, not table entries: one must actually
    // land light on the sensor.
    let vdir = light_direction([0.33, 0.30], 0.5625, vb.focal_mm);
    let landed = vb.pairs.iter().filter(|p| p[2] != NO_BOUNCE).any(|&path| {
        (0..8).any(|k| {
            let frac = k as f32 / 8.0;
            let o = [vb.pupil_mm * frac, vb.pupil_mm * frac * 0.5, vb.start_z_mm];
            matches!(
                trace_splat(&vb, path, 550.0, o, vdir, 1.0, 1.0, 0.0),
                Some((_, w)) if w > 0.0
            )
        })
    });
    assert!(landed, "no four-bounce path put light on the sensor");

    // Modern coatings keep them rare: on the Master Prime the brightest
    // ghosts are all still the two-bounce ones.
    let modern = bake(&default_flare_params()); // lens 16, Master Prime
    for path in modern.pairs.iter().take(8) {
        assert_eq!(
            path[2], NO_BOUNCE,
            "a four-bounce path outranked the two-bounce ghosts on a \
             multi-coated lens: {path:?}"
        );
    }

    // The enumeration bound holds: no more four-bounce paths survive than
    // were ever probed.
    for b in [&baked, &vb] {
        let four = b.pairs.iter().filter(|p| p[2] != NO_BOUNCE).count();
        assert!(
            four <= FOUR_BOUNCE_PROBE_CAP,
            "{four} probed-and-kept paths"
        );
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

/// **The starburst still counts the blades**, re-checked after the field
/// slices: the sprite is the iris polygon's
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

// An anamorphic squeeze (or scale) below 1 asks the combine for flare
// coordinates past the buffer. Up to the 2× padding cap the buffer now
// renders wider and carries real flare there; past even the
// padded extent there is still NO flare — the clamp-addressed tap
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
        "the padded buffer must reach the squeezed frame edge"
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

// Forward migration: a built-in instance saved before its schema
// grew a parameter gains it at the default on load — the panel had been
// drawing a dash and set_value refusing the id.
#[test]
fn lens_flare_backfill_restores_missing_params() {
    let mut inst = instantiate("lens_flare").unwrap();
    // Simulate an older save: strip the params that were added later.
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

// The Background → Blend migration. A project
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

// The share-of-the-frame → px@comp conversions read old projects
// forward, which is the forward-migration rule applied to a *unit* change
// rather than to a missing row: what a saved file rendered, it still renders.
//
// Radial blur's centre was a per cent of the frame, so 30 / 70 on a 1920x1080
// comp is the pixel 576, 756 — and the same point either way, which is the
// only thing the conversion has to be true about. The declared version is the
// gate: a file read twice converts once, and a file saved since the conversion
// is left exactly alone.
#[test]
fn radial_blurs_percent_centre_converts_to_pixels_on_load() {
    let (w, h) = (1920.0, 1080.0);
    let mut inst = instantiate("radial_blur").unwrap();
    inst.effect.version = 1;
    for p in &mut inst.params {
        match p.id.as_str() {
            "centre_x" => p.value = EffectValue::Float(Property::fixed(30.0)),
            "centre_y" => p.value = EffectValue::Float(Property::fixed(70.0)),
            _ => {}
        }
    }
    let mut effects = vec![inst];
    migrate_percent_to_px(&mut effects, w, h);
    let read = |effects: &[crate::model::EffectInstance], id: &str| match effects[0].param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => panic!("{id} must be a float"),
    };
    assert!((read(&effects, "centre_x") - 576.0).abs() < 1e-9);
    assert!((read(&effects, "centre_y") - 756.0).abs() < 1e-9);
    assert_eq!(effects[0].effect.version, 2, "the instance is v2 now");

    // Idempotent: a second read converts nothing, because the version says so.
    migrate_percent_to_px(&mut effects, w, h);
    assert!((read(&effects, "centre_x") - 576.0).abs() < 1e-9);
    assert!((read(&effects, "centre_y") - 756.0).abs() < 1e-9);

    // A keyframed centre keeps its curve: every value scales, and so do the
    // bezier speeds, which live on the value axis (see `scale_property`).
    let mut animated = instantiate("radial_blur").unwrap();
    animated.effect.version = 1;
    for p in &mut animated.params {
        if p.id == "centre_x" {
            p.value = EffectValue::Float(Property {
                animation: crate::anim::Animation::Keyframed(vec![
                    crate::anim::Keyframe {
                        time: crate::time::Rational::new(0, 1).unwrap(),
                        value: 25.0,
                        interp_in: crate::anim::SideInterp::Linear,
                        interp_out: crate::anim::SideInterp::Bezier {
                            speed: 10.0,
                            influence: 1.0 / 3.0,
                        },
                    },
                    crate::anim::Keyframe {
                        time: crate::time::Rational::new(1, 1).unwrap(),
                        value: 75.0,
                        interp_in: crate::anim::SideInterp::Linear,
                        interp_out: crate::anim::SideInterp::Linear,
                    },
                ]),
                extra: serde_json::Map::new(),
            });
        }
    }
    let mut effects = vec![animated];
    migrate_percent_to_px(&mut effects, w, h);
    let Some(EffectValue::Float(p)) = effects[0].param("centre_x") else {
        panic!("centre_x must be a float");
    };
    let crate::anim::Animation::Keyframed(keys) = &p.animation else {
        panic!("centre_x must still be keyframed");
    };
    assert!((keys[0].value - 480.0).abs() < 1e-9, "25% of 1920");
    assert!((keys[1].value - 1440.0).abs() < 1e-9, "75% of 1920");
    assert!(
        matches!(keys[0].interp_out, crate::anim::SideInterp::Bezier { speed, .. }
            if (speed - 192.0).abs() < 1e-9),
        "the speed scales with the values it describes"
    );

    // Nothing else moves: a Percent row on another effect is not a distance.
    let mut untouched = instantiate("levels").unwrap();
    untouched.effect.version = 1;
    let mut effects = vec![untouched];
    let before = effects[0].clone();
    migrate_percent_to_px(&mut effects, w, h);
    assert_eq!(effects[0].params, before.params);
}

// Beam's Length was a per cent of the *run* between Start and End, so its
// conversion reads the instance's own points rather than the frame:
// 25 % of a 1560-pixel run is 390 pixels, and the beam that saved is the beam
// that loads. The points are read at time zero — a keyframed pair means the
// old percentage described a distance that moved, and no single pixel number
// can be all of them.
#[test]
fn beams_percent_length_converts_against_its_own_run() {
    let mut inst = instantiate("beam").unwrap();
    inst.effect.version = 1;
    for p in &mut inst.params {
        if p.id == "length" {
            p.value = EffectValue::Float(Property::fixed(25.0));
        }
    }
    // The declared points: 240,840 to 1680,240 — a run of exactly 1560.
    let mut effects = vec![inst];
    migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
    let read = |effects: &[crate::model::EffectInstance], id: &str| match effects[0].param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => panic!("{id} must be a float"),
    };
    assert!((read(&effects, "length") - 390.0).abs() < 1e-9);
    assert_eq!(effects[0].effect.version, 2);
    migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
    assert!(
        (read(&effects, "length") - 390.0).abs() < 1e-9,
        "idempotent"
    );
}

// Card wipe's Transition width was a per cent of the frame measured along
// whichever axis Flip order runs, so its conversion reads the
// instance's own order: 25 % is 480 pixels across a 1920 frame going left to
// right, and 270 down a 1080 one going top to bottom.
#[test]
fn card_wipes_percent_width_converts_along_its_own_order() {
    for (order, want) in [(0u32, 480.0), (1, 480.0), (2, 270.0), (3, 270.0)] {
        let mut inst = instantiate("card_wipe").unwrap();
        inst.effect.version = 1;
        for p in &mut inst.params {
            match p.id.as_str() {
                "transition_width" => p.value = EffectValue::Float(Property::fixed(25.0)),
                "flip_order" => p.value = EffectValue::Choice(order),
                _ => {}
            }
        }
        let mut effects = vec![inst];
        migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
        let read =
            |effects: &[crate::model::EffectInstance]| match effects[0].param("transition_width") {
                Some(EffectValue::Float(p)) => p.value_at(0.0),
                _ => panic!("transition_width must be a float"),
            };
        assert!((read(&effects) - want).abs() < 1e-9, "order {order}");
        assert_eq!(effects[0].effect.version, 2);
        migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
        assert!(
            (read(&effects) - want).abs() < 1e-9,
            "order {order} idempotent"
        );
    }
}

// Tile's two sizes were per cents of the frame and are px@comp now, so
// a saved v1 instance converts each axis against its own extent: a 2x2 repeat
// stamped over the whole frame is 960 x 540 out of 1920 x 1080. The centre was
// pixels already and must not be touched twice.
#[test]
fn tiles_percent_sizes_convert_axis_by_axis() {
    let mut inst = instantiate("tile").unwrap();
    inst.effect.version = 1;
    for p in &mut inst.params {
        match p.id.as_str() {
            "tile_width" | "tile_height" => p.value = EffectValue::Float(Property::fixed(50.0)),
            "output_width" | "output_height" => {
                p.value = EffectValue::Float(Property::fixed(200.0))
            }
            "tile_centre_x" => p.value = EffectValue::Float(Property::fixed(640.0)),
            _ => {}
        }
    }
    let mut effects = vec![inst];
    migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
    let read = |effects: &[crate::model::EffectInstance], id: &str| match effects[0].param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => panic!("{id} must be a float"),
    };
    let want = [
        ("tile_width", 960.0),
        ("tile_height", 540.0),
        ("output_width", 3840.0),
        ("output_height", 2160.0),
        ("tile_centre_x", 640.0),
    ];
    for (id, v) in want {
        assert!((read(&effects, id) - v).abs() < 1e-9, "{id}");
    }
    assert_eq!(effects[0].effect.version, 2);
    // Read twice, converted once: the version is the gate.
    migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
    for (id, v) in want {
        assert!((read(&effects, id) - v).abs() < 1e-9, "{id} idempotent");
    }
}

// Lens flare's Ghost softness was a per cent of the frame *diagonal* — the one
// exception to the px@comp rule, now closed — so its conversion is the only one
// in the sweep that reads a diagonal: 1 % of a 1080p frame's 2202.9 px is
// 22.03 pixels of blur, and the flare that saved is the flare that loads.
#[test]
fn lens_flares_percent_softness_converts_against_the_diagonal() {
    let mut inst = instantiate("lens_flare").unwrap();
    inst.effect.version = 11;
    for p in &mut inst.params {
        if p.id == "ghost_softness" {
            p.value = EffectValue::Float(Property::fixed(1.0));
        }
    }
    let mut effects = vec![inst];
    migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
    let read = |effects: &[crate::model::EffectInstance]| match effects[0].param("ghost_softness") {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => panic!("ghost_softness must be a float"),
    };
    let want = 1920.0f64.hypot(1080.0) / 100.0;
    assert!((read(&effects) - want).abs() < 1e-9);
    assert_eq!(effects[0].effect.version, 12);
    // Read twice, converted once.
    migrate_percent_to_px(&mut effects, 1920.0, 1080.0);
    assert!((read(&effects) - want).abs() < 1e-9, "idempotent");
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
    assert_eq!(ops.len(), 1, "lens_flare resolves to exactly one op");
    let p = flare_packed(&ops);
    assert!(
        (p.intensity - 3.0).abs() < 1e-6,
        "intensity must follow the expression through the context: {}",
        p.intensity
    );
}

// The blend table itself, against the formulas written out by hand.
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

// Frame-time grid probe: the bake spread is a bounding-box measure
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
    // Normal tier — the condition the budget raise exists for.
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
        // Every element left as the lens file describes it — the
        // drop-on default, and byte-for-byte the picture it always drew.
        coating_elements: [crate::fx::lens_flare::COATING_AS_FILE;
            crate::fx::lens_flare::MAX_COATING_ELEMENTS],
        // Raster pixels: tests divide by their own raster via
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
        // px@comp now. The old 0.05 % of a 96x54 diagonal was a
        // twentieth of a pixel and rounded to no blur; half a pixel is the
        // same picture, said in the unit the dial now speaks.
        ghost_softness: 0.5,
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
        matte_invert: false,
        anamorphic: 1.0,
        quality: 1,
        detail: 1.0,
        blend: crate::fx::lens_flare::BLEND_ADD,
        mix: 1.0,
    }
}

// Matte-mode source detection (impl note §6): the CPU reference finds
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

    let lights = detect_lights(&matte, w, h, 1.0, 0.0, true, [1.0; 3], false);
    assert_eq!(
        lights.len(),
        2,
        "the neighbour must be suppressed: {lights:?}"
    );
    // Brightest first — and the light sits at the flux centre of
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

    // The soft gate scales (luma 4 against a gate opening 3 → 5
    // lands half-way, 0.5), and a threshold above every source finds none —
    // including one AT a source's luma, which "brighter than" excludes.
    let gated = detect_lights(&matte, w, h, 3.0, 2.0, true, [1.0; 3], false);
    assert!(!gated.is_empty());
    assert!(gated[0].rgb[0] < 4.0, "the gate must attenuate: {gated:?}");
    assert!(detect_lights(&matte, w, h, 10.0, 0.0, true, [1.0; 3], false).is_empty());
    assert!(
        detect_lights(&matte, w, h, 4.0, 0.0, true, [1.0; 3], false).is_empty(),
        "a threshold at the brightest source's own luma finds nothing:          the gate is 'brighter than', not 'at least'"
    );

    // Determinism: two runs agree bit-for-bit.
    assert_eq!(
        lights,
        detect_lights(&matte, w, h, 1.0, 0.0, true, [1.0; 3], false)
    );

    // The gate itself (one-sided): closed at and below the threshold,
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

/// **An area source renders as a smooth shape, not a woven grid**.
///
/// The per-ray source integration hops each ray's source point by more
/// than the whole source between pupil neighbours — that is what
/// equidistributes the samples — and three things in the reconstruction let
/// that read as a quasi-periodic mesh stamped across every ghost, which is
/// what the owner photographed: central-difference footprints cancelled
/// toward zero wherever a ray's two neighbours hopped to the same side; the
/// old `PHI_V` sits within 0.002 of 4/7, so its samples fell into seven
/// slow-drifting combs that lined up into stripes; and every band re-traced
/// the same source points, so the bands' summed ripple never averaged.
///
/// The flux tests all passed throughout, exactly as the imprint test records
/// for the kernel's own version of this lesson: they measure how much light
/// there is, never whether it is smooth. So this measures smoothness — the
/// row-to-row and column-to-column ripple of the rendered disc against its
/// own local mean — on the brightest ghost of an area render.
#[test]
fn lens_flare_an_area_source_renders_without_stripes() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        // No ghost blur: the ripple must die in the reconstruction, not be
        // hidden under a blur the user is free to turn off. No starburst:
        // this measures the ghosts.
        ghost_softness: 0.0,
        starburst_intensity: 0.0,
        quality: 1,
        source_size: [16.0, 10.0],
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (256u32, 144u32);
    let buf = cpu_flare(&p, &baked, w, h, &manual_light(&p, w, h));
    let lum = |x: usize, y: usize| {
        let i = (y * w as usize + x) * 3;
        buf[i] + buf[i + 1] + buf[i + 2]
    };
    // The grid-imprint metric with a WIDER neighbourhood: each pixel's
    // departure from its own 9×9 mean, relative to that mean, over the lit
    // region. A 3×3 neighbourhood cannot see this artefact — the mesh's
    // period is the ray spacing, several pixels, so every pixel sits close to
    // a 3×3 mean and a plainly striped ghost scores under that test's bound
    // (measured; it is the same passed-while-visible trap that test itself
    // records). A 9×9 mean spans the mesh's period and reads it.
    let mx = (0..h as usize)
        .flat_map(|y| (0..w as usize).map(move |x| (x, y)))
        .map(|(x, y)| lum(x, y))
        .fold(0.0_f32, f32::max);
    assert!(mx > 0.0, "the area flare must render something to measure");
    let (mut num, mut den) = (0.0_f64, 0.0_f64);
    for y in 4..h as usize - 4 {
        for x in 4..w as usize - 4 {
            let c = lum(x, y);
            if c < mx * 0.02 {
                continue;
            }
            let mut m = 0.0_f32;
            for dy in 0..9 {
                for dx in 0..9 {
                    m += lum(x + dx - 4, y + dy - 4);
                }
            }
            m /= 81.0;
            num += f64::from((c - m).abs());
            den += f64::from(m);
        }
    }
    let ripple = (100.0 * num / den.max(1e-9)) as f32;
    assert!(
        ripple < 3.0,
        "an area source's ghosts are rippling at {ripple:.2}% against the \
         ~4% the current reconstruction measures — the woven mesh is coming \
         back (the earlier one measured ~13% here, and read as a \
         grid stamped across every ghost on screen)"
    );
}

/// **Light layers resolve, and an area light keeps its size**.
///
/// The whole reason the layer exists is the area kind: a light with a real
/// width and height flares as its own shape through the source machinery,
/// where a point can only ever be a dot. This pins the resolve — including that
/// only an area light reports extent, whatever the stored numbers say, and that
/// a light switched off is not a light (the rule for every layer).
#[test]
fn lens_flare_light_layers_resolve_with_their_extent() {
    use crate::anim::Property;
    use crate::model::*;
    use crate::time::{CompTime, Duration, FrameRate, Rational};

    let mut comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
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
            graph: Default::default(),
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
            pan: Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
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

    // ---- and the whole way through to the trace ----
    //
    // Lights mode's sources are the one thing about this effect that is neither
    // a control nor a picture the render prepared, so they ride the resolve-time
    // derivation hook into the same bag as everything else. Nothing checks the
    // ids that carry them but this: a derivation that pushed under a name the
    // reader does not look for resolves a perfectly ordinary flare with no
    // lights in it, which looks exactly like a comp that has none.
    let mut flare = instantiate("lens_flare").unwrap();
    for p in &mut flare.params {
        if p.id == "source_type" {
            p.value = EffectValue::Choice(2);
        }
    }
    let comp_id = comp.id;
    let mut document = crate::model::Document::new();
    document
        .items
        .push(crate::model::ProjectItem::Composition(comp));
    let context = Arc::new(ExpressionContext {
        document: Arc::new(document),
        comp: Some(comp_id),
        comp_time: 1.0,
        ..ExpressionContext::detached()
    });
    let ops = super::resolve_stack(
        std::slice::from_ref(&flare),
        0.0,
        2202.9,
        1.0,
        &MarkerContext::NONE,
        context.clone(),
    );
    let p = flare_packed(&ops);
    assert_eq!(p.source, 2, "Lights mode");
    assert_eq!(
        p.light_count, 2,
        "the two visible lights, and not the third"
    );
    assert_eq!(
        p.lights[0].pos,
        [300.0, 200.0],
        "raster pixels, as resolved"
    );
    assert_eq!(
        p.lights[0].extent,
        [80.0, 40.0],
        "the area light's half-size"
    );
    assert_eq!(p.lights[0].rgb, [1.0, 1.0, 1.0]);
    assert_eq!(
        p.lights[1].extent,
        [0.0, 0.0],
        "a point light has no extent"
    );
    // And `manual_light` divides them by the raster exactly as it divides the
    // Manual point — one place decides the fraction.
    let placed = crate::fx::lens_flare::manual_light(&p, 1920, 1080);
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0].pos, [300.0 / 1920.0, 200.0 / 1080.0]);

    // Manual mode carries none of it: the derivation pushes nothing at all, so
    // the flare is the single light at the parameter position it always was.
    let manual = instantiate("lens_flare").unwrap();
    let ops = super::resolve_stack(
        std::slice::from_ref(&manual),
        0.0,
        2202.9,
        1.0,
        &MarkerContext::NONE,
        context,
    );
    let p = flare_packed(&ops);
    assert_eq!(p.light_count, 0, "Manual mode has no light layers to carry");
    assert_eq!(crate::fx::lens_flare::manual_light(&p, 1920, 1080).len(), 1);
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

        // A group's members must be a contiguous run of `params`,
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
    // The focus point reads its **declared default** rather than the old arm's
    // separate `unwrap_or(0.0)` fallback — the rule for a parameter a saved
    // project has never heard of, and the one the arena applies to every row.
    // Nothing renders differently for it: the point is read only when Use focus
    // point is on, which this instance does not carry either, so it stays false.
    assert_eq!(
        dof_packed(&legacy, 1.0, false),
        neutral_dof(8.0, 8.0, [960.0, 540.0])
    );
}

// **The fold's load-bearing promise**: at the shipped defaults the
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

fn set_float(e: &mut EffectInstance, id: &str, v: f64) {
    for p in &mut e.params {
        if p.id == id {
            p.value = EffectValue::Float(Property::fixed(v));
        }
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

/// An instance may carry the user's own name. `None` — every older
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

/// **Spectral radiometry preserves exposure and actually resolves the
/// coating** (entry A2). Two halves:
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

// ---------------------------------------------------------------------------
// The effect registry (docs/impl/effect-registry.md §7)
// ---------------------------------------------------------------------------

use uuid::Uuid;

/// A name is how a saved project finds its effect again, so two effects sharing
/// one is a project-corrupting defect rather than a mere mistake.
#[test]
fn every_builtin_declares_a_unique_match_name() {
    let mut seen: Vec<&str> = Vec::new();
    for s in BUILTINS {
        assert!(
            !seen.contains(&s.match_name),
            "two effects answer to {}",
            s.match_name
        );
        seen.push(s.match_name);
    }
}

/// A project saved before a parameter existed carries no entry for it, and must
/// render — reading the declared default, never panicking.
#[test]
fn a_missing_parameter_reads_its_default() {
    let empty = Params::EMPTY;
    let v = effects::saturation::Saturation::read(empty);
    assert_eq!(v.saturation, 100.0);
    assert_eq!(v.mix, 100.0);

    // And a parameter that *is* present wins over the default.
    let entries = [(
        effects::saturation::Saturation::SATURATION,
        Value::Float(50.0),
    )];
    let v = effects::saturation::Saturation::read(Params::new(&entries));
    assert_eq!(v.saturation, 50.0);
    assert_eq!(v.mix, 100.0);
}

/// A document holding one comp lit by `n` visible area lights, spaced along
/// x from 300 by 100, each `(80, 40)` half-size at y 200 — what a Lights-mode
/// flare resolves its derived sources from. Returns the document and the
/// comp's id, ready for an [`ExpressionContext`].
fn lit_document(n: usize) -> (crate::model::Document, Uuid) {
    use crate::model::*;
    use crate::time::{CompTime, Duration, FrameRate};

    let mut comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
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
    for i in 0..n {
        comp.layers.push(Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "Light".into(),
            kind: LayerKind::Light {
                light: Box::new(LightDef {
                    kind: LightKind::Area,
                    half_size: [Property::fixed(80.0), Property::fixed(40.0)],
                    ..LightDef::default()
                }),
            },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: TransformGroup {
                position_x: Property::fixed(300.0 + 100.0 * i as f64),
                position_y: Property::fixed(200.0),
                ..TransformGroup::default()
            },
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            pan: Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        });
    }
    let comp_id = comp.id;
    let mut document = Document::new();
    document.items.push(ProjectItem::Composition(comp));
    (document, comp_id)
}

/// Every byte [`ResolvedStack::feed_hash`] writes for the stack — the frame
/// key's view of it, so two arenas that hash alike are the same arena.
fn stack_bytes(stack: &ResolvedStack) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    stack.feed_hash(&mut |b| bytes.extend_from_slice(b));
    bytes
}

/// **A derived pixel length follows the raster** (docs/impl/effect-registry.md
/// §2.4a). Scanlines' `derived.roll_px` is roll speed × layer time × the
/// *raster* period, so it is in raster pixels — but a derived id matches no
/// schema row, and the generic rescale used to move the period and leave the
/// roll behind, shifting the pattern's phase with the size of the raster a
/// precomp was realised at. [`EffectDef::derived_spatial`] is how the effect
/// tells the pass; this pins that it does, that the derived intensity (a
/// strength, not a length) stays put, that the result lands exactly where a
/// direct resolve against the smaller raster lands, and that factor 1 leaves
/// the arena bit-identical.
#[test]
fn a_derived_roll_moves_with_the_raster() {
    let mut e = instantiate("scanlines").unwrap();
    for p in &mut e.params {
        if p.id == "scanline_roll" {
            p.value = EffectValue::Float(Property::fixed(4.0));
        }
    }
    let resolve = |px_scale: f32| {
        super::resolve_stack(
            std::slice::from_ref(&e),
            0.5,
            1000.0 * px_scale,
            px_scale,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        )
    };
    let packed = |ops: &ResolvedStack| {
        let p = ops.get(0).expect("the scanlines op").params;
        let (i, r) = effects::scanlines::Scanlines::derived_of(p);
        effects::scanlines::Scanlines::read(p).packed(i, r)
    };

    // At the comp raster: a 3 px period, and 4 lines/s at 0.5 s over it = 6 px.
    let mut ops = resolve(1.0);
    let (_, period, roll, _, _) = packed(&ops);
    assert_eq!(period, 3.0);
    assert_eq!(
        roll, 6.0,
        "the roll is non-zero, so a lost multiply cannot hide"
    );

    // Reused at half size: both lengths halve, and nothing else moves.
    ops.rescale_spatial(0.5);
    let (intensity, period, roll, interlace, mix) = packed(&ops);
    assert_eq!(period, 1.5, "the declared Px period follows the raster");
    assert_eq!(roll, 3.0, "and so does the derived roll offset");
    assert_eq!(intensity, 0.35, "the derived intensity is not a length");
    assert!(!interlace);
    assert_eq!(mix, 1.0);

    // Which is exactly where resolving against the half raster lands — the
    // phase a precomp at that size would have had on its own.
    assert_eq!(packed(&resolve(0.5)), packed(&ops));

    // Factor 1 is exactly a no-op, over the whole arena.
    let mut same = resolve(1.0);
    let before = stack_bytes(&same);
    same.rescale_spatial(1.0);
    assert_eq!(stack_bytes(&same), before);
}

/// **Every derived spatial id is one the effect actually derives.** A list
/// entry that names a declared row would double-scale it (the row's own unit
/// already moves it); one that names nothing the hook pushes is a promise the
/// rescale pass can never keep; and one whose value is not a length has no
/// business in the list at all. So for every built-in: no entry is a schema
/// id, none repeats, and each is pushed by `resolve_derived` as a `Float`,
/// `Colour` or `Vec4` — run in the richest context any declaring effect
/// wants, a Lights-mode comp with the full complement of lights, so the
/// flare's sixteen geometry ids all come out.
#[test]
fn every_derived_spatial_id_is_one_the_effect_actually_derives() {
    let (document, comp_id) = lit_document(crate::fx::lens_flare::MAX_SOURCES);
    let context = Arc::new(ExpressionContext {
        document: Arc::new(document),
        comp: Some(comp_id),
        comp_time: 1.0,
        ..ExpressionContext::detached()
    });
    let mut declaring = 0;
    for def in BUILTIN_DEFS.iter() {
        let name = def.schema().match_name;
        let spatial = def.derived_spatial();
        for p in def.schema().params {
            assert!(
                !spatial.contains(&ParamId::new(p.id)),
                "{name}: {} is a declared row and carries its own unit",
                p.id
            );
        }
        for (i, id) in spatial.iter().enumerate() {
            assert!(
                !spatial[..i].contains(id),
                "{name}: a derived spatial id is listed twice"
            );
        }
        if spatial.is_empty() {
            continue;
        }
        declaring += 1;
        let mut inst = instantiate(name).unwrap_or_else(|| panic!("{name} is a built-in"));
        // The one mode fork among the declaring effects: the flare pushes its
        // lights only in Lights mode.
        for p in &mut inst.params {
            if p.id == "source_type" {
                p.value = EffectValue::Choice(2);
            }
        }
        let mut pushed: Vec<(ParamId, Value)> = Vec::new();
        def.resolve_derived(
            &ResolveCx {
                inst: &inst,
                lt: 0.5,
                diag_px: 2202.9,
                px_scale: 1.0,
                markers: &MarkerContext::NONE,
                context: context.clone(),
            },
            &mut |id, value| pushed.push((id, value)),
        );
        for id in spatial {
            let Some((_, value)) = pushed.iter().find(|(k, _)| k == id) else {
                panic!("{name} lists a derived spatial id its resolve_derived never pushes");
            };
            assert!(
                matches!(value, Value::Float(_) | Value::Colour(_) | Value::Vec4(_)),
                "{name}: a derived spatial value is a length or a vector of them, not {value:?}"
            );
        }
    }
    assert_eq!(
        declaring, 2,
        "Scanlines and the Lens flare are the two today"
    );
}

/// docs/impl/effect-registry.md §7 test 4, deferred from the plumbing stage: a
/// spatial parameter in the arena rescales under [`ResolvedStack::rescale_spatial`]
/// **exactly** as the old `Resolved` op did.
///
/// This is the one property the migration could silently lose. The repair —
/// a stack resolved against the comp raster and then run on a smaller preview
/// target — reaches the arena through `ResolvedStack::rescale_spatial`, which calls
/// both halves; if a blur declared `Unit::Raw` it would render at full-size
/// radii on a half-size preview, which is precisely the bug that was fixed for
/// the flare. So the golden values are written out: the old table said "radius
/// scales, mix does not", and both are checked through the public entry point
/// rather than through `rescale_spatial` directly.
#[test]
fn a_migrated_spatial_parameter_rescales_as_the_old_op_did() {
    // 30 px@comp at a px_scale of 1 = 30 px, the comp-raster resolve.
    let e = instantiate("blur").expect("blur is a built-in");
    let mut ops = super::resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    let radius = |ops: &super::ResolvedStack| -> f32 {
        effects::blur::Blur::read(ops.get(0).expect("the blur op").params)
            .packed()
            .0
    };
    assert_eq!(radius(&ops), 30.0);

    // Half-resolution preview: the old `rescale_px` multiplied `radius_px` by
    // the factor, and so must the arena.
    ops.rescale_spatial(0.5);
    assert_eq!(radius(&ops), 15.0, "the radius follows the preview raster");
    assert_eq!(
        effects::blur::Blur::read(ops.get(0).expect("the blur op").params)
            .packed()
            .2,
        1.0,
        "the Mix does not"
    );

    // Resolving directly against the smaller raster must land in the same
    // place — which is the whole point of the correction.
    let direct = super::resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        500.0,
        0.5,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    assert_eq!(radius(&direct), radius(&ops));

    // Factor 1 is exactly a no-op, as it was for the variants.
    let mut same = super::resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    same.rescale_spatial(1.0);
    assert_eq!(radius(&same), 30.0);
}

/// Every parameter the catalogue declares says what its number means, and only
/// the two raster-following units are treated as lengths — the guard that stops
/// a spatial parameter being declared `Raw` and quietly skipping the rescale.
#[test]
fn every_parameter_declares_a_unit() {
    let spatial: Vec<(&str, &str)> = BUILTIN_DEFS
        .iter()
        .flat_map(|d| {
            d.schema()
                .params
                .iter()
                .map(move |p| (d.schema().match_name, p))
        })
        .filter(|(_, p)| p.unit.is_spatial())
        .map(|(name, p)| (name, p.id))
        .collect();
    // (The history below names some entries "% diag": that was their unit
    // when they joined the list. Every one of them is px@comp now; the
    // list of *which* parameters follow the raster has not changed.)
    //
    // The blur family's lengths, the two flare/transform families'
    // px@comp points and radii, and Block glitch's own pair of currencies —
    // nothing else. Radial blur's Centre is a fraction of the frame, Sharpen's
    // neighbour distance is a kernel stride, Sprite flare's Ghost spacing is a
    // fraction of the light→centre distance, Transform's Scale is per cent, and
    // every Vignette distance is read in a metric derived from the raster's own
    // w/h — so none of those follows the raster, exactly what the old
    // `rescale_px` match said about them.
    //
    // Scanlines' Line period is the one entry the old match disagreed with: it
    // was scaled by the preview factor at resolve and then *not* rescaled with
    // the stack, so a scanlined adjustment layer under a reduced-resolution
    // preview kept comp-sized lines. Declaring the unit states it once and the
    // generic pass does both halves.
    //
    // Depth of field's **Aperture is deliberately absent**, though it is
    // authored in px@comp like the two radii beside it. It enters the maths as
    // the unitless ratio `aperture / 8` multiplying Near/Far, and exactly one
    // factor of that product may follow the raster — declaring all three would
    // blur a half-resolution preview by a quarter of the disc. The factor that
    // follows it is Near/Far, which is also the pair the old `rescale_px` moved.
    //
    // The Lens flare's **Source size** is the second entry the old match
    // disagreed with, for the same reason Scanlines' period was: it was scaled by
    // the preview factor at resolve and then not rescaled with the stack, so an
    // area source on an adjustment layer kept its comp-sized extent under a
    // reduced-resolution preview while the Light point beside it moved. Both are
    // px@comp and the pair has to travel together, or the flare's ghosts take the
    // shape of a source the wrong size for the frame they land on.
    //
    // Shake's **Amplitude** is the one entry the old match reached by another
    // road: the arm multiplied it by the diagonal by hand and `rescale_px`
    // scaled the *resolved offsets* instead. Declaring the unit puts the scaling
    // one multiply earlier, which is the same wobble to within the
    // reassociation `shake_amplitude_rescales_as_the_old_offsets_did` bounds.
    //
    // The Generate family brings nine more px@comp entries and no % diag
    // ones. Gradient's two **points** must travel together or the ramp slides
    // when the preview resolution changes; Fractal noise's three **cell sizes**
    // and its **offset** likewise, and its Scale being a length rather than AE's
    // per cent of an unnamed base is exactly what lets it be declared at all
    // (docs/08 §3.37 decision 1). Fill and Noise declare none — neither has a
    // spatial control.
    //
    // The distort batch brings ten more px@comp entries and, again, no % diag
    // ones. Turbulent displace's **Amount** and **Size** are both lengths, for
    // §3.37 decision 1's reason applied to a warp: a per cent of an unnamed base
    // does not survive a resize. Its **Offset** and every other effect's
    // **centre** are the point pairs that must be pixels. Tile declares only
    // its Tile centre — the four per cents beside it are fractions of the raster
    // and so do not follow it — and Lens distort only its Centre, since a field
    // of view is an angle.
    //
    // The utility and transition batch brings seven more, four % diag and
    // three px@comp. **Channel blur's four radii** are % diag exactly as the
    // Gaussian blur's is, being the same kernel four times over. **Drop shadow's
    // Distance and Softness** are px@comp and must travel together, or a
    // half-resolution preview would move the shadow and not soften it (or the
    // reverse); its Direction is an angle and its Opacity a per cent, so neither
    // is here. The **wipes' centres and feathers** are px@comp for the point-pair
    // reason and for the shadow's; their Completion is a per cent of the frame's
    // own extent, which the kernel derives from the raster it is handed, so it
    // needs no rescaling and gets none — Tile's four per cents again.
    //
    // Wave 2's Distort I batch (docs/08 §3.48-§3.52) brings eighteen more: two
    // % diag and sixteen px@comp. **Corner pin's eight point coordinates** are
    // pixels as point pairs and must travel together or a half-resolution
    // preview would pin three corners and stretch the fourth. **Displacement
    // map's two Amounts** are lengths for §3.38 decision 5's reason, a third
    // time. **Twirl's and Spherize's radii** are % diag exactly as the blur's
    // is — a reach into the picture, not a pixel-scale look — while their
    // centres are px@comp points. Polar coordinates declares none at all: its
    // centre and its radius scale are both functions of the raster the kernel is
    // handed (§3.39's precedent), and its Interpolation is a per cent.
    // Wave 2's Distort II batch (docs/08 §3.53-§3.57) brings twenty-eight more:
    // three % diag and twenty-five px@comp. **Ripple's Radius, Wave height and
    // Wave width** are % diag — the whole effect is a reach into the picture, so
    // all three have to travel together or a resize would change the ripple's
    // shape rather than its size; its centre is px@comp. **Wave warp's two
    // lengths** are px@comp (AE's are raster pixels), and its Direction and
    // Phase are angles. **Bezier warp's twenty-four point coordinates** are
    // pixels as point pairs and must travel together, exactly as Corner
    // pin's eight do. **Roughen edges' Border, Scale and Offset** are lengths
    // for §3.37 decision 1's reason a fourth time — its Edge sharpness and
    // Fractal influence are per cents and are not here. Warp declares none at
    // all: every one of its controls is a per cent of the frame's own extent,
    // which the kernel derives from the raster it is handed (§3.39's
    // precedent).
    //
    // Wave 2's Stylise I batch (docs/08 §3.58-§3.63) brings exactly one, and it
    // is a % diag: **Shadow highlight's Radius**, which is the Gaussian blur's
    // own control under another name — how large a neighbourhood decides whether
    // a pixel is in shadow — and so carries the blur's unit and the blur's
    // default. The other five effects in the batch are pointwise and declare
    // none: a rung, a cut, a stop, a density and six channel weights are all
    // positions on the tone range, which has no size.
    //
    // Wave 2's Stylise II batch (docs/08 §3.64-§3.69) brings three, all px@comp
    // and all deliberately pixel-scale looks. **Median's Radius** is the size of
    // the neighbourhood being voted over — a Half-resolution preview must vote
    // over half as many raster pixels to despeckle the same picture. **Emboss's
    // and Texturize's Relief** are the separation between the two taps that make
    // the relief, which is exactly the kind of pixel-scale look §2.3 names.
    // Mosaic declares none: its two block counts are counts, and the block's
    // size in pixels is derived from the raster the kernel is handed (§3.39's
    // precedent). Find edges and Broadcast safe are pointwise.
    //
    // The Matte key's spatial controls bring three. **Screen pre-blur**
    // is a blur radius like any other; **Screen shrink/grow** is how far the
    // matte's edge marches, which must be the same distance in the picture at
    // any preview resolution; **Screen softness** is a blur radius again. Its
    // two garbage-mask rows declare none, for the reason the three line-drawing
    // effects' rows declare none: the geometry is flattened once in px@comp and
    // each consumer takes it to its own raster. Despot black and white are per
    // cent and reach exactly one pixel by definition, so neither is a distance
    // a preview could get wrong.
    //
    // The two path consumers (docs/08 §3.78-§3.79) bring four, all px@comp and
    // all pixel-scale looks. **Scribble's Stroke width, Spacing and Path
    // overlap** are the pencil's own dimensions and must travel together, or a
    // half-resolution preview would draw a hatch of a different density from
    // the export's. **Stroke's Brush size** is Vegas' Width under another name.
    // Neither declares one for the mask's own vertices, and that is the point of
    // the tolerance being a constant: the polyline is flattened once in
    // px@comp and each consumer takes it to its own raster, so the geometry
    // cannot acquire a second unit. Stroke's Spacing is a per cent *of the
    // brush*, so it rides on Brush size and is not here.
    //
    // The Controls family brings two more, and they are the first pair
    // here that never *reaches* the rescale pass: a Point control draws
    // nothing, so it resolves to no op at all. They are declared px@comp all
    // the same, because that is what the numbers mean and because what
    // reads them through an expression is going to put them in a picture.
    //
    // **Points sample's Position** is the last pair, and the second
    // that never reaches the rescale: a driver resolves at px@comp always, so
    // its query point and the stream it searches are in the same units by
    // construction, whatever raster the preview is drawn at. Declared px@comp
    // for the same reason as the Point control's — that is what the number
    // means, and the distance it answers with lands in a picture.
    assert_eq!(
        spatial,
        vec![
            ("blur", "radius"),
            ("directional_blur", "length"),
            ("radial_blur", "amount"),
            // Radial blur's centre is the last point to stop being a
            // per cent of the frame, so it joins the pass that follows the
            // raster.
            ("radial_blur", "centre_x"),
            ("radial_blur", "centre_y"),
            ("sharpen", "radius"),
            ("sprite_flare", "light_x"),
            ("sprite_flare", "light_y"),
            ("sprite_flare", "glow_size"),
            ("sprite_flare", "ghost_size"),
            ("sprite_flare", "streak_length"),
            ("light_wrap", "width"),
            ("rgb_split", "amount"),
            ("chromatic_aberration", "amount"),
            ("dof", "focus_point_x"),
            ("dof", "focus_point_y"),
            ("dof", "near_aperture"),
            ("dof", "far_aperture"),
            ("channel_blur", "red"),
            ("channel_blur", "green"),
            ("channel_blur", "blue"),
            ("channel_blur", "alpha"),
            ("transform", "anchor_x"),
            ("transform", "anchor_y"),
            ("transform", "position_x"),
            ("transform", "position_y"),
            ("glow", "radius"),
            ("shake", "amplitude"),
            ("block_glitch", "block_size"),
            ("block_glitch", "block_amount"),
            ("block_glitch", "channel_offset"),
            ("scanlines", "scanline_period"),
            ("turbulent_displace", "amount"),
            ("turbulent_displace", "size"),
            ("turbulent_displace", "offset_x"),
            ("turbulent_displace", "offset_y"),
            ("tile", "tile_centre_x"),
            ("tile", "tile_centre_y"),
            // The tile's own size and the output window's size are
            // sizes, so they are distances and follow the raster too.
            ("tile", "tile_width"),
            ("tile", "tile_height"),
            ("tile", "output_width"),
            ("tile", "output_height"),
            ("offset", "shift_x"),
            ("offset", "shift_y"),
            ("mirror", "centre_x"),
            ("mirror", "centre_y"),
            ("lens_distort", "centre_x"),
            ("lens_distort", "centre_y"),
            ("corner_pin", "upper_left_x"),
            ("corner_pin", "upper_left_y"),
            ("corner_pin", "upper_right_x"),
            ("corner_pin", "upper_right_y"),
            ("corner_pin", "lower_left_x"),
            ("corner_pin", "lower_left_y"),
            ("corner_pin", "lower_right_x"),
            ("corner_pin", "lower_right_y"),
            ("displacement_map", "horizontal_amount"),
            ("displacement_map", "vertical_amount"),
            ("twirl", "radius"),
            ("twirl", "centre_x"),
            ("twirl", "centre_y"),
            ("spherize", "radius"),
            ("spherize", "centre_x"),
            ("spherize", "centre_y"),
            ("ripple", "radius"),
            ("ripple", "centre_x"),
            ("ripple", "centre_y"),
            ("ripple", "wave_height"),
            ("ripple", "wave_width"),
            ("wave_warp", "wave_height"),
            ("wave_warp", "wave_width"),
            ("bezier_warp", "upper_left_x"),
            ("bezier_warp", "upper_left_y"),
            ("bezier_warp", "upper_right_x"),
            ("bezier_warp", "upper_right_y"),
            ("bezier_warp", "lower_right_x"),
            ("bezier_warp", "lower_right_y"),
            ("bezier_warp", "lower_left_x"),
            ("bezier_warp", "lower_left_y"),
            ("bezier_warp", "top_left_tangent_x"),
            ("bezier_warp", "top_left_tangent_y"),
            ("bezier_warp", "top_right_tangent_x"),
            ("bezier_warp", "top_right_tangent_y"),
            ("bezier_warp", "right_top_tangent_x"),
            ("bezier_warp", "right_top_tangent_y"),
            ("bezier_warp", "right_bottom_tangent_x"),
            ("bezier_warp", "right_bottom_tangent_y"),
            ("bezier_warp", "bottom_left_tangent_x"),
            ("bezier_warp", "bottom_left_tangent_y"),
            ("bezier_warp", "bottom_right_tangent_x"),
            ("bezier_warp", "bottom_right_tangent_y"),
            ("bezier_warp", "left_top_tangent_x"),
            ("bezier_warp", "left_top_tangent_y"),
            ("bezier_warp", "left_bottom_tangent_x"),
            ("bezier_warp", "left_bottom_tangent_y"),
            ("gradient", "start_x"),
            ("gradient", "start_y"),
            ("gradient", "end_x"),
            ("gradient", "end_y"),
            ("fractal_noise", "scale"),
            ("fractal_noise", "scale_width"),
            ("fractal_noise", "scale_height"),
            ("fractal_noise", "offset_x"),
            ("fractal_noise", "offset_y"),
            ("beam", "start_x"),
            ("beam", "start_y"),
            ("beam", "end_x"),
            ("beam", "end_y"),
            ("beam", "length"),
            ("beam", "start_thickness"),
            ("beam", "end_thickness"),
            ("lightning", "origin_x"),
            ("lightning", "origin_y"),
            ("lightning", "direction_x"),
            ("lightning", "direction_y"),
            ("lightning", "core_radius"),
            ("lightning", "glow_radius"),
            ("radio_waves", "centre_x"),
            ("radio_waves", "centre_y"),
            ("radio_waves", "expansion"),
            ("radio_waves", "stroke_width"),
            ("vegas", "width"),
            ("vegas", "segment_length"),
            ("add_grain", "size"),
            ("scribble", "stroke_width"),
            ("scribble", "spacing"),
            ("scribble", "path_overlap"),
            ("stroke", "brush_size"),
            // Particulate (px@comp through a particle system): eleven, all
            // px@comp, and every one of them has to follow the raster or a
            // half-resolution preview would show a different picture from the
            // export. The **emitter's** position and extents place the births;
            // **Initial speed**, **Gravity** and the two **Wind** components
            // are lengths per second and per second² — a speed that did not
            // scale would fling particles twice as far across a preview — and
            // **Size** is the disc's own diameter. **Turbulence amount** is a
            // displacement and **Turbulence scale** the wavelength it is
            // measured against, so the pair travels together or the noise
            // changes shape rather than size. Emit rate, the jitters, Drag and
            // Turbulence speed are counts, per cents and rates: no length in
            // any of them, and none is here.
            ("particulate", "position_x"),
            ("particulate", "position_y"),
            // The third axis: a depth and an extent through the plane
            // are lengths like the two beside them, and rescale with the
            // raster for the same reason.
            ("particulate", "position_z"),
            ("particulate", "width"),
            ("particulate", "height"),
            ("particulate", "depth"),
            ("particulate", "initial_speed"),
            ("particulate", "size"),
            ("particulate", "gravity"),
            ("particulate", "wind_x"),
            ("particulate", "wind_y"),
            ("particulate", "wind_z"),
            ("particulate", "turbulence_amount"),
            ("particulate", "turbulence_scale"),
            // Grid: every length in a lattice — the gaps between
            // cells, where its centre sits, how far a cell may wander, and the
            // disc a point is drawn as. The three counts and the per cents are
            // not lengths and are not here.
            ("grid", "spacing_x"),
            ("grid", "spacing_y"),
            ("grid", "spacing_z"),
            ("grid", "position_x"),
            ("grid", "position_y"),
            ("grid", "position_z"),
            ("grid", "jitter_x"),
            ("grid", "jitter_y"),
            ("grid", "jitter_z"),
            ("grid", "size"),
            // Scatter: the disc a point is drawn as. Density is a count
            // per composition area and rescales nowhere — it is measured
            // against the comp, not against the raster.
            ("scatter", "size"),
            // Emit from image: the disc a point is drawn as, Scatter's
            // row exactly. Threshold is a share of full white and Density a
            // count per composition area, so neither rescales.
            ("emit_from_image", "size"),
            // Points along path: the disc a point is drawn as, Grid's row.
            ("points_along_path", "size"),
            // Connect points: how far apart two points may be and
            // still be joined, and how thick the line between them is. Both
            // are distances in the picture and must travel with the stream —
            // which is rescaled beside them — or a half-resolution preview
            // would weave a different web from the export's.
            ("connect_points", "max_distance"),
            ("connect_points", "width"),
            // Vary points and Pick points: the pattern's own distances, and
            // how far Vary points moves a point. All travel with the stream.
            ("vary_points", "noise_scale"),
            ("vary_points", "centre_x"),
            ("vary_points", "centre_y"),
            ("vary_points", "radius"),
            ("vary_points", "offset_x"),
            ("vary_points", "offset_y"),
            ("vary_points", "offset_z"),
            ("pick_points", "noise_scale"),
            ("pick_points", "centre_x"),
            ("pick_points", "centre_y"),
            ("pick_points", "radius"),
            // What a full channel of a Motion vectors layer means, in pixels
            // of movement.
            ("motion_blur", "vector_scale"),
            ("matte_key", "pre_blur"),
            ("matte_key", "shrink_grow"),
            ("matte_key", "softness"),
            // Planar track's quad is four points in px@comp, exactly as
            // the Corner pin it writes is — and for the extra reason that they
            // are what the analysis is *given*: a quad measured against the
            // wrong raster would follow the wrong patch of picture.
            ("planar_track", "upper_left_x"),
            ("planar_track", "upper_left_y"),
            ("planar_track", "upper_right_x"),
            ("planar_track", "upper_right_y"),
            ("planar_track", "lower_left_x"),
            ("planar_track", "lower_left_y"),
            ("planar_track", "lower_right_x"),
            ("planar_track", "lower_right_y"),
            // Its two search points and their box are the same numbers
            // asked the same question, one patch at a time.
            ("planar_track", "point1_x"),
            ("planar_track", "point1_y"),
            ("planar_track", "point2_x"),
            ("planar_track", "point2_y"),
            ("planar_track", "region"),
            ("shadow_highlight", "radius"),
            ("lens_flare", "light_x"),
            ("lens_flare", "light_y"),
            ("lens_flare", "source_width"),
            ("lens_flare", "source_height"),
            // The ghost blur's radius is a distance, and the one remaining
            // per cent of the diagonal was this one.
            ("lens_flare", "ghost_softness"),
            ("drop_shadow", "distance"),
            ("drop_shadow", "softness"),
            ("roughen_edges", "border"),
            ("roughen_edges", "scale"),
            ("roughen_edges", "offset_x"),
            ("roughen_edges", "offset_y"),
            ("median", "radius"),
            ("emboss", "relief"),
            ("texturize", "relief"),
            ("mood_lighting", "scale"),
            ("pixel_sort", "max_span"),
            ("linear_wipe", "centre_x"),
            ("linear_wipe", "centre_y"),
            ("linear_wipe", "feather"),
            ("radial_wipe", "centre_x"),
            ("radial_wipe", "centre_y"),
            ("radial_wipe", "feather"),
            ("venetian_blinds", "width"),
            ("venetian_blinds", "feather"),
            ("iris_wipe", "centre_x"),
            ("iris_wipe", "centre_y"),
            ("iris_wipe", "outer_radius"),
            ("iris_wipe", "inner_radius"),
            ("iris_wipe", "feather"),
            // The flipping wave's width is a distance across the frame,
            // so it follows the raster like every other distance.
            ("card_wipe", "transition_width"),
            ("point_control", "point_x"),
            ("point_control", "point_y"),
            ("points_sample", "position_x"),
            ("points_sample", "position_y"),
        ]
    );
    // Both are px@comp, multiplied by the preview factor: RGB split's
    // Amount was a % diag until the owner's ruling and must not drift back.
    let unit_of = |name: &str, id: &str| {
        BUILTIN_DEFS
            .get(name)
            .and_then(|d| d.schema().params.iter().find(|p| p.id == id))
            .map(|p| p.unit)
    };
    assert_eq!(unit_of("rgb_split", "amount"), Some(Unit::Px));
    assert_eq!(unit_of("chromatic_aberration", "amount"), Some(Unit::Px));
}

/// **No parameter is a percentage of the composition diagonal** (the
/// owner's rule: every distance, radius and displacement is px@comp, and the
/// resolve step scales it to the raster in play). `Unit::PctDiag` stays in the
/// enum for the ROI declarations and the reference format, but a parameter
/// declared in it is a defect this test catches.
#[test]
fn no_parameter_is_a_per_cent_of_the_diagonal() {
    let offenders: Vec<(&str, &str)> = BUILTIN_DEFS
        .iter()
        .flat_map(|d| {
            d.schema()
                .params
                .iter()
                .map(move |p| (d.schema().match_name, p))
        })
        .filter(|(_, p)| p.unit == Unit::PctDiag)
        .map(|(name, p)| (name, p.id))
        .collect();
    assert!(
        offenders.is_empty(),
        "parameters declared PctDiag, which is forbidden: {offenders:?}"
    );
}

/// **Every padding covers its effect's own hard maximum**. The old
/// declaration was 25 % of the comp diagonal — 551 pixels on a 1080p frame, a
/// quarter of Gaussian blur's 2 000 px hard maximum — so a typed radius clipped
/// at the tile edge. The last assertion is that figure, and fails against it.
#[test]
fn the_roi_padding_covers_the_hard_max_radius_at_1080p() {
    let hard_max = |name: &str, id: &str| -> f64 {
        let p = BUILTIN_DEFS
            .get(name)
            .and_then(|d| d.schema().params.iter().find(|p| p.id == id))
            .unwrap_or_else(|| panic!("{name}.{id} is declared"));
        match p.kind {
            ParamKind::Float {
                hard: (_, Some(max)),
                ..
            }
            | ParamKind::Slider {
                range: (_, max), ..
            } => max,
            _ => panic!("{name}.{id} has no closed hard maximum"),
        }
    };
    let padding = |name: &str| match BUILTIN_DEFS
        .get(name)
        .expect("declared")
        .schema()
        .traits
        .roi
    {
        Roi::PaddedPx(px) => px,
        other => panic!("{name} declares {other:?}, not a pixel padding"),
    };
    for (name, id) in [
        ("blur", "radius"),
        ("channel_blur", "red"),
        ("shadow_highlight", "radius"),
        ("rgb_split", "amount"),
        ("sharpen", "radius"),
        ("median", "radius"),
    ] {
        assert!(
            padding(name) >= hard_max(name, id) as f32,
            "{name}'s padding must cover its own hard maximum"
        );
    }
    assert!(
        0.25 * 1920f32.hypot(1080.0) < hard_max("blur", "radius") as f32,
        "25 % of a 1080p diagonal never covered a 2 000 px radius"
    );
}

/// [`Value::Vec4`] is a kind of its own: its tag is distinct from every
/// other kind's, it is fed to the frame key as tag + four floats, and it reads
/// back through [`Params::vec4`] — never through the Colour accessor, and never
/// the other way about.
#[test]
fn a_vec4_is_its_own_kind_and_reads_back_whole() {
    // Tag distinctness, stated over the whole set rather than pairwise by hand:
    // a new kind that forgets its own tag fails here.
    let kinds = [
        Value::Float(1.0),
        Value::Int(1),
        Value::Bool(true),
        Value::Choice(1),
        Value::Colour([1.0; 4]),
        Value::Layer(true),
        Value::File(1),
        Value::Vec4([1.0; 4]),
        Value::MaskPath(true),
        Value::Curve(CurvePoints::IDENTITY),
    ];
    let mut tags: Vec<u8> = Vec::new();
    for k in kinds {
        // The tag is private, so read it the way the frame key does: the byte
        // that follows the id in the fed stream.
        let mut stack = ResolvedStack::new();
        stack.begin(&effects::invert::InvertDef, Uuid::nil());
        stack.push(ParamId::new("x"), k);
        let mut bytes: Vec<u8> = Vec::new();
        stack.feed_hash(&mut |b| bytes.extend_from_slice(b));
        let tag = *bytes.get(bytes.len() - 1 - payload_len(k)).expect("a tag");
        assert!(!tags.contains(&tag), "two kinds share tag {tag}");
        tags.push(tag);
    }

    // Read-back: the exact four floats, in order.
    let v = [1.5f32, -2.5, 0.0, 7.25];
    let id = ParamId::new("derived.something");
    let entries = [(id, Value::Vec4(v))];
    let p = Params::new(&entries);
    assert_eq!(p.vec4(id, [0.0; 4]), v);
    // Absent, and present-but-another-kind, both fall back to the default —
    // the same rule every typed reader follows.
    assert_eq!(p.vec4(ParamId::new("nothing"), [9.0; 4]), [9.0; 4]);
    let wrong = [(id, Value::Colour(v))];
    assert_eq!(
        Params::new(&wrong).vec4(id, [9.0; 4]),
        [9.0; 4],
        "a Colour is not a Vec4"
    );
    // `as_f32` gives the first component, as it does for a Colour.
    assert_eq!(Value::Vec4(v).as_f32(), 1.5);
}

/// How many payload bytes [`ResolvedStack::feed_hash`] writes for a value —
/// the test above walks back over them to reach the tag byte.
fn payload_len(v: Value) -> usize {
    match v {
        Value::Bool(_) | Value::Layer(_) | Value::MaskPath(_) => 1,
        Value::Float(_) | Value::Int(_) | Value::Choice(_) | Value::File(_) => 4,
        Value::Colour(_) | Value::Vec4(_) => 16,
        // A length, then two floats a live point — the unused tail of
        // the fixed array is padding by another name and never feeds a key.
        Value::Curve(c) => 4 + 8 * c.points().len(),
    }
}

/// Every migrated effect, dispatched through the registry, renders exactly what
/// the old `Resolved` arm rendered — the acceptance criterion for a batch.
///
/// The old arms are deleted, so the numbers they used to compute are written out
/// here as literal calls to the same `cpu::` reference. That is the port made
/// checkable: the left-hand side goes the whole way round the new path (arena →
/// [`cpu::apply_stack`] → [`EffectDef::apply_cpu`] → the effect's `packed`), and
/// the right-hand side is the arm's arithmetic transcribed by hand. If the
/// migration changed a clamp, a divisor or a formula, these disagree.
#[test]
fn every_migrated_effect_renders_what_the_old_dispatch_rendered() {
    let source: Vec<f32> = (0..64).map(|i| (i % 17) as f32 / 17.0).collect();
    let both =
        |def: &'static dyn EffectDef, entries: &[(ParamId, Value)], old: &dyn Fn(&mut Vec<f32>)| {
            let mut new = source.clone();
            let mut ops = ResolvedStack::new();
            ops.begin(def, uuid::Uuid::now_v7());
            for (id, value) in entries {
                ops.push(*id, *value);
            }
            cpu::apply_stack(&mut new, 4, 4, &ops);

            let mut legacy = source.clone();
            old(&mut legacy);
            assert_eq!(
                new,
                legacy,
                "{} renders differently through the registry",
                def.schema().match_name
            );
        };

    both(
        &effects::saturation::SaturationDef,
        &[
            (
                effects::saturation::Saturation::SATURATION,
                Value::Float(250.0),
            ),
            (effects::saturation::Saturation::MIX, Value::Float(80.0)),
        ],
        &|p| cpu::saturate(p, 2.5, 0.8),
    );
    both(
        &effects::vibrancy::VibrancyDef,
        &[
            (effects::vibrancy::Vibrancy::AMOUNT, Value::Float(120.0)),
            (effects::vibrancy::Vibrancy::MIX, Value::Float(100.0)),
        ],
        &|p| cpu::vibrance(p, 1.2, 1.0),
    );
    both(
        &effects::exposure::ExposureDef,
        &[(effects::exposure::Exposure::STOPS, Value::Float(1.5))],
        // The old arm's `2f64.powf(stops) as f32`, at the same stops.
        &|p| cpu::exposure(p, 2f64.powf(1.5) as f32, 1.0),
    );
    both(
        &effects::contrast::ContrastDef,
        &[(effects::contrast::Contrast::CONTRAST, Value::Float(160.0))],
        // 1.36, not 1.6: the factor is quadratic in the distance from
        // neutral. The kernel this transcribes is untouched - only what the
        // effect resolves to before it moved.
        &|p| cpu::contrast(p, 1.36, 1.0),
    );
    both(
        &effects::gamma::GammaDef,
        &[(effects::gamma::Gamma::GAMMA, Value::Float(2.2))],
        &|p| cpu::gamma(p, 2.2, 1.0),
    );
    both(
        &effects::temperature::TemperatureDef,
        &[(
            effects::temperature::Temperature::TEMPERATURE,
            Value::Float(60.0),
        )],
        // The old arm's gains: k = 60/100, 1 ± 0.75·k, floored at 0.
        &|p| cpu::temperature(p, 1.0 + 0.75 * 0.6, 1.0 - 0.75 * 0.6, 1.0),
    );
    both(
        &effects::hue_shift::HueShiftDef,
        &[(effects::hue_shift::HueShift::ANGLE, Value::Float(120.0))],
        &|p| cpu::hue_shift(p, hue_matrix(120.0), 1.0),
    );
    both(
        &effects::tint::TintDef,
        &[
            (
                effects::tint::Tint::BLACK,
                Value::Colour([0.05, 0.0, 0.1, 1.0]),
            ),
            (
                effects::tint::Tint::WHITE,
                Value::Colour([1.0, 0.9, 0.6, 1.0]),
            ),
        ],
        &|p| cpu::tint(p, [0.05, 0.0, 0.1], [1.0, 0.9, 0.6], 1.0),
    );
    both(
        // Flash's strength is derived rather than declared, so it goes
        // into the bag here the way `resolve_derived` puts it there.
        &effects::flash::FlashDef,
        &[
            (
                effects::flash::Flash::COLOUR,
                Value::Colour([1.0, 0.8, 0.5, 1.0]),
            ),
            (effects::flash::Flash::MIX, Value::Float(50.0)),
            (effects::flash::Flash::DERIVED_STRENGTH, Value::Float(0.6)),
        ],
        &|p| cpu::flash(p, 0.6, [1.0, 0.8, 0.5, 1.0], 0.5),
    );
    both(
        &effects::glow::GlowDef,
        &[
            (effects::glow::Glow::THRESHOLD, Value::Float(0.2)),
            (effects::glow::Glow::KNEE, Value::Float(0.5)),
            // Already through the preview factor, as the bag carries it.
            (effects::glow::Glow::RADIUS, Value::Float(2.0)),
            (effects::glow::Glow::INTENSITY, Value::Float(1.5)),
            (
                effects::glow::Glow::TINT,
                Value::Colour([1.0, 0.8, 0.5, 1.0]),
            ),
            (effects::glow::Glow::MIX, Value::Float(60.0)),
        ],
        &|p| cpu::glow(p, 4, 4, 2.0, 0.2, 0.5, 1.5, [1.0, 0.8, 0.5, 1.0], 0.6, &[]),
    );
    both(
        &effects::transform::TransformDef,
        &[
            (effects::transform::Transform::ANCHOR_X, Value::Float(2.0)),
            (effects::transform::Transform::ANCHOR_Y, Value::Float(2.0)),
            (effects::transform::Transform::POSITION_X, Value::Float(3.0)),
            (effects::transform::Transform::POSITION_Y, Value::Float(1.0)),
            (effects::transform::Transform::SCALE_X, Value::Float(200.0)),
            (effects::transform::Transform::SCALE_Y, Value::Float(50.0)),
            (effects::transform::Transform::ROTATION, Value::Float(30.0)),
            (effects::transform::Transform::OPACITY, Value::Float(80.0)),
            (effects::transform::Transform::MIX, Value::Float(75.0)),
        ],
        // The old arm's `px`/`pct` helpers, and the Transform effect's fixed
        // transparent edge.
        &|p| {
            cpu::transform(
                p,
                4,
                4,
                [2.0, 2.0],
                [3.0, 1.0],
                [2.0, 0.5],
                30.0,
                NO_SKEW,
                0,
                0.8,
                0.75,
            )
        },
    );
    both(
        &effects::sprite_flare::SpriteFlareDef,
        &[
            (
                effects::sprite_flare::SpriteFlare::LIGHT_X,
                Value::Float(1.0),
            ),
            (
                effects::sprite_flare::SpriteFlare::LIGHT_Y,
                Value::Float(2.0),
            ),
            (
                effects::sprite_flare::SpriteFlare::INTENSITY,
                Value::Float(1.0),
            ),
            (
                effects::sprite_flare::SpriteFlare::TINT,
                Value::Colour([1.0, 0.5, 0.25, 1.0]),
            ),
            (
                effects::sprite_flare::SpriteFlare::GLOW_SIZE,
                Value::Float(3.0),
            ),
            (
                effects::sprite_flare::SpriteFlare::GLOW_INTENSITY,
                Value::Float(1.0),
            ),
            (effects::sprite_flare::SpriteFlare::GHOSTS, Value::Int(3)),
            (
                effects::sprite_flare::SpriteFlare::GHOST_SPACING,
                Value::Float(0.4),
            ),
            (
                effects::sprite_flare::SpriteFlare::GHOST_SIZE,
                Value::Float(2.0),
            ),
            (
                effects::sprite_flare::SpriteFlare::GHOST_INTENSITY,
                Value::Float(0.5),
            ),
            (
                effects::sprite_flare::SpriteFlare::STREAK_LENGTH,
                Value::Float(4.0),
            ),
            (
                effects::sprite_flare::SpriteFlare::STREAK_INTENSITY,
                Value::Float(0.6),
            ),
            (
                effects::sprite_flare::SpriteFlare::STREAK_ANGLE,
                Value::Float(15.0),
            ),
            (effects::sprite_flare::SpriteFlare::MIX, Value::Float(50.0)),
        ],
        &|p| {
            cpu::sprite_flare(
                p,
                4,
                4,
                &cpu::SpriteFlareParams {
                    light: [1.0, 2.0],
                    intensity: 1.0,
                    tint: [1.0, 0.5, 0.25],
                    glow_size: 3.0,
                    glow_intensity: 1.0,
                    ghosts: 3,
                    ghost_spacing: 0.4,
                    ghost_size: 2.0,
                    ghost_intensity: 0.5,
                    streak_length: 4.0,
                    streak_intensity: 0.6,
                    streak_angle_deg: 15.0,
                    mix: 0.5,
                },
            )
        },
    );
    both(
        // Block glitch's tick is derived rather than declared, so it
        // goes into the bag here the way `resolve_derived` puts it there.
        &effects::block_glitch::BlockGlitchDef,
        &[
            (
                effects::block_glitch::BlockGlitch::INTENSITY,
                Value::Float(0.5),
            ),
            (
                effects::block_glitch::BlockGlitch::BLOCK_SIZE,
                Value::Float(3.0),
            ),
            (
                effects::block_glitch::BlockGlitch::BLOCK_JITTER,
                Value::Float(40.0),
            ),
            (
                effects::block_glitch::BlockGlitch::BLOCK_AMOUNT,
                Value::Float(2.0),
            ),
            (
                effects::block_glitch::BlockGlitch::CHANNEL_OFFSET,
                Value::Float(1.0),
            ),
            (
                effects::block_glitch::BlockGlitch::SLICE_REPEAT,
                Value::Float(30.0),
            ),
            (effects::block_glitch::BlockGlitch::SEED, Value::Int(7)),
            (effects::block_glitch::BlockGlitch::MIX, Value::Float(80.0)),
            (
                effects::block_glitch::BlockGlitch::DERIVED_TICK,
                Value::Int(3),
            ),
        ],
        &|p| cpu::block_glitch(p, 4, 4, 0.5, 7, 3, 3.0, 0.4, 2.0, 1.0, 0.3, 0.8),
    );
    both(
        // Scanlines' folded intensity and roll offset are derived too.
        &effects::scanlines::ScanlinesDef,
        &[
            (effects::scanlines::Scanlines::INTENSITY, Value::Float(0.9)),
            (
                effects::scanlines::Scanlines::SCANLINE_PERIOD,
                Value::Float(2.0),
            ),
            (
                effects::scanlines::Scanlines::SCANLINE_ROLL,
                Value::Float(3.0),
            ),
            (
                effects::scanlines::Scanlines::SCANLINE_INTERLACE,
                Value::Bool(true),
            ),
            (effects::scanlines::Scanlines::MIX, Value::Float(50.0)),
            (
                effects::scanlines::Scanlines::DERIVED_INTENSITY,
                Value::Float(0.6),
            ),
            (
                effects::scanlines::Scanlines::DERIVED_ROLL_PX,
                Value::Float(1.5),
            ),
        ],
        // The derived pair wins over the declared Intensity and Roll speed —
        // which is the whole point of them.
        &|p| cpu::scanlines(p, 4, 4, 0.6, 2.0, 1.5, true, 0.5),
    );
    both(
        &effects::invert::InvertDef,
        &[(effects::invert::Invert::MIX, Value::Float(100.0))],
        &|p| cpu::invert(p, 1.0),
    );
    both(
        // Matte key is the only one of the side-table batch with a CPU
        // reference to compare at all: the other four (Light wrap, Depth of
        // field, Motion blur, Datamosh) need a second picture the
        // single-buffer dispatcher has not got, so their `apply_cpu` is the
        // identity by design — the same passthrough their `cpu::apply` arms
        // were, and pinned by `the_side_table_batch_stays_a_cpu_passthrough`.
        //
        // Every per-cent dial here is off its default, so the old arm's
        // divisions and clamps are all exercised: 150 % gain is the open-above
        // case, and the two Choice rows go through the wire-code enums.
        &effects::matte_key::MatteKeyDef,
        &[
            (effects::matte_key::MatteKey::VIEW, Value::Choice(1)),
            (
                effects::matte_key::MatteKey::KEY,
                Value::Colour([0.0, 0.7, 0.1, 1.0]),
            ),
            (
                effects::matte_key::MatteKey::SCREEN_GAIN,
                Value::Float(150.0),
            ),
            (
                effects::matte_key::MatteKey::SCREEN_BALANCE,
                Value::Float(40.0),
            ),
            (
                effects::matte_key::MatteKey::DESPILL_BIAS,
                Value::Colour([0.4, 0.5, 0.6, 1.0]),
            ),
            (
                effects::matte_key::MatteKey::ALPHA_BIAS,
                Value::Colour([0.6, 0.5, 0.4, 1.0]),
            ),
            (effects::matte_key::MatteKey::SPILL, Value::Float(80.0)),
            (effects::matte_key::MatteKey::CLIP_BLACK, Value::Float(10.0)),
            (effects::matte_key::MatteKey::CLIP_WHITE, Value::Float(90.0)),
            (
                effects::matte_key::MatteKey::CLIP_ROLLBACK,
                Value::Float(25.0),
            ),
            (
                effects::matte_key::MatteKey::REPLACE_METHOD,
                Value::Choice(1),
            ),
            (
                effects::matte_key::MatteKey::REPLACE_COLOUR,
                Value::Colour([0.3, 0.3, 0.3, 1.0]),
            ),
            (effects::matte_key::MatteKey::MIX, Value::Float(75.0)),
        ],
        // The old arm's arithmetic, transcribed: per cent ÷ 100, gain floored
        // rather than clamped, and both Choice rows normalised through their
        // wire codes.
        &|p| {
            cpu::matte_key(
                p,
                &MatteKeyParams {
                    view: 1,
                    key: [0.0, 0.7, 0.1, 1.0],
                    gain: 1.5,
                    balance: 0.4,
                    despill_bias: [0.4, 0.5, 0.6, 1.0],
                    alpha_bias: [0.6, 0.5, 0.4, 1.0],
                    spill: 0.8,
                    clip_black: 0.1,
                    clip_white: 0.9,
                    clip_rollback: 0.25,
                    pre_blur: 0.0,
                    shrink_grow: 0.0,
                    softness: 0.0,
                    despot_black: 0.0,
                    despot_white: 0.0,
                    replace_method: 1,
                    replace_colour: [0.3, 0.3, 0.3, 1.0],
                    mix: 0.75,
                },
            )
        },
    );
}

/// **Every effect has a Matte, and nobody had to write it down**.
///
/// The declaration is what makes the row meaningful on all thirty-odd effects
/// from day one, and the whole point of injecting it is that a new effect cannot
/// forget it — so the gate is the catalogue itself, not a list kept beside it.
///
/// The five that *claim* the matte are named here on purpose. Two of them owned
/// the concept first and keep their stored ids: Depth of field's
/// `depth`, the Lens flare's `matte`. Three take the injected row and simply mean
/// something deeper by it: the Gaussian blur scales its radius, the Glow gates
/// its seed, Turbulent displace scales its displacement vector, Set matte makes
/// it the alpha and Displacement map makes it the map itself. Adding an eighth is
/// a deliberate act, and it lands here.
#[test]
fn every_effect_carries_a_matte_row() {
    use crate::fx::MatteRole;
    for def in BUILTIN_DEFS.builtins() {
        let s = def.schema();
        // The Controls family opts out entirely, the Drivers family with it,
        // the **Audio** family with them, and so do the two tracking effects,
        // which are handles for a background analysis rather than image
        // operations. They
        // are the answer to the question `MatteRole::None` was written for: an
        // effect that touches no pixel cannot be driven by a picture, so a
        // Matte row on one would be a control that could never do anything.
        // Every *image* effect below still has to carry one.
        //
        // Audio is the family, not a list of names, because every effect in it
        // processes sound and draws nothing. There is no such thing as an
        // audio effect that wants a matte, and one added tomorrow needs no
        // entry here (docs/impl/audio-effects.md §2). Compositing is the
        // same shape: a Merge and a Switch join pictures the graph walk hands
        // them rather than drawing one.
        if matches!(
            s.category,
            FxCategory::Controls
                | FxCategory::Drivers
                | FxCategory::Audio
                | FxCategory::Compositing
        ) || matches!(s.match_name, "camera_track" | "planar_track")
        {
            assert_eq!(
                s.matte,
                MatteRole::None,
                "{} is a control and takes no matte",
                s.match_name
            );
            assert!(
                !def.is_image_op(),
                "{} declares no matte, so it had better draw nothing",
                s.match_name
            );
            continue;
        }
        // **Five image effects opt out** (the owner's rule for mattes), and
        // each has its own reason.
        //
        // The **Matte key**: a keyer's subject is the picture it keys, and a
        // strength matte over a key is a garbage matte, which is a mask's job.
        //
        // **Set matte**: every Matte row answers "how much of me happens here",
        // and this effect has no answer to give — what it takes from another
        // layer is the coverage itself, which is the whole effect rather than
        // an amount of one. The row it shows is its own source picker, on the
        // ordinary auxiliary-layer carriage that `layer_input` is the predicate
        // for.
        //
        // **Roto brush**: the same answer as Set matte's, arrived at
        // from the other side. What this effect applies IS a coverage — the
        // matte its propagation solved for this source frame — so a second
        // picture saying how much of it happens here would be a coverage laid
        // over a coverage, and the honest place to say "not there" is another
        // stroke.
        //
        // **Depth**: the third of that family. What it draws is a reading of
        // the picture underneath - how far away every pixel is, as a model saw
        // it - and a matte over a reading would gate a measurement, which is
        // not a thing a measurement has an answer to. Where a reading is wanted
        // in part of the frame, the effect that consumes it takes the matte.
        //
        // **Remove background**: Set matte's answer and the Roto brush's, on
        // the tier Depth is on. What it applies IS the coverage a model made of
        // this frame, so a second picture saying how much of it happens here
        // would be a coverage over a coverage, and the honest way to keep part
        // of the background is a mask on the layer.
        //
        // Anything else that wants to opt out is argued for here, in these
        // words, before it may.
        if matches!(
            s.match_name,
            "matte_key" | "set_matte" | "roto_brush" | "depth" | "remove_background"
        ) {
            assert_eq!(
                s.matte,
                MatteRole::None,
                "{} carries no matte row",
                s.match_name
            );
            assert!(
                !s.matte_channel(),
                "{} — no matte row means no injected Channel row either",
                s.match_name
            );
            // The Matte key carries no such row at all; Set matte's is its own,
            // and is the auxiliary layer the render threads to it.
            assert_eq!(
                s.layer_input().is_some(),
                s.match_name == "set_matte",
                "{} — Set matte keeps its source on the layer-input carriage",
                s.match_name
            );
            assert_eq!(
                s.params.iter().any(|p| p.id == MATTE_PARAM),
                s.match_name == "set_matte",
                "{} — only Set matte declares a row under that id, and it is its own",
                s.match_name
            );
            continue;
        }
        // The owner's rule for mattes: the matte scales the amount.
        // Every blur, sharpen and colour effect whose scaled amount is not
        // already a straight lerp of the input claims it; the rest (Tritone,
        // Black and white, Tint, Curves, Levels, Invert, LUT, Broadcast safe,
        // Contrast, Vignette) keep the strength dissolve because scaling
        // their amount IS that dissolve, and Threshold because it has no
        // honest per-pixel form. The Distortion family claims it the same way:
        // the matte scales the displacement, read at the destination
        // pixel. Datamosh stays on the dissolve because scaling its Intensity
        // IS the dissolve to the bit; Tile, Mirror and Polar coordinates have
        // no amount to scale. The Transform effect shares the Shake's kernel
        // but never binds a matte. Generate and Stylise claim it the same way
        // again: the grain's Intensity, the drawn thing's Opacity, the
        // shadow's Opacity, Border, Radius and Relief. Noise, Flash, Sprite
        // flare and Light wrap keep the dissolve there, because each adds a
        // linear amount to the picture and scaling it IS the dissolve; Fill,
        // Gradient, Fractal noise, Beam, Mosaic and Find edges have no amount
        // of their own to scale. Temporal and Transition claim it once more:
        // Echo's Decay, both motion blurs' Shutter angle, and every
        // wipe's Completion — the Iris wipe scaling its radius instead, having
        // no Completion to scale (§3.71: the radius IS the transition).
        // Posterize time keeps the dissolve, holding a time rather than drawing
        // an amount, and so do Transform and Broadcast safe, because scaling
        // their amount IS that dissolve.
        let claims = matches!(
            s.match_name,
            "dof"
                | "rgb_split"
                | "chromatic_aberration"
                | "shake"
                | "block_glitch"
                | "scanlines"
                | "offset"
                | "lens_distort"
                | "corner_pin"
                | "bezier_warp"
                | "twirl"
                | "spherize"
                | "ripple"
                | "wave_warp"
                | "warp"
                | "lens_flare"
                | "blur"
                | "glow"
                | "turbulent_displace"
                | "displacement_map"
                | "directional_blur"
                | "radial_blur"
                | "sharpen"
                | "sharpen_simple"
                | "channel_blur"
                | "exposure"
                | "saturation"
                | "gamma"
                | "temperature"
                | "vibrancy"
                | "hue_shift"
                | "brightness"
                | "colour_balance"
                | "hue_saturation"
                | "photo_filter"
                | "shadow_highlight"
                | "posterize"
                | "threshold"
                | "add_grain"
                | "lightning"
                | "radio_waves"
                | "vegas"
                | "scribble"
                | "stroke"
                | "drop_shadow"
                | "roughen_edges"
                | "median"
                | "emboss"
                | "texturize"
                | "mood_lighting"
                // Pixel sort: the matte is *where the spans are*, which is
                // as deep inside an effect's own maths as a matte gets — it
                // decides which pixels move rather than fading the picture
                // they moved in.
                | "pixel_sort"
                | "echo"
                | "motion_blur"
                | "accumulation_mb"
                | "linear_wipe"
                | "radial_wipe"
                | "venetian_blinds"
                | "iris_wipe"
                | "card_wipe"
                // Scatter: the matte is *where the points go*, which is
                // as deep inside an effect's own maths as a matte gets — it
                // decides the set rather than fading the picture that set drew.
                | "scatter"
        );
        assert_eq!(
            !s.matte.generic(),
            claims,
            "{} — the effects that claim the matte inside their own maths are              listed here; anything else that wants a deeper meaning              must say so here too",
            s.match_name
        );
        // Every image effect takes one, whatever it means by it.
        // `MatteRole::None` is for an effect that genuinely cannot be driven by
        // a picture, and the Controls family above is the whole of that list —
        // anything else that wants to opt out is argued for here.
        let param = s
            .matte
            .param()
            .unwrap_or_else(|| panic!("{} declares no matte at all", s.match_name));
        // An override says what it means, in the schema, which is what the
        // manual's tables print (`fx-reference.json`).
        if let MatteRole::Own { meaning, .. } = s.matte {
            assert!(
                meaning.len() > 40 && !meaning.ends_with('.'),
                "{} — an override must document what its matte means, in one                  sentence without a full stop: {meaning:?}",
                s.match_name
            );
        }
        let row =
            s.params.iter().find(|p| p.id == param).unwrap_or_else(|| {
                panic!("{} names {param} but declares no such row", s.match_name)
            });
        assert_eq!(row.label, "Matte", "{}", s.match_name);
        assert!(
            matches!(row.kind, ParamKind::Layer { .. }),
            "{} — a matte is a layer",
            s.match_name
        );
        // The defaults: unset, and its Invert off. A `self_default = true`
        // on the injected row would point every effect on every layer at its own
        // input on the day it was added, which is a picture change disguised as
        // a default. The flare's own row predates that and keeps its `true` —
        // its stored id and its behaviour are both a save's business.
        if param == MATTE_PARAM && s.match_name != "lens_flare" {
            assert_eq!(
                row.kind,
                ParamKind::Layer {
                    self_default: false
                },
                "{} — a fresh Matte row starts unset",
                s.match_name
            );
        }
        // The Invert rides beside the picker, under the picker's own id — the
        // injected `matte_invert`, or Depth of field's older `depth_invert`. The
        // flare has none, and never had one.
        let Some(invert) = s.params.iter().find(|p| p.id == format!("{param}_invert")) else {
            assert_eq!(
                s.match_name, "lens_flare",
                "{} has a Matte row with no Invert beside it",
                s.match_name
            );
            continue;
        };
        assert_eq!(invert.label, "Invert", "{}", s.match_name);
        assert_eq!(
            invert.kind,
            ParamKind::Bool { default: false },
            "{} — a fresh Invert starts off",
            s.match_name
        );
        // The Channel choice rides beside the injected pair on every
        // effect that does not pick its matte's channels itself. The four that
        // do — Depth of field (`depth_channel`), Displacement map (its two
        // channel choices), the Lens flare (source detection) and Scatter,
        // which reads **alpha and only alpha** — carry none, and the
        // seam leaves their matte raw. Set matte used to be a fifth, and
        // carries no Matte row at all now.
        let owns_channel = matches!(
            s.match_name,
            "dof" | "displacement_map" | "lens_flare" | "scatter"
        );
        let channel = s.params.iter().find(|p| p.id == MATTE_CHANNEL_PARAM);
        assert_eq!(
            channel.is_some(),
            !owns_channel,
            "{} — the Channel row is injected exactly where the effect does not own one",
            s.match_name
        );
        assert_eq!(s.matte_channel(), !owns_channel, "{}", s.match_name);
        if let Some(channel) = channel {
            assert_eq!(channel.label, "Channel", "{}", s.match_name);
            assert_eq!(
                channel.kind,
                ParamKind::Choice {
                    options: CHANNEL_OPTIONS,
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
                "{} — Luminance by default, the reading every kernel had",
                s.match_name
            );
            // Beside the pair, in schema order: picker, Invert, Channel.
            let at = |id: &str| s.params.iter().position(|p| p.id == id).unwrap();
            assert_eq!(
                at(MATTE_CHANNEL_PARAM),
                at(MATTE_INVERT_PARAM) + 1,
                "{}",
                s.match_name
            );
        }
    }
}

/// **Every Mix slider has a Blend beside it**: the layer blend modes,
/// verbatim, Normal by default, injected right after `mix` in schema order so
/// the panel can draw it on the Mix row. The Lens flare declares its own
/// `blend` and keeps it; an effect with no Mix (the Controls, the Camera
/// track, Posterize time) touches no pixel and gets none.
///
/// An **audio** effect's wet/dry row is called `wet`, never `mix`, and this is
/// why: a row named `mix` here would be handed a Blend menu of Screen and
/// Multiply, which are things to do to a picture (docs/impl/audio-effects.md
/// §2). Wet is also the word every mixing desk uses.
#[test]
fn every_mix_row_carries_a_blend() {
    use crate::model::BlendMode;
    for def in BUILTIN_DEFS.builtins() {
        let s = def.schema();
        let mix = s.params.iter().position(|p| p.id == MIX_PARAM);
        let blend = s.params.iter().position(|p| p.id == BLEND_PARAM);
        match mix {
            None => assert!(
                blend.is_none(),
                "{} has no Mix and so nothing to blend",
                s.match_name
            ),
            Some(at) => {
                let at_blend = blend
                    .unwrap_or_else(|| panic!("{} has a Mix and no Blend beside it", s.match_name));
                if s.match_name == "lens_flare" {
                    // Its own, older row: a save is a save. And
                    // therefore NOT the seam's — the flare's combine applies
                    // its own menu, and a second blend out at the seam reads
                    // that index into the layer modes and spends it against
                    // the untouched input, which is how a fresh flare came
                    // back black.
                    assert!(!s.blend(), "the flare's own row is not the injected one");
                    continue;
                }
                assert!(s.blend(), "{}", s.match_name);
                assert_eq!(
                    at_blend,
                    at + 1,
                    "{} — Blend sits right after Mix",
                    s.match_name
                );
                let row = s.params[at_blend];
                assert_eq!(row.label, "Blend", "{}", s.match_name);
                assert_eq!(
                    row.kind,
                    ParamKind::Choice {
                        options: BlendMode::NAMES,
                        default: 0,
                        dividers_after: CHOICE_UNGROUPED,
                    },
                    "{} — the layer modes, Normal by default",
                    s.match_name
                );
            }
        }
    }
    assert_eq!(BlendMode::NAMES[0], "Normal");
}

/// **The blend maths, pinned at its end stops**: Add sums, Multiply
/// multiplies, Normal is the effect's output, and the Mix lerp runs after the
/// blend — so Mix 0 is the input on every mode and Mix 1 the blend alone.
/// Alpha is always the effect's own.
#[test]
fn the_blend_combines_the_effect_with_its_input_and_then_mixes() {
    let d = [0.25, 0.5, 0.75, 1.0];
    let s = [0.5, 0.5, 0.5, 0.5];
    assert_eq!(
        cpu::blend_pixel(0, d, s),
        s,
        "Normal is the effect's output"
    );
    assert_eq!(cpu::blend_pixel(6, d, s), [0.75, 1.0, 1.25, 0.5], "Add");
    assert_eq!(
        cpu::blend_pixel(2, d, s),
        [0.125, 0.25, 0.375, 0.5],
        "Multiply"
    );
    assert_eq!(cpu::blend_pixel(7, d, s), [0.5, 0.5, 0.75, 0.5], "Lighten");
    assert_eq!(cpu::blend_pixel(1, d, s), [0.25, 0.5, 0.5, 0.5], "Darken");
    assert_eq!(
        cpu::blend_pixel(20, d, s),
        [0.0, 0.0, 0.25, 0.5],
        "Subtract"
    );
    // The encoded set: Screen of x with itself is 1 - (1-x)^2 in the encoded
    // domain, which for encoded 0.5 is 0.75 — round-tripped through the curve.
    let e = crate::pixels::srgb_decode(128);
    let screen = cpu::blend_pixel(8, [e, e, e, 1.0], [e, e, e, 1.0]);
    let want = {
        let enc = f32::from(128u8) / 255.0;
        let v = 1.0 - (1.0 - enc) * (1.0 - enc);
        ((v + 0.055) / 1.055f32).powf(2.4)
    };
    assert!(
        (screen[0] - want).abs() < 1e-3,
        "Screen runs encoded: {} vs {want}",
        screen[0]
    );
    // Difference of a pixel with itself is black, whatever the domain.
    assert_eq!(cpu::blend_pixel(18, d, d)[..3], [0.0, 0.0, 0.0]);

    let input = vec![0.25, 0.5, 0.75, 1.0];
    let mut out = vec![0.5, 0.5, 0.5, 0.5];
    cpu::blend_mix(&mut out, &input, 6, 0.0);
    assert_eq!(out, input, "Mix 0 is the input on any mode");
    let mut out = vec![0.5, 0.5, 0.5, 0.5];
    cpu::blend_mix(&mut out, &input, 6, 1.0);
    assert_eq!(out, [0.75, 1.0, 1.25, 0.5], "Mix 1 is the blend alone");
    let mut out = vec![0.5, 0.5, 0.5, 0.5];
    cpu::blend_mix(&mut out, &input, 6, 0.5);
    assert_eq!(out, [0.5, 0.75, 1.0, 0.75], "Mix 0.5 is halfway");
}

/// **The stack applies the blend through the seam**: an Exposure at
/// Blend = Multiply and Mix 50 through `apply_stack` equals the kernel run at
/// Mix 100, multiplied with its input, then lerped to half — and at Normal the
/// kernel's own Mix does the whole job, untouched.
#[test]
fn apply_stack_runs_the_kernel_at_full_mix_and_blends_once() {
    use crate::fx::effects::exposure::Exposure;
    let (w, h) = (2u32, 2u32);
    let img: Vec<f32> = (0..(w * h * 4) as usize)
        .map(|i| {
            if i % 4 == 3 {
                1.0
            } else {
                0.1 + i as f32 * 0.03
            }
        })
        .collect();
    let resolve = |blend: u32| {
        let mut inst = instantiate("exposure").unwrap();
        for p in &mut inst.params {
            if p.id == "stops" {
                p.value = EffectValue::Float(Property::fixed(1.0));
            }
            if p.id == "mix" {
                p.value = EffectValue::Float(Property::fixed(50.0));
            }
            if p.id == BLEND_PARAM {
                p.value = EffectValue::Choice(blend);
            }
        }
        super::resolve_stack(
            &[inst],
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            std::sync::Arc::new(crate::expression::ExpressionContext::detached()),
        )
    };
    let mut normal = img.clone();
    cpu::apply_stack(&mut normal, w, h, &resolve(0));
    let mut direct = img.clone();
    {
        let ops = resolve(0);
        let op = ops.iter().next().unwrap();
        let (stops, mix) = Exposure::read(op.params).packed();
        assert!(
            (mix - 0.5).abs() < 1e-6,
            "the kernel sees the real Mix at Normal"
        );
        cpu::exposure(&mut direct, stops, mix);
    }
    assert_eq!(normal, direct, "Normal is the kernel alone, byte for byte");

    let mut multiplied = img.clone();
    cpu::apply_stack(&mut multiplied, w, h, &resolve(2));
    let mut want = img.clone();
    {
        let ops = resolve(2);
        let op = ops.iter().next().unwrap();
        let (stops, _) = Exposure::read(op.params).packed();
        cpu::exposure(&mut want, stops, 1.0);
        cpu::blend_mix(&mut want, &img, 2, 0.5);
    }
    assert_eq!(
        multiplied, want,
        "Multiply: kernel at Mix 100, blended, then mixed once"
    );
    assert_ne!(multiplied, normal);
}

/// The generic strength semantic itself (docs/08 §2.6), on the CPU side
/// where the maths are readable: white is the effect in full, black is the
/// input untouched, grey is part way, and Invert swaps the ends.
#[test]
fn the_matte_dissolves_the_effect_by_luma() {
    let input = vec![0.2f32, 0.4, 0.6, 1.0];
    let effected = vec![1.0f32, 0.0, 0.5, 0.5];
    let matte = |v: f32| vec![v, v, v, 1.0];

    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &matte(1.0), false);
    assert_eq!(out, effected, "a white matte is today's output, exactly");

    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &matte(0.0), false);
    assert_eq!(out, input, "a black matte is a passthrough, exactly");

    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &matte(1.0), true);
    assert_eq!(out, input, "Invert turns white into the passthrough");

    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &matte(0.0), true);
    assert_eq!(out, effected, "…and black into the effect in full");

    // Half way, and it is the *luma* that drives — a matte that is bright only
    // in blue barely applies the effect at all (Rec. 709 gives blue 7 %).
    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &matte(0.5), false);
    for c in 0..4 {
        assert!(
            (out[c] - (input[c] + effected[c]) * 0.5).abs() < 1e-6,
            "channel {c}: mid grey is half way"
        );
    }
    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &[0.0, 0.0, 1.0, 1.0], false);
    for c in 0..4 {
        let want = input[c] * (1.0 - 0.0722) + effected[c] * 0.0722;
        assert!(
            (out[c] - want).abs() < 1e-6,
            "channel {c}: blue weighs 0.0722"
        );
    }

    // Above white, and below black, both clamp rather than overshooting: an HDR
    // matte cannot drive the effect past its own output.
    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &[8.0, 8.0, 8.0, 1.0], false);
    assert_eq!(out, effected, "an HDR matte clamps at full strength");
    let mut out = effected.clone();
    cpu::matte_mix(&mut out, &input, &[-4.0, -4.0, -4.0, 1.0], false);
    assert_eq!(out, input, "a negative matte clamps at none");
}

/// **Past the budget a chain is coarsened, never cut** (docs/08 §3.78): the
/// whole shape still draws, with fewer and straighter pieces. A truncation
/// would draw half a shape, which is the failure somebody notices.
#[test]
fn a_chain_past_the_budget_coarsens_rather_than_stopping() {
    let n = cpu::PATH_PRIMITIVES * 3;
    let long: Vec<[f32; 2]> = (0..=n).map(|i| [i as f32, 0.0]).collect();
    let mut p = cpu::PathDrawParams::blank();
    cpu::path_chain(&long, 0.0, 100.0, &mut p);
    assert!(p.count as usize <= cpu::PATH_PRIMITIVES, "the budget holds");
    let last = p.segments[p.count as usize - 1][2];
    assert!(
        (last - n as f32).abs() < 1e-3,
        "the coarsened chain must still reach the end, stopped at {last}"
    );
}

/// **A scribble lifts the pen across a hole** (docs/08 §3.78): a line that
/// crosses a notched shape twice must not be joined through the gap.
#[test]
fn a_scribble_lifts_the_pen_across_a_notch() {
    // A U: two uprights and a floor, so a horizontal line high up crosses it
    // twice with nothing in between.
    let u: Vec<[f32; 2]> = vec![
        [0.0, 0.0],
        [30.0, 0.0],
        [30.0, 100.0],
        [70.0, 100.0],
        [70.0, 0.0],
        [100.0, 0.0],
        [100.0, 130.0],
        [0.0, 130.0],
    ];
    let chain = cpu::scribble_chain(&u, 0.0, 10.0, 0.0);
    assert!(
        chain.iter().any(|q| !q[0].is_finite()),
        "a notched shape must lift the pen at least once"
    );

    // And with the lifts honoured, nothing is drawn down the middle of the
    // notch — the hole the U has.
    let mut p = cpu::PathDrawParams::blank();
    cpu::path_chain(&chain, 0.0, 100.0, &mut p);
    p.half_width = 2.0;
    p.opacity = 1.0;
    assert!(p.count > 0, "the U must be hatched");
    // Scanned down a column rather than sampled at a point, because where the
    // strokes fall between the two is the spacing's business and not this
    // test's: what matters is that the notch has none of them and the upright
    // has some.
    let down = |x: f32| {
        (5..95)
            .map(|y| cpu::path_draw_sample(x, y as f32, &p))
            .fold(0.0f32, f32::max)
    };
    assert_eq!(down(50.0), 0.0, "the scribble drew through the notch");
    assert!(down(15.0) > 0.0, "the scribble missed the U's left upright");
    assert!(
        down(85.0) > 0.0,
        "the scribble missed the U's right upright"
    );

    // A convex shape needs no lift at all: the pen crosses, hops down the edge,
    // and comes back, which is what makes it one continuous scribble.
    let square: Vec<[f32; 2]> = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    assert!(
        cpu::scribble_chain(&square, 0.0, 10.0, 0.0)
            .iter()
            .all(|q| q[0].is_finite()),
        "a convex shape must be one unbroken line"
    );
}

/// **A brush stroke is the swept path while its stamps overlap, and separate
/// dots once they do not** (docs/08 §3.79's second decision) — the same picture
/// either way, and the only form that fits a long path with a fine brush.
#[test]
fn a_brush_stroke_changes_shape_when_its_stamps_come_apart() {
    let mut m = crate::mask::Mask::ellipse(200.0, 200.0, 150.0, 150.0);
    m.name = "Ring".into();
    let poly = crate::mask::mask_path_at(std::slice::from_ref(&m), None, true, 0.0);
    assert!(!poly.is_empty(), "the ellipse must flatten to something");

    let mut close = cpu::PathDrawParams::blank();
    cpu::stroke_geometry(&poly, 1.0, 20.0, 3.0, 0.0, 100.0, &mut close);
    // Overlapping: the pieces have length, because they are the path itself.
    let swept = (0..close.count as usize)
        .filter(|&i| {
            (close.segments[i][2] - close.segments[i][0]).abs()
                + (close.segments[i][3] - close.segments[i][1]).abs()
                > 1e-3
        })
        .count();
    assert_eq!(
        swept, close.count as usize,
        "a continuous stroke must be drawn as the path it sweeps"
    );

    let mut apart = cpu::PathDrawParams::blank();
    cpu::stroke_geometry(&poly, 1.0, 20.0, 60.0, 0.0, 100.0, &mut apart);
    // Well apart: every piece is a stamp, with no length at all.
    for i in 0..apart.count as usize {
        let s = apart.segments[i];
        assert_eq!((s[0], s[1]), (s[2], s[3]), "a dot must have no length");
    }
    // And they are spaced by what was asked for, round the path.
    assert!(apart.count >= 2);
    assert!(
        (apart.arcs[1] - apart.arcs[0] - 60.0).abs() < 1e-3,
        "the dots must be laid at the spacing asked for"
    );

    // Start and End trim the dots too, and by distance round the path.
    let mut window = cpu::PathDrawParams::blank();
    cpu::stroke_geometry(&poly, 1.0, 20.0, 60.0, 25.0, 75.0, &mut window);
    assert!(window.count < apart.count, "a window must draw fewer dots");
    assert!(
        window.arcs[0] > poly.length() * 0.24,
        "the first dot must start at the Start mark"
    );

    // An absent mask builds nothing, which is the documented no-op.
    let mut none = cpu::PathDrawParams::blank();
    cpu::stroke_geometry(
        &crate::mask::MaskPolyline::default(),
        1.0,
        20.0,
        3.0,
        0.0,
        100.0,
        &mut none,
    );
    assert_eq!(none.count, 0);
}

/// **The raster factor and the waver's tick reach the bag**. The
/// three path effects each read a number at draw time that no row carries: how
/// many raster pixels a comp pixel is, since the seam hands its vertices over in
/// px@comp, and — for Scribble — where in the waver's evolution this frame sits.
/// Both are pushed by `resolve_derived`, and nothing else in the chain would
/// notice if they stopped being: `packed` would quietly fall back to its
/// defaults and the drawing would come out at the wrong size on a Half preview.
#[test]
fn a_path_effect_is_told_the_raster_and_the_clock_at_resolve() {
    use crate::fx::effects::{scribble::Scribble, stroke::Stroke, vegas::Vegas};

    let at = |name: &str, lt: f64, px_scale: f32| {
        let e = instantiate(name).expect(name);
        resolve_bag(
            std::slice::from_ref(&e),
            lt,
            1000.0,
            px_scale,
            &MarkerContext::NONE,
        )
    };
    let float = |bag: &[(ParamId, Value)], id: ParamId| Params::new(bag).float(id, f32::NAN);

    for (name, id) in [
        ("scribble", Scribble::DERIVED_PX_SCALE),
        ("stroke", Stroke::DERIVED_PX_SCALE),
        ("vegas", Vegas::DERIVED_PX_SCALE),
    ] {
        assert_eq!(
            float(&at(name, 0.0, 0.5), id),
            0.5,
            "{name} was not told the raster"
        );
        assert_eq!(
            float(&at(name, 0.0, 1.0), id),
            1.0,
            "{name} was not told the raster"
        );
    }

    // Scribble's tick: Static holds at nothing whatever the clock says, and the
    // default wiggle type *is* Static, so a fresh instance never moves.
    let tick = |lt: f64| float(&at("scribble", lt, 1.0), Scribble::DERIVED_TICK);
    assert_eq!(tick(0.0), 0.0);
    assert_eq!(
        tick(2.5),
        0.0,
        "a fresh Scribble is Static and must not move"
    );

    // Jagged floors, Wiggly drifts — the one line of arithmetic that separates
    // the three (docs/08 §3.78's third decision), read back through the bag.
    let with_type = |kind: u32, lt: f64| {
        let mut e = instantiate("scribble").expect("scribble");
        for prop in &mut e.params {
            if prop.id == "wiggle_type" {
                prop.value = EffectValue::Choice(kind);
            }
        }
        let bag = resolve_bag(
            std::slice::from_ref(&e),
            lt,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        float(&bag, Scribble::DERIVED_TICK)
    };
    // Wiggles per second defaults to 8, so a quarter of a second is two wiggles.
    assert_eq!(
        with_type(1, 0.25),
        2.0,
        "Jagged must snap on a whole wiggle"
    );
    assert_eq!(with_type(1, 0.30), 2.0, "and hold there until the next one");
    assert!(
        (with_type(2, 0.30) - 2.4).abs() < 1e-4,
        "Wiggly must drift between them"
    );
}

/// **A monotone point set stays inside the unit square**. A cubic
/// through rising points can bulge past the highest of them; a tone curve that
/// climbed above the white the user placed would ring a bright halo into a
/// roll-off, which is what the bake's clamp exists to stop.
#[test]
fn a_monotone_curve_stays_in_the_unit_square() {
    use crate::fx::cpu::curve_table;

    for points in [
        // The overshooting shape: a long flat run, then a sudden rise.
        &[
            [0.0, 0.0],
            [0.1, 0.02],
            [0.5, 0.05],
            [0.6, 0.95],
            [1.0, 1.0],
        ][..],
        // A hard S, and a lifted black under a crushed white.
        &[[0.0, 0.0], [0.25, 0.05], [0.75, 0.95], [1.0, 1.0]][..],
        &[[0.0, 0.2], [0.5, 0.4], [1.0, 0.9]][..],
    ] {
        let table = curve_table(&CurvePoints::sanitised(points));
        for (i, v) in table.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(v),
                "entry {i} of {points:?} left the square: {v}"
            );
        }
        // And it really does pass through the points it was given — read the
        // way the kernels read it, since a control point rarely lands on a
        // table entry and the lookup is what both paths actually see.
        for p in points {
            let at = crate::fx::cpu::curve_at(p[0], &table);
            assert!(
                (at - p[1]).abs() < 2e-3,
                "the curve misses its own point {p:?}: {at}"
            );
        }
    }
}

/// **A malformed point list is straightened, never refused**. Out of
/// order, out of the square, repeated x, too many, too few: each reads to a
/// curve, quietly, because the list comes off a document a hand or an importer
/// wrote and 14-ENGINEERING-RULES §4 forbids a panic on it.
#[test]
fn a_curve_is_sanitised_on_read() {
    // Sorted by x, and the square is the square.
    let messy = CurvePoints::sanitised(&[[0.8, 2.0], [0.2, -1.0], [0.5, 0.5]]);
    assert_eq!(
        messy.points(),
        [[0.2, 0.0], [0.5, 0.5], [0.8, 1.0]],
        "sorted by x and clamped into the unit square"
    );

    // Two points at one x have no curve between them; the first wins.
    let repeated = CurvePoints::sanitised(&[[0.0, 0.0], [0.5, 0.9], [0.5, 0.1], [1.0, 1.0]]);
    assert_eq!(repeated.points(), [[0.0, 0.0], [0.5, 0.9], [1.0, 1.0]]);

    // Past sixteen, the tail is dropped rather than the list refused.
    let many: Vec<[f32; 2]> = (0..40).map(|i| [i as f32 / 39.0, 0.5]).collect();
    assert_eq!(
        CurvePoints::sanitised(&many).points().len(),
        CURVE_MAX_POINTS
    );

    // Fewer than two survivors is not a curve at all: the diagonal stands in.
    assert_eq!(CurvePoints::sanitised(&[]), CurvePoints::IDENTITY);
    assert_eq!(CurvePoints::sanitised(&[[0.4, 0.6]]), CurvePoints::IDENTITY);
    assert_eq!(
        CurvePoints::sanitised(&[[0.4, 0.6], [0.4, 0.2]]),
        CurvePoints::IDENTITY
    );
    // A NaN is a number nobody typed; it reads as zero rather than poisoning
    // the sort and, through it, the whole table.
    assert_eq!(
        CurvePoints::sanitised(&[[f32::NAN, 0.5], [1.0, 1.0]]).points(),
        [[0.0, 0.5], [1.0, 1.0]]
    );
}

/// **A curve parameter resolves through the arena, straightened, and keys a
/// frame**. Curve values are static, so this is the whole of their
/// resolve: the document's list arrives as a [`Value::Curve`], sanitised, and
/// two different curves feed two different hashes.
#[test]
fn a_curve_parameter_resolves_and_feeds_the_key() {
    use crate::fx::effects::curves::Curves;

    let mut e = instantiate("curves").expect("curves");
    assert_eq!(
        e.param("master"),
        Some(&EffectValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]])),
        "a fresh curve is the identity diagonal"
    );

    for p in &mut e.params {
        if p.id == "master" {
            // Deliberately out of order and outside the square.
            p.value = EffectValue::Curve(vec![[1.0, 1.0], [0.5, 1.4], [0.0, 0.0]]);
        }
    }
    let bag = resolve_bag(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        Params::new(&bag).curve(Curves::MASTER).points(),
        [[0.0, 0.0], [0.5, 1.0], [1.0, 1.0]],
        "the arena carries the straightened curve"
    );

    let hash = |fx: &EffectInstance| {
        let stack = super::resolve_stack(
            std::slice::from_ref(fx),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        let mut out: Vec<u8> = Vec::new();
        stack.feed_hash(&mut |b| out.extend_from_slice(b));
        out
    };
    let mut other = e.clone();
    for p in &mut other.params {
        if p.id == "master" {
            p.value = EffectValue::Curve(vec![[0.0, 0.0], [0.5, 0.2], [1.0, 1.0]]);
        }
    }
    assert_ne!(
        hash(&e),
        hash(&other),
        "two different curves must key two different frames"
    );
}

/// **A project saved before an effect dropped a parameter still loads**. The
/// Matte key gave its Matte row up once a keyer was ruled to need none, and
/// Set matte gave the universal row up once the effect that *is* a matte was
/// ruled to carry none — so a save made before either carries three ids the
/// schema no longer declares.
///
/// The forward-migration walk only ever *appends* what a schema has grown, and
/// that is exactly what makes it tolerant here: it never asks whether a stored
/// row is still declared, so a row nobody declares any more is carried along
/// untouched. It is inert on the way out too — the panel draws the schema, and
/// `set_value` answers to declared ids — so the save round-trips rather than
/// being quietly rewritten. That is deliberate, and it is the same courtesy
/// Gaussian blur's unread `mode` and Posterize time's unread `scope` already
/// get; `migrate_lens_flare_background` is the other shape, for a value that
/// had somewhere new to *go*, and neither of these has.
#[test]
fn the_two_keyers_still_load_a_save_that_holds_their_old_matte_rows() {
    use crate::model::{EffectParam, EffectValue};
    for name in ["matte_key", "set_matte"] {
        let mut e = instantiate(name).expect("instantiates");
        // Strip whatever the fresh instance wrote under those ids, then put
        // back what a project saved before the drop would hold.
        e.params
            .retain(|p| !p.id.starts_with(crate::fx::MATTE_PARAM));
        for (id, value) in [
            (crate::fx::MATTE_PARAM, EffectValue::Layer(None)),
            (crate::fx::MATTE_INVERT_PARAM, EffectValue::Bool(true)),
            (crate::fx::MATTE_CHANNEL_PARAM, EffectValue::Choice(3)),
        ] {
            e.params.push(EffectParam {
                id: id.to_owned(),
                value,
                extra: serde_json::Map::new(),
            });
        }
        let mut list = vec![e];
        crate::fx::backfill_builtin_params(&mut list);
        let e = &list[0];

        // Every row the schema declares is present, at a readable value.
        let s = BUILTIN_DEFS.get(name).expect("declared").schema();
        for p in s.params {
            assert!(
                e.param(p.id).is_some(),
                "{name}: the backfill left {} missing",
                p.id
            );
        }
        // The rows the schema dropped are still carried, untouched — a save is
        // a save, and a load that silently threw part of one away would be the
        // worse failure of the two.
        assert_eq!(
            e.param(crate::fx::MATTE_INVERT_PARAM),
            Some(&EffectValue::Bool(true)),
            "{name}: the stored Invert was thrown away"
        );
        assert_eq!(
            e.param(crate::fx::MATTE_CHANNEL_PARAM),
            Some(&EffectValue::Choice(3)),
            "{name}: the dropped Channel was rewritten"
        );

        // And it resolves: the stack builds, the effect keeps its op, and
        // nothing about the undeclared rows reaches it.
        let (ids, ops) = super::resolve_stack_temporal_named(
            &list,
            super::ResolvedDrivers::NONE,
            0.0,
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        assert_eq!(ids.len(), 1, "{name}: the effect resolved to no op");
        assert_eq!(ops.len(), 1, "{name}: the effect resolved to no op");
    }
}

// ---------------------------------------------------------------------------
// Units, vector pairs and the pair link flag
// ---------------------------------------------------------------------------

/// **Every parameter says what its number means.** The panel draws the unit
/// beside the value, so a parameter that never declared one would show a
/// bare number and nobody would notice; the derive answers for the kinds that
/// cannot carry a unit and for a dial, and leaves the numeric kinds to decide,
/// which is what this catches when one forgets.
///
/// `Unit::Unset` is the derive's default for `#[slider]`, `#[bounded]` and
/// `#[counter]`, so this test *is* the gate: it fails with the offenders named.
#[test]
fn every_parameter_declares_a_deliberate_unit() {
    let offenders: Vec<(&str, &str)> = BUILTIN_DEFS
        .iter()
        .flat_map(|d| {
            d.schema()
                .params
                .iter()
                .map(move |p| (d.schema().match_name, p))
        })
        .filter(|(_, p)| p.unit == Unit::Unset)
        .map(|(name, p)| (name, p.id))
        .collect();
    assert!(
        offenders.is_empty(),
        "parameters with no declared unit — add `unit = Px | Percent | Degrees | \
         Seconds | Frames | Raw` to each (Raw is the deliberate 'no unit'): {offenders:?}"
    );
}

/// **Every `_x` is half of a pair the declaration names.** A point is two
/// adjacent Float rows by convention (docs/07 §6.1); [`EffectSchema::pairs`] is
/// where that convention is read now, so an `_x` with no `_y` after it — a typo,
/// a row inserted between the halves — would silently stop being a point, lose
/// its link chain and its crosshair, and look like a plain number instead.
#[test]
fn every_x_parameter_is_half_of_a_declared_pair() {
    let mut pairs_seen = 0;
    for d in BUILTIN_DEFS.builtins() {
        let schema = d.schema();
        let declared: Vec<&str> = schema.pairs().map(|p| p.x).collect();
        for p in schema.params {
            if let Some(stem) = p.id.strip_suffix("_x") {
                assert!(
                    declared.contains(&p.id),
                    "{}.{}: no `{stem}_y` Float directly after it, so the panel \
                     cannot draw the pair",
                    schema.match_name,
                    p.id
                );
            }
            if let Some(stem) = p.id.strip_suffix("_y") {
                assert!(
                    schema.pairs().any(|q| q.stem == stem),
                    "{}.{}: a `_y` with no `{stem}_x` before it",
                    schema.match_name,
                    p.id
                );
            }
        }
        pairs_seen += schema.pairs().count();
    }
    // A walk that suddenly finds nothing is a broken walk, not a catalogue with
    // no points in it.
    assert!(
        pairs_seen >= 40,
        "only {pairs_seen} vector pairs found across the catalogue"
    );
}

/// **A pair starts unlinked, and the flag survives the file.**
///
/// Unlinked is what every project written before the flag existed means, and
/// what it did: two numbers that moved on their own. A document that has never
/// been linked writes no field at all, so an untouched project saves back the
/// same bytes, and a document from before the field loads with every
/// pair unlinked rather than refusing.
#[test]
fn a_vector_pair_link_is_off_by_default_and_survives_the_file() {
    let mut e = instantiate("lens_flare").expect("the flare is a built-in");
    assert!(!e.pair_linked("light"), "a fresh pair is unlinked");
    assert!(e.linked_pairs.is_empty());

    // Nothing linked: nothing written.
    let bare = serde_json::to_value(&e).expect("an instance serialises");
    assert!(
        bare.get("linked_pairs").is_none(),
        "an unlinked instance must not grow a field: {bare}"
    );

    // Linked: written, and read back linked.
    assert!(e.set_pair_linked("light", true), "the toggle changed it");
    assert!(!e.set_pair_linked("light", true), "and is idempotent");
    let saved = serde_json::to_value(&e).expect("an instance serialises");
    let back: crate::model::EffectInstance = serde_json::from_value(saved).expect("it reads back");
    assert!(back.pair_linked("light"));
    assert_eq!(back.linked_pairs, vec!["light".to_owned()]);

    // A file from before the field: every pair unlinked, no error.
    let mut older = serde_json::to_value(&e).expect("an instance serialises");
    older
        .as_object_mut()
        .expect("an object")
        .remove("linked_pairs");
    let old: crate::model::EffectInstance =
        serde_json::from_value(older).expect("an older instance still loads");
    assert!(!old.pair_linked("light"));

    // Unlinking takes the field away again, so the document goes back to the
    // bytes it had before anyone touched the chain.
    let mut relinked = back;
    assert!(relinked.set_pair_linked("light", false));
    assert!(serde_json::to_value(&relinked)
        .expect("an instance serialises")
        .get("linked_pairs")
        .is_none());

    // Two pairs stay sorted whatever order the chains were clicked in, so the
    // same links always save the same bytes.
    let mut both = instantiate("transform").expect("Transform is a built-in");
    both.set_pair_linked("scale", true);
    both.set_pair_linked("anchor", true);
    assert_eq!(
        both.linked_pairs,
        vec!["anchor".to_owned(), "scale".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// Particulate and the points stream
// (docs/impl/particulate.md §9 items 1-7 and 9-11, on the CPU path).
// ---------------------------------------------------------------------------

use crate::fx::effects::particulate::Particulate;
use crate::fx::points::{self, EmitterShape, PointsStream, Schedule};
use crate::mask::MaskPolyline;

/// A Particulate instance with its declared defaults, and `edits` applied —
/// the shape every test below starts from.
fn particulate(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("particulate").expect("Particulate is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("particulate has no {id}"));
        p.value = value.clone();
    }
    e
}

fn fixed(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

/// Everything the closed forms read, from an instance's declared controls.
fn particulate_params(e: &EffectInstance) -> points::PointsParams {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    Particulate::read(Params::new(&bag)).points()
}

/// The stream one instance draws at layer time `t`, at 60 fps.
fn particulate_stream(e: &EffectInstance, t: f64) -> PointsStream {
    let dt = 1.0 / 60.0;
    let bag = resolve_bag(
        std::slice::from_ref(e),
        t,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let p = Particulate::read(Params::new(&bag));
    // The rate is read at each frame the scan walks, keyframes and expressions
    // applied — which is what makes a keyframed Emit rate an ordinary control.
    let rate_at = |lt: f64| {
        e.float_at_with_context("emit_rate", lt, Arc::new(ExpressionContext::detached()))
            .unwrap_or(0.0)
    };
    let sched = Schedule::scan(dt, (t / dt).floor() as i64, p.window_frames(dt), &rate_at);
    points::evaluate(&p.points(), &sched, t, &MaskPolyline::default())
}

/// **Random access** (§9 item 2): frames evaluated out of order equal the same
/// frames evaluated in order. The scrub-safety property, as a test — and the
/// whole reason the closed form was chosen over a simulation.
#[test]
fn particulate_scrubs_in_any_order() {
    let e = particulate(&[]);
    let dt = 1.0 / 60.0;
    let ascending: Vec<PointsStream> = [3, 250, 500]
        .iter()
        .map(|f| particulate_stream(&e, f64::from(*f) * dt))
        .collect();
    // {500, 3, 250, 3} — the note's own order, the last one a repeat.
    let jumped: Vec<PointsStream> = [500, 3, 250, 3]
        .iter()
        .map(|f| particulate_stream(&e, f64::from(*f) * dt))
        .collect();
    assert_eq!(jumped[0], ascending[2]);
    assert_eq!(jumped[1], ascending[0]);
    assert_eq!(jumped[2], ascending[1]);
    assert_eq!(jumped[3], ascending[0]);
    assert!(!ascending[2].is_empty(), "frame 500 has particles");
}

/// **The birth schedule** (§9 item 3), three ways: a constant rate against the
/// closed-form count, a keyframed ramp against a hand-computed table, and a
/// A layer's time comes from a retime curve, and a retime curve is a project
/// file's numbers. "At frame 10 this layer is at ten million seconds" is a thing
/// a keyframe can say, and the scan used to answer it by stepping one frame at a
/// time from the in point — six hundred million steps, each one asking the Emit
/// rate what it is through keyframes, expressions and driver wires, on the
/// render thread.
///
/// Not slow: stopped. The ceiling is what keeps a project file from hanging the
/// application, and the flag is what keeps the clamp from being silent
/// (docs/14 §8).
#[test]
fn a_layer_carried_absurdly_far_forward_does_not_hang_the_scan() {
    let dt = 1.0 / 60.0;
    // Every frame the walk visits calls this, so it counts the walk.
    let calls = std::cell::Cell::new(0u64);
    let rate = |_: f64| {
        calls.set(calls.get() + 1);
        150.0
    };

    // Ten million seconds in, which is a retime keyframe away.
    let absurd = 600_000_000i64;
    let s = Schedule::scan(dt, absurd, 60, &rate);

    assert!(
        calls.get() <= points::MAX_SCAN_FRAMES as u64 + 1,
        "the walk visited {} frames, past the ceiling of {}",
        calls.get(),
        points::MAX_SCAN_FRAMES
    );
    assert!(
        !s.is_exact(),
        "a clamped walk must say so rather than pass for a complete one"
    );
    // And it is still a usable schedule: the window it records is the window
    // the frame can see, so the particles that are drawn are drawn properly.
    assert_eq!(s.counts().len(), 60);

    // An ordinary layer is untouched — no clamp, no flag, the same answer as
    // ever.
    let ordinary = Schedule::scan(dt, 599, 60, &|_| 150.0);
    assert!(ordinary.is_exact());
    assert_eq!(
        ordinary.total(),
        Schedule::scan(dt, 599, 60, &|_| 150.0).total()
    );
}

/// Trimming used to sum the whole `counts` vector to ask whether it was under
/// the ceiling yet, and then `Vec::remove(0)` — both linear, inside a loop that
/// runs once per dropped frame. At the hundred thousand frames the window
/// permits that is 10^10 operations for one trim, from a Life somebody typed.
///
/// This asserts the answer is unchanged; that it now arrives in one pass is what
/// makes it finish.
#[test]
fn trimming_to_the_newest_drops_the_right_frames_in_one_pass() {
    let dt = 1.0 / 60.0;
    // Ten births a frame, a thousand frames: ten thousand candidates.
    let mut s = Schedule::scan(dt, 999, 1000, &|_| 600.0);
    let before = s.candidates();
    assert!(before > 5_000, "the fixture needs enough to trim: {before}");

    let first_frame = s.first_frame();
    let first_birth = s.first_birth();
    s.trim_to_newest(500);

    assert!(
        s.candidates() <= 500 || s.counts().len() == 1,
        "trimmed to {} candidates over a ceiling of 500",
        s.candidates()
    );
    // The newest are what survive, so the window moved forward and the first
    // birth index moved with it by exactly what was dropped.
    assert!(s.first_frame() > first_frame);
    assert_eq!(
        s.first_birth() - first_birth,
        before - s.candidates(),
        "the births dropped and the index moved by different amounts"
    );
    // Trimming again to the same ceiling is a no-op rather than a walk.
    let settled = (s.first_frame(), s.first_birth(), s.candidates());
    s.trim_to_newest(500);
    assert_eq!((s.first_frame(), s.first_birth(), s.candidates()), settled);

    // A ceiling nothing reaches leaves it alone.
    let mut untouched = Schedule::scan(dt, 59, 60, &|_| 60.0);
    let was = (untouched.first_frame(), untouched.candidates());
    untouched.trim_to_newest(u64::MAX);
    assert_eq!((untouched.first_frame(), untouched.candidates()), was);
}

/// cache hit against the cold scan.
#[test]
fn the_birth_schedule_is_the_rate_curves_integral() {
    let dt = 1.0 / 60.0;
    // A constant 150 per second: after `n` frames, `floor(150 · n · Δt)` have
    // been born, give or take the carry the frame is holding.
    for frames in [1i64, 7, 60, 601] {
        let s = Schedule::scan(dt, frames - 1, frames, &|_| 150.0);
        let want = (150.0 * frames as f64 * dt).floor() as u64;
        assert!(
            s.total().abs_diff(want) <= 1,
            "{frames} frames at 150/s: {} births, expected about {want}",
            s.total()
        );
    }

    // A keyframed ramp, hand-computed: 0 → 120 per second over the first
    // second, sampled at each frame start. The carry after `n` frames is
    // `Σ rate(f)·Δt` over f < n, and the count is its floor.
    let ramp = |lt: f64| (lt.clamp(0.0, 1.0) * 120.0).floor();
    let mut carry = 0.0f64;
    let mut want = 0u64;
    for f in 0..60i64 {
        carry += ramp(f as f64 * dt) * dt;
        let n = carry.floor();
        carry -= n;
        want += n as u64;
    }
    let s = Schedule::scan(dt, 59, 60, &ramp);
    assert_eq!(s.total(), want, "the ramp's schedule is its own integral");

    // The cache: a hit is the cold scan, and a changed key scans again.
    let mut cache = points::ScheduleCache::default();
    let cold = Schedule::scan(dt, 59, 60, &|_| 150.0);
    let mut scans = 0;
    let hit = cache
        .get_or_scan(1, || {
            scans += 1;
            Schedule::scan(dt, 59, 60, &|_| 150.0)
        })
        .clone();
    assert_eq!(hit, cold, "a cold scan and a cached one must agree");
    let again = cache
        .get_or_scan(1, || {
            scans += 1;
            Schedule::default()
        })
        .clone();
    assert_eq!(again, cold, "the same key is served from the cache");
    assert_eq!(scans, 1, "one key, one scan");
    cache.get_or_scan(2, || {
        scans += 1;
        Schedule::default()
    });
    assert_eq!(scans, 2, "a changed key scans again");
}

/// **The closed forms** (§9 item 4): position and speed against the analytic
/// solutions at no drag, at `k = 0.5`, and either side of the series guard —
/// and wind with no drag as exactly motionless wind, which is the documented
/// behaviour rather than an accident of the algebra.
///
/// **All three axes**: the depth component is held to the same textbook
/// solution as the other two, with a wind of its own, which is what "the same
/// drag and wind algebra" has to mean if it is to mean anything.
#[test]
fn the_closed_forms_match_the_analytic_solutions() {
    let base = points::Forces {
        gravity: 0.0,
        wind: [0.0, 0.0, 0.0],
        drag: 0.0,
        turbulence: 0.0,
        turbulence_scale: 200.0,
        turbulence_speed: 0.0,
    };
    let p0 = [100.0f32, 50.0, -25.0];
    let v0 = [30.0f32, -80.0, 45.0];

    // k = 0: p = p0 + v0·t + ½g·t², v = v0 + g·t. Written out here so the
    // test is the textbook and not the implementation.
    let f = points::Forces {
        gravity: 400.0,
        ..base
    };
    for age in [0.0f32, 0.25, 1.0, 3.0] {
        let (pos, vel) = points::integrate(p0, v0, &f, age);
        let want = [
            p0[0] + v0[0] * age,
            p0[1] + v0[1] * age + 0.5 * 400.0 * age * age,
            // Gravity stays down: the depth axis is unaccelerated.
            p0[2] + v0[2] * age,
        ];
        assert!((pos[0] - want[0]).abs() < 1e-3, "x at {age}");
        assert!((pos[1] - want[1]).abs() < 1e-2, "y at {age}: {pos:?}");
        assert!((pos[2] - want[2]).abs() < 1e-3, "z at {age}: {pos:?}");
        assert!((vel[1] - (v0[1] + 400.0 * age)).abs() < 1e-2, "vy at {age}");
        assert!((vel[2] - v0[2]).abs() < 1e-3, "vz at {age}");
    }

    // k = 0.5, with wind and gravity: the published form, `g/k` and all,
    // against the rearrangement the implementation uses.
    let f = points::Forces {
        gravity: 400.0,
        wind: [120.0, 0.0, -60.0],
        drag: 0.5,
        ..base
    };
    for age in [0.1f32, 1.0, 4.0] {
        let k = 0.5f32;
        let (pos, vel) = points::integrate(p0, v0, &f, age);
        for i in 0..3 {
            let g = if i == 1 { 400.0f32 } else { 0.0 };
            let w = f.wind[i];
            let term = v0[i] - w - g / k;
            let want_p = p0[i] + (w + g / k) * age + term * (1.0 - (-k * age).exp()) / k;
            let want_v = w + g / k + term * (-k * age).exp();
            assert!(
                (pos[i] - want_p).abs() < 1e-2,
                "axis {i} position at {age}: {} vs {want_p}",
                pos[i]
            );
            assert!((vel[i] - want_v).abs() < 1e-2, "axis {i} speed at {age}");
        }
    }

    // Across the guard: `k·age = 0.1` is where the series takes over, and the
    // two branches have to *meet* there rather than step — which is the whole
    // reason the guard is not at particulate.md's 1e−4, where `1 − e^(−x)` has
    // already lost three of f32's seven digits (see `drag_terms`).
    let f = points::Forces {
        gravity: 400.0,
        wind: [120.0, 0.0, -60.0],
        drag: 0.1,
        ..base
    };
    let below = points::integrate(p0, v0, &f, 0.999).0;
    let above = points::integrate(p0, v0, &f, 1.001).0;
    let at = points::integrate(p0, v0, &f, 1.0).0;
    for i in 0..3 {
        assert!(
            (at[i] - (below[i] + above[i]) * 0.5).abs() < 1e-3,
            "the guard steps at axis {i}: {} against {} and {}",
            at[i],
            below[i],
            above[i]
        );
    }

    // Wind acts *through* drag: with no drag, wind does nothing at all.
    let windy = points::Forces {
        wind: [500.0, -500.0, 250.0],
        ..base
    };
    let (still, _) = points::integrate(p0, v0, &base, 2.0);
    let (blown, _) = points::integrate(p0, v0, &windy, 2.0);
    assert_eq!(still, blown, "wind with no drag moved a particle");
}

/// **The cap rule** (§9 item 7): over budget, the live set is exactly the
/// newest `cap` by birth index — and the degradation rung is the same rule
/// again at half the number. Old particles vanish early under overload:
/// visible, deterministic, and the same from any scrub direction.
#[test]
fn the_cap_keeps_the_newest_particles() {
    let over = particulate(&[
        ("emit_rate", fixed(600.0)),
        ("life", fixed(4.0)),
        ("life_jitter", fixed(0.0)),
        ("max_particles", fixed(100.0)),
    ]);
    let s = particulate_stream(&over, 3.0);
    assert_eq!(s.len(), 100, "the cap is the live count");

    // The same frame with room to spare: the capped set is the *tail* of it.
    let roomy = particulate(&[
        ("emit_rate", fixed(600.0)),
        ("life", fixed(4.0)),
        ("life_jitter", fixed(0.0)),
        ("max_particles", fixed(20000.0)),
    ]);
    let all = particulate_stream(&roomy, 3.0);
    assert!(all.len() > 100, "the fixture is not over budget");
    assert_eq!(
        s.id,
        all.id[all.len() - 100..],
        "the cap kept something other than the newest hundred"
    );

    // The degradation rung: the newest half, by the same rule.
    let half = all.len() / 2;
    let mut halved = all.clone();
    halved.keep_newest(half);
    assert_eq!(halved.len(), half);
    assert_eq!(halved.id, all.id[all.len() - half..]);
    assert_eq!(halved.position, all.position[all.len() - half..]);
}

/// **The mask-path emitter's no-op** (§9 item 9): nothing to walk means no
/// particles at all, and the effect passes its input through — degrade, never
/// fault (14-ENGINEERING-RULES §4).
#[test]
fn a_mask_path_emitter_with_no_path_emits_nothing() {
    let e = particulate(&[("shape", EffectValue::Choice(4))]);
    let p = particulate_params(&e);
    assert_eq!(p.emitter.shape, EmitterShape::MaskPath);
    let s = particulate_stream(&e, 2.0);
    assert!(s.is_empty(), "an empty polyline emitted {} points", s.len());

    let mut rgba = vec![0.25f32; 16 * 16 * 4];
    let before = rgba.clone();
    points::draw_discs(&mut rgba, 16, 16, &s, 1.0);
    assert_eq!(rgba, before, "the input did not pass through untouched");

    // A path to walk, and the same emitter emits again.
    let path = MaskPolyline {
        expansion: 0.0,
        feather: 0.0,
        points: vec![[0.0, 0.0], [100.0, 0.0]],
        arc: vec![0.0, 100.0],
        closed: false,
    };
    let dt = 1.0 / 60.0;
    let sched = Schedule::scan(dt, 120, 600, &|_| 150.0);
    // Nothing to carry them off the line, so where they are is where they were
    // born — which is what the assertion below is really about.
    let mut still = p.clone();
    still.emitter.speed = 0.0;
    still.forces.turbulence = 0.0;
    let walked = points::evaluate(&still, &sched, 2.0, &path);
    assert!(!walked.is_empty(), "a path with length emitted nothing");
    for at in &walked.position {
        assert!(
            (0.0..=100.0).contains(&at[0]),
            "a particle was born off the path at {at:?}"
        );
    }
}

// --------------------------------------------------- Scatter

/// A Scatter instance with its declared defaults, and `edits` applied.
fn scatter(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("scatter").expect("Scatter is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("scatter has no {id}"));
        p.value = value.clone();
    }
    e
}

/// The instance's reduced controls, resolved at full raster.
fn scatter_of(e: &EffectInstance) -> crate::fx::effects::scatter::Scatter {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    crate::fx::effects::scatter::Scatter::read(Params::new(&bag))
}

/// A picture whose left half is opaque and whose right half is transparent —
/// a hard edge, so nothing in the assertions below turns on a rounding.
fn half_alpha(w: u32, h: u32) -> Vec<f32> {
    let mut rgba = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            rgba[d + 3] = if x < w / 2 { 1.0 } else { 0.0 };
        }
    }
    rgba
}

/// **Points land inside the alpha and nowhere else** — the whole of what this
/// effect claims. A hard-edged field admits every candidate on the opaque side
/// and refuses every one on the other.
#[test]
fn scatter_keeps_the_points_that_land_on_alpha() {
    let (w, h) = (200u32, 100u32);
    let rgba = half_alpha(w, h);
    let s = scatter_of(&scatter(&[]));
    let all = s.candidates(w, h, 1.0, points::Projection::FLAT);
    let kept = s.stream(w, h, 1.0, &rgba, false, points::Projection::FLAT);
    assert!(!kept.is_empty(), "nothing survived a half-opaque picture");
    assert!(
        kept.len() < all.len(),
        "everything survived — nothing was refused"
    );
    for at in &kept.position {
        assert!(
            at[0] < w as f32 / 2.0,
            "a point stood on the transparent half at {at:?}"
        );
    }
    // Half the frame is opaque and the field there is 1, so every candidate on
    // that side stands: the count is the candidates that fell on the left.
    let left = all
        .position
        .iter()
        .filter(|at| at[0] < w as f32 / 2.0)
        .count();
    assert_eq!(kept.len(), left, "the opaque half refused somebody");
}

/// **The preview divisor never re-rolls the crowd**:
/// Density is a count per *composition* area, so a half-resolution raster
/// throws the same candidates at the same places in comp pixels. What may
/// differ is which of them a **soft** edge admits, which is why this fixture
/// has a hard one.
#[test]
fn scatter_throws_the_same_candidates_at_every_raster() {
    let s = scatter_of(&scatter(&[]));
    let full = s.candidates(400, 300, 1.0, points::Projection::FLAT);
    let half = s.candidates(200, 150, 0.5, points::Projection::FLAT);
    assert_eq!(full.len(), half.len(), "a different number of candidates");
    for (a, b) in full.position.iter().zip(half.position.iter()) {
        assert!(
            (a[0] * 0.5 - b[0]).abs() < 1e-3 && (a[1] * 0.5 - b[1]).abs() < 1e-3,
            "a candidate moved between rasters: {a:?} against {b:?}"
        );
    }
}

/// **The cap is a ceiling on the work** (a generator's shape of the rule):
/// Max points bounds the *candidates*, and what stands is a subset.
#[test]
fn scatter_throws_no_more_candidates_than_its_cap() {
    let s = scatter_of(&scatter(&[
        ("density", fixed(100.0)),
        ("max_points", fixed(500.0)),
    ]));
    let all = s.candidates(1920, 1080, 1.0, points::Projection::FLAT);
    assert_eq!(all.len(), 500);
    assert_eq!(all.id, (0..500).collect::<Vec<u64>>());
    // And under the cap the count is the density's own arithmetic.
    let plenty = scatter_of(&scatter(&[("density", fixed(10.0))]));
    assert_eq!(plenty.candidate_count(1000, 1000, 1.0), 1000);
}

// -------------------------------------------- Emit from image

/// An Emit from image instance with its declared defaults, and `edits` applied.
fn emit_from_image(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("emit_from_image").expect("Emit from image is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("emit_from_image has no {id}"));
        p.value = value.clone();
    }
    e
}

/// The instance's reduced controls, resolved at full raster.
fn emit_of(e: &EffectInstance) -> crate::fx::effects::emit_from_image::EmitFromImage {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    crate::fx::effects::emit_from_image::EmitFromImage::read(Params::new(&bag))
}

/// A picture whose left half is opaque white and whose right half is opaque
/// black — a hard *brightness* edge at full coverage everywhere, so nothing in
/// the assertions below turns on the alpha or on a rounding.
fn half_bright(w: u32, h: u32) -> Vec<f32> {
    let mut rgba = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let v = if x < w / 2 { 1.0 } else { 0.0 };
            rgba[d] = v;
            rgba[d + 1] = v;
            rgba[d + 2] = v;
            rgba[d + 3] = 1.0;
        }
    }
    rgba
}

/// **Points land where the picture is bright and nowhere else** — the whole of
/// what this effect claims. A hard-edged field admits every candidate on the
/// white side and refuses every one on the black.
#[test]
fn emit_from_image_keeps_the_points_that_land_on_light() {
    let (w, h) = (200u32, 100u32);
    let rgba = half_bright(w, h);
    let e = emit_of(&emit_from_image(&[]));
    let all = e.candidates(w, h, 1.0, points::Projection::FLAT);
    let kept = e.stream(w, h, 1.0, &rgba, points::Projection::FLAT);
    assert!(!kept.is_empty(), "nothing survived a half-white picture");
    assert!(
        kept.len() < all.len(),
        "everything survived — nothing was refused"
    );
    for at in &kept.position {
        assert!(
            at[0] < w as f32 / 2.0,
            "a point stood on the dark half at {at:?}"
        );
    }
    // The white half's field is 1 at any threshold below full, so every
    // candidate there stands: the count is the candidates that fell left.
    let left = all
        .position
        .iter()
        .filter(|at| at[0] < w as f32 / 2.0)
        .count();
    assert_eq!(kept.len(), left, "the bright half refused somebody");
}

/// **The cap is a ceiling on the work** (a generator's shape of the rule):
/// Max points bounds the *candidates*, and what stands is a subset.
#[test]
fn emit_from_image_throws_no_more_candidates_than_its_cap() {
    let e = emit_of(&emit_from_image(&[
        ("density", fixed(100.0)),
        ("max_points", fixed(500.0)),
    ]));
    let all = e.candidates(1920, 1080, 1.0, points::Projection::FLAT);
    assert_eq!(all.len(), 500);
    assert_eq!(all.id, (0..500).collect::<Vec<u64>>());
}

// ------------------------------------------------------ Grid

/// A Grid instance with its declared defaults, and `edits` applied.
fn grid(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("grid").expect("Grid is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("grid has no {id}"));
        p.value = value.clone();
    }
    e
}

/// The lattice one instance emits at layer time `t`, in px@comp.
fn grid_stream(e: &EffectInstance, t: f64) -> PointsStream {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        t,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    crate::fx::effects::grid::Grid::read(Params::new(&bag)).stream(points::Projection::FLAT)
}

/// **The lattice is a lattice**: as many points as cells, spaced by Spacing,
/// centred on Position, and `id` the walk's own index.
#[test]
fn a_grid_emits_one_point_per_cell_on_its_spacing() {
    let e = grid(&[
        ("columns", fixed(5.0)),
        ("rows", fixed(3.0)),
        ("planes", fixed(1.0)),
        ("spacing_x", fixed(100.0)),
        ("spacing_y", fixed(50.0)),
    ]);
    let s = grid_stream(&e, 0.0);
    assert_eq!(s.len(), 15, "five columns of three rows");
    assert_eq!(
        s.id,
        (0..15).collect::<Vec<u64>>(),
        "id is the walk's index"
    );
    // Row-major from the top left corner, centred on the comp's middle.
    for (i, at) in s.position.iter().enumerate() {
        let (c, r) = (i % 5, i / 5);
        let want = [
            960.0 + (c as f32 - 2.0) * 100.0,
            540.0 + (r as f32 - 1.0) * 50.0,
            0.0,
        ];
        assert_eq!(*at, want, "cell {i}");
    }
}

/// **The cap rule, a generator's shape of it**: a lattice past Max
/// points keeps the **first** cap by index. A lattice has no birth order, so
/// the rule is a prefix of the one fixed ordering rather than the newest —
/// deterministic, and the same from any scrub direction.
#[test]
fn a_grid_over_its_cap_keeps_the_first_cells_by_index() {
    let e = grid(&[
        ("columns", fixed(40.0)),
        ("rows", fixed(40.0)),
        ("max_points", fixed(100.0)),
    ]);
    let capped = grid_stream(&e, 0.0);
    assert_eq!(capped.len(), 100);
    assert_eq!(capped.id, (0..100).collect::<Vec<u64>>());

    let whole = grid_stream(
        &grid(&[("columns", fixed(40.0)), ("rows", fixed(40.0))]),
        0.0,
    );
    assert_eq!(whole.len(), 1600);
    assert_eq!(capped.position, whole.position[..100], "a different prefix");
}

// --------------------------------------- Clone to points

/// A Clone to points instance with its declared defaults, and `edits` applied.
fn clone_to_points(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("clone_to_points").expect("Clone to points is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("clone_to_points has no {id}"));
        p.value = value.clone();
    }
    e
}

/// The stamps one instance lays over `stream`, through the one expression both
/// render paths read.
fn clone_stamps(e: &EffectInstance, stream: &PointsStream) -> PointsStream {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    crate::fx::effects::clone_to_points::CloneToPoints::read(Params::new(&bag)).stamps(stream)
}

/// A small hand-made stream: two points, a known distance apart, with distinct
/// colours so a painter's-order test can tell which one landed last.
fn two_points() -> PointsStream {
    PointsStream {
        position: vec![[20.0, 20.0, 0.0], [22.0, 20.0, 0.0]],
        speed: vec![[0.0; 3]; 2],
        age: vec![0.0; 2],
        life: vec![1.0; 2],
        size: vec![10.0, 10.0],
        rotation: vec![0.0, 0.0],
        colour: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
        id: vec![0, 1],
        projection: points::Projection::FLAT,
    }
}

/// A flat opaque sprite of `n × n` — a picture with nothing in it but coverage,
/// so what a stamp test measures is placement and tint rather than filtering.
fn flat_sprite(n: u32) -> Vec<f32> {
    vec![1.0; (n * n * 4) as usize]
}

/// **One stamp per point**, with the point's own size and rotation and the
/// effect's two dials on top.
#[test]
fn clone_to_points_stamps_one_copy_per_point() {
    let stream = grid_stream(&grid(&[("columns", fixed(4.0)), ("rows", fixed(3.0))]), 0.0);
    let stamps = clone_stamps(
        &clone_to_points(&[("scale", fixed(50.0)), ("rotation", fixed(90.0))]),
        &stream,
    );
    assert_eq!(stamps.len(), stream.len(), "a stamp per point");
    assert_eq!(stamps.position, stream.position, "a stamp moved its point");
    assert_eq!(stamps.id, stream.id, "and lost its identity");
    for (from, to) in stream.size.iter().zip(stamps.size.iter()) {
        assert!((from * 0.5 - to).abs() < 1e-5, "Scale did not halve {from}");
    }
    for (from, to) in stream.rotation.iter().zip(stamps.rotation.iter()) {
        assert!(
            (from + std::f32::consts::FRAC_PI_2 - to).abs() < 1e-5,
            "Rotation did not turn the stamp"
        );
    }
}

/// **Painter's order is `id` order**, which is what makes the picture
/// the same on every machine: the stream arrives ordered by birth index
/// ascending, and a later stamp lands on top of an earlier one.
#[test]
fn clone_to_points_lays_its_stamps_in_id_order() {
    let (w, h) = (48u32, 48u32);
    let sprite = flat_sprite(4);
    let style = points::DrawStyle {
        mode: points::RenderMode::Sprite,
        feather: 0.0,
        streak_seconds: 0.0,
        mix: 1.0,
    };
    let draw = |s: &PointsStream| {
        let mut rgba = vec![0.0f32; (w * h * 4) as usize];
        points::draw_stream(
            &mut rgba,
            w,
            h,
            s,
            &[],
            &style,
            Some(points::Sprite {
                rgba: &sprite,
                w: 4,
                h: 4,
            }),
        );
        // The pixel the two stamps share.
        let d = ((20 * w + 21) * 4) as usize;
        [rgba[d], rgba[d + 1], rgba[d + 2]]
    };
    let ascending = two_points();
    let over = draw(&ascending);
    assert!(
        over[1] > over[0],
        "the higher id did not land on top: {over:?}"
    );

    // The same two points handed over in the other order paint the other way
    // round — which is what makes the ordering above a *fact being kept* rather
    // than an accident of where the stamps happen to sit.
    let mut descending = ascending.clone();
    descending.position.reverse();
    descending.colour.reverse();
    descending.id.reverse();
    let under = draw(&descending);
    assert!(under[0] > under[1], "the order changed nothing: {under:?}");
}

/// **The px@comp rule for a stream read off a wire**: a stream
/// is data in composition pixels, and rearranging it into the raster a frame is
/// drawn at is one multiplication per length. At full resolution nothing moves
/// at all, which is what makes preview and export bit-identical.
#[test]
fn a_stream_rescales_into_the_raster_it_is_drawn_at() {
    let full = grid_stream(&grid(&[("spacing_x", fixed(100.0))]), 0.0);
    assert_eq!(full.rescaled(1.0), full, "full resolution moved something");

    let half = full.rescaled(0.5);
    assert_eq!(half.len(), full.len());
    assert_eq!(half.id, full.id, "a point lost its identity in the rescale");
    for (a, b) in full.position.iter().zip(half.position.iter()) {
        for k in 0..3 {
            assert!((a[k] * 0.5 - b[k]).abs() < 1e-4, "{a:?} against {b:?}");
        }
    }
    for (a, b) in full.size.iter().zip(half.size.iter()) {
        assert!((a * 0.5 - b).abs() < 1e-4, "the disc did not follow");
    }
    assert_eq!(
        half.projection,
        points::Projection::FLAT,
        "a flat camera stayed flat"
    );
}

// -------------------------------------------------- Trail

/// A Trail instance with its declared defaults, and `edits` applied.
fn trail(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("trail").expect("Trail is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("trail has no {id}"));
        p.value = value.clone();
    }
    e
}

/// The tail one instance draws over `samples`, through the one expression both
/// render paths read.
fn trail_tail(e: &EffectInstance, samples: &[PointsStream]) -> (PointsStream, Vec<[f32; 3]>) {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    crate::fx::effects::trail::Trail::read(Params::new(&bag)).tail(samples)
}

/// A moving emitter's stream at `t`, and the same producer at `t − k·step` —
/// the carriage a wired Trail is handed, newest first.
fn moving_samples(n: usize, step: f64) -> Vec<PointsStream> {
    let e = particulate(&[
        ("emit_rate", fixed(30.0)),
        ("life", fixed(4.0)),
        ("life_jitter", fixed(0.0)),
        ("initial_speed", fixed(200.0)),
        ("speed_jitter", fixed(0.0)),
        ("spread", fixed(0.0)),
        ("turbulence_amount", fixed(0.0)),
        ("drag", fixed(0.0)),
        ("gravity", fixed(0.0)),
    ]);
    (0..n)
        .map(|k| particulate_stream(&e, 2.0 - k as f64 * step))
        .collect()
}

/// Where every dab a tail should carry comes from — `(sample index, index into
/// that sample)`, **oldest sample first and ascending `id` inside it**, which is
/// the whole of the painter's order.
///
/// Written out rather than assumed, because the blocks are **not** the same
/// length: a point born between two samples is not in the earlier one, so a
/// test that chunked the answer by the head count would be asserting something
/// the design never promised.
fn expected_dabs(
    samples: &[PointsStream],
    heads: &PointsStream,
    wanted: usize,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for k in (0..wanted).rev() {
        let mut cursor = 0;
        for id in &heads.id {
            if let Some(j) = PointsStream::seek_id(&samples[k], *id, &mut cursor) {
                out.push((k, j));
            }
        }
    }
    out
}

/// **Oldest sample first, `id` inside it** — the painter's order that makes the
/// picture the same on every machine, and puts the near end of a tail on top of
/// the far end.
#[test]
fn a_trail_lays_its_dabs_oldest_sample_first() {
    let samples = moving_samples(3, 0.1);
    let (drawn, _) = trail_tail(&trail(&[("back_samples", fixed(3.0))]), &samples);
    let want = expected_dabs(&samples, &samples[0], 3);
    assert_eq!(drawn.len(), want.len(), "the wrong number of dabs");
    for (i, (k, j)) in want.iter().enumerate() {
        assert_eq!(drawn.id[i], samples[*k].id[*j], "dab {i} is out of order");
        assert_eq!(
            drawn.position[i], samples[*k].position[*j],
            "dab {i} is somewhere the producer never put the point"
        );
    }
    // And the order is not the trivial one: the sample the dab came from only
    // ever runs from the far end towards the near one.
    assert!(
        want.windows(2).all(|w| w[0].0 >= w[1].0),
        "the samples are not laid down oldest first"
    );
}

// ----------------------------------------- Connect points

/// A Connect points instance with its declared defaults, and `edits` applied.
fn connect_points(edits: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = instantiate("connect_points").expect("Connect points is declared");
    for (id, value) in edits {
        let p = e
            .params
            .iter_mut()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("connect_points has no {id}"));
        p.value = value.clone();
    }
    e
}

/// The web one instance draws over `stream`, through the one expression both
/// render paths read.
fn connect_links(e: &EffectInstance, stream: &PointsStream) -> (PointsStream, Vec<[f32; 3]>) {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    crate::fx::effects::connect_points::ConnectPoints::read(Params::new(&bag)).links(stream)
}

/// Every pair of `stream` within `reach` of each other, **by a full
/// comparison** — the `n²` answer the bucketed walk has to reproduce exactly.
fn pairs_within(stream: &PointsStream, reach: f32) -> usize {
    let mut n = 0;
    for i in 0..stream.len() {
        for j in i + 1..stream.len() {
            let (a, b) = (stream.projected(i), stream.projected(j));
            if (b[0] - a[0]).hypot(b[1] - a[1]) <= reach {
                n += 1;
            }
        }
    }
    n
}

/// **The buckets find what a full comparison finds** — the property the
/// whole optimisation rests on, and the one a regression would quietly break:
/// cutting the plane into squares must change what the walk *costs*, never what
/// it answers.
///
/// A jittered lattice rather than a regular one, so the pairs fall at every
/// distance and across every cell boundary instead of at a handful of exact
/// spacings.
#[test]
fn connect_points_pairs_exactly_as_a_full_comparison_would() {
    let stream = grid_stream(
        &grid(&[
            ("columns", fixed(7.0)),
            ("rows", fixed(5.0)),
            ("planes", fixed(1.0)),
            ("spacing_x", fixed(40.0)),
            ("spacing_y", fixed(40.0)),
            ("jitter_x", fixed(30.0)),
            ("jitter_y", fixed(30.0)),
        ]),
        0.0,
    );
    // Max connections at its hard ceiling, so the *pairing* is what is being
    // measured rather than the allowance running out.
    for reach in [1.0, 25.0, 47.0, 90.0, 400.0] {
        let (web, _) = connect_links(
            &connect_points(&[("max_distance", fixed(reach)), ("max_links", fixed(64.0))]),
            &stream,
        );
        assert_eq!(
            web.len(),
            pairs_within(&stream, reach as f32),
            "the buckets and a full comparison disagree at reach {reach}"
        );
    }
}

// ------------------------------------------- border emission

/// A still emitter of the given shape: nothing carries a particle off the
/// outline, so where it is drawn is where it was born.
fn outline_stream(shape: u32) -> PointsStream {
    let e = particulate(&[
        ("shape", EffectValue::Choice(shape)),
        ("initial_speed", fixed(0.0)),
        ("turbulence_amount", fixed(0.0)),
        ("drag", fixed(0.0)),
        ("gravity", fixed(0.0)),
        ("width", fixed(400.0)),
        ("height", fixed(200.0)),
        ("emitter_angle", fixed(0.0)),
        ("life", fixed(10.0)),
        ("life_jitter", fixed(0.0)),
    ]);
    particulate_stream(&e, 2.0)
}

/// **The outline is an outline**: every particle lands *on* the ring, not
/// inside it. The ellipse's own equation is the assertion — `(x/a)² + (y/b)² =
/// 1` — and the chord flattening is the only slack allowed for
/// (`OUTLINE_SEGMENTS` cuts at most three parts in ten thousand of the radius).
#[test]
fn an_ellipse_outline_emitter_puts_every_particle_on_the_ring() {
    let s = outline_stream(5);
    assert!(!s.is_empty(), "the outline emitted nothing");
    let (cx, cy) = (960.0f32, 540.0f32);
    let (a, b) = (200.0f32, 100.0f32);
    for at in &s.position {
        let (u, v) = ((at[0] - cx) / a, (at[1] - cy) / b);
        let r = (u * u + v * v).sqrt();
        assert!(
            (r - 1.0).abs() < 1e-3,
            "a particle sat at radius {r} rather than on the ring ({at:?})"
        );
    }
}

/// **The filled shapes are untouched**: the two new codes are appended,
/// so every shape a saved document can name emits exactly what it always did.
#[test]
fn adding_the_outline_shapes_moved_no_existing_code() {
    for (code, shape) in [
        (0, EmitterShape::Point),
        (1, EmitterShape::Line),
        (2, EmitterShape::Ellipse),
        (3, EmitterShape::Rectangle),
        (4, EmitterShape::MaskPath),
        (5, EmitterShape::EllipseOutline),
        (6, EmitterShape::RectangleOutline),
    ] {
        assert_eq!(EmitterShape::from_code(code), shape, "code {code}");
    }
    assert_eq!(EmitterShape::OPTIONS.len(), 7);
    // A filled ellipse still fills: the interior draw is the one that was
    // there, and the outline is a different code entirely.
    let inside = outline_stream(2);
    let on_ring = inside
        .position
        .iter()
        .filter(|at| {
            let (u, v) = ((at[0] - 960.0) / 200.0, (at[1] - 540.0) / 100.0);
            ((u * u + v * v).sqrt() - 1.0).abs() < 1e-3
        })
        .count();
    assert!(
        on_ring * 20 < inside.len(),
        "a filled ellipse put {on_ring} of {} particles exactly on its rim",
        inside.len()
    );
}

// ------------------------------------------------ the third axis

/// A camera one `zoom` back from a layer whose plane is at `z = 0`, as the
/// projection restricted to that plane comes out: a particle at depth `z`
/// scales by `zoom / (zoom + z)` about the frame's centre.
///
/// Written out rather than built through the renderer because this file is
/// `lumit-core`, which has no compositor in it — and because a hand-written
/// perspective is what makes the assertions below checkable by eye. The
/// renderer's own construction is held to the compositor's matrices in
/// `lumit-render`.
fn head_on_camera(centre: [f32; 2], zoom: f32) -> points::Projection {
    // x' = cx + (x − cx)·zoom/(zoom + z), which as a 3×4 is:
    //   X = x + cx·z/zoom      Y = y + cy·z/zoom      W = 1 + z/zoom
    // — the same column `lumit_gpu::camera_matrix` writes its perspective into.
    points::Projection {
        m: [
            [1.0, 0.0, centre[0] / zoom, 0.0],
            [0.0, 1.0, centre[1] / zoom, 0.0],
            [0.0, 0.0, 1.0 / zoom, 1.0],
        ],
    }
}

/// **A camera foreshortens, and the plane does not move**: a particle
/// at `z = 0` lands exactly where its x and y say, one further off is drawn
/// smaller and pulled towards the centre, and one nearer is drawn larger.
#[test]
fn the_camera_puts_depth_where_perspective_puts_it() {
    let proj = head_on_camera([100.0, 100.0], 400.0);
    let (on_plane, scale) = proj.apply([160.0, 100.0, 0.0]);
    assert!((on_plane[0] - 160.0).abs() < 1e-3, "{on_plane:?}");
    assert!((on_plane[1] - 100.0).abs() < 1e-3, "{on_plane:?}");
    assert!(
        (scale - 1.0).abs() < 1e-6,
        "the plane foreshortened: {scale}"
    );

    // 400 further off: half the size, and half as far from the centre.
    let (far, far_scale) = proj.apply([160.0, 100.0, 400.0]);
    assert!((far_scale - 0.5).abs() < 1e-5, "{far_scale}");
    assert!((far[0] - 130.0).abs() < 1e-3, "{far:?}");

    // 200 nearer: twice the size, twice as far out.
    let (near, near_scale) = proj.apply([160.0, 100.0, -200.0]);
    assert!((near_scale - 2.0).abs() < 1e-5, "{near_scale}");
    assert!((near[0] - 220.0).abs() < 1e-3, "{near:?}");

    // At the camera's own plane: nought, which draws nothing rather than
    // flinging the particle across the frame (docs/14 §4).
    assert_eq!(proj.apply([160.0, 100.0, -400.0]).1, 0.0);
    assert_eq!(proj.apply([160.0, 100.0, -1000.0]).1, 0.0);
}

/// **The forward-migration gate**: a project saved with the 2D defaults draws
/// the picture it always drew, bit for bit.
///
/// The five new rows all default to nought, so the stream's own x and y — and
/// therefore every pixel — are the arithmetic they were before the axis
/// existed. What is asserted is the strongest available form of that: the
/// evaluation with the depth rows explicitly set to their defaults is
/// **identical** to one with them absent from the instance altogether, which is
/// exactly what an old file loads as.
#[test]
fn two_dimensional_defaults_draw_the_picture_they_always_drew() {
    // An instance with no depth rows stored at all — an old file, since a
    // parameter a document does not carry reads its schema default.
    let old = particulate(&[
        ("position_x", fixed(32.0)),
        ("position_y", fixed(32.0)),
        ("width", fixed(20.0)),
        ("height", fixed(20.0)),
    ]);
    let mut aged = old.clone();
    aged.params.retain(|p| {
        !matches!(
            p.id.as_str(),
            "position_z" | "depth" | "direction_z" | "spread_z" | "wind_z"
        )
    });
    assert_eq!(
        aged.params.len() + 5,
        old.params.len(),
        "the five depth rows are not the ones being removed"
    );

    let fresh = particulate_stream(&old, 2.0);
    let loaded = particulate_stream(&aged, 2.0);
    assert!(!fresh.is_empty(), "the fixture drew no particles");
    assert_eq!(fresh, loaded, "an old file's stream is not the new default");

    // Flat all the way through: no camera, so nothing about z can be seen.
    //
    // The *stream's* z is not necessarily nought — the default look has
    // Turbulence at 40, and turbulence gained a third lattice like every other
    // jitter with an x and a y. What the guarantee is about is the
    // **picture**: with no camera the projection drops the depth, so what is
    // drawn is exactly the pair it always was.
    assert!(fresh.projection.is_flat());
    for (i, p) in fresh.position.iter().enumerate() {
        assert_eq!(fresh.projected(i), [p[0], p[1]], "particle {i} was moved");
        assert_eq!(fresh.depth_scale(i), 1.0, "particle {i} foreshortened");
    }
    // With no turbulence there is nothing at all to leave the plane with.
    let still = particulate_stream(&particulate(&[("turbulence_amount", fixed(0.0))]), 2.0);
    assert!(!still.is_empty());
    for (i, p) in still.position.iter().enumerate() {
        assert_eq!(p[2], 0.0, "particle {i} left the plane on 2D defaults");
        assert_eq!(still.speed[i][2], 0.0, "particle {i} has a depth speed");
    }

    let mut a = vec![0.0f32; 64 * 64 * 4];
    let mut b = vec![0.0f32; 64 * 64 * 4];
    points::draw_discs(&mut a, 64, 64, &fresh, 1.0);
    points::draw_discs(&mut b, 64, 64, &loaded, 1.0);
    assert!(a.iter().any(|v| *v > 0.0), "nothing was drawn");
    assert_eq!(a, b, "an old file's picture moved");
}

/// **Frame-key sensitivity** (§9 item 11): the seed changes the key, an edit to
/// a control changes it, and nothing else does. No new terms — the standard
/// formula, which is the whole claim (particulate.md §5).
#[test]
fn particulates_frame_key_follows_its_seed_and_its_controls() {
    let key = |e: &EffectInstance, lt: f64| {
        let stack = super::resolve_stack(
            std::slice::from_ref(e),
            lt,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        let mut bytes: Vec<u8> = Vec::new();
        stack.feed_hash(&mut |b| bytes.extend_from_slice(b));
        bytes
    };
    let e = particulate(&[]);
    let mut reseeded = e.clone();
    for p in &mut reseeded.params {
        if p.id == "seed" {
            p.value = EffectValue::Seed(1234);
        }
    }
    assert_ne!(key(&e, 1.0), key(&reseeded, 1.0), "the seed is in the key");
    assert_eq!(key(&e, 1.0), key(&e, 1.0), "the key is not stable");
    // Scrubbing changes the picture, and what carries that into the key is the
    // `seeded` trait folding the layer's local time in — outside this hash, by
    // the standard rule. What the parameters must do is stay put while nothing
    // about them has changed, so that the fold is the only reason two frames
    // differ.
    assert_eq!(
        key(&e, 1.0),
        key(&e, 2.0),
        "no parameter animates by default"
    );
    assert!(
        BUILTIN_DEFS
            .get("particulate")
            .expect("declared")
            .schema()
            .traits
            .seeded,
        "Particulate must declare itself seeded, or its frames would share a key"
    );
    let mut nudged = e.clone();
    for p in &mut nudged.params {
        if p.id == "emit_rate" {
            p.value = fixed(200.0);
        }
    }
    assert_ne!(key(&e, 1.0), key(&nudged, 1.0), "the rate is in the key");
}

/// **Output width and height above 100 % grow the working raster** (docs/08
/// §3.39): the copies land past the frame's edges, and the effects after
/// Tile in the stack run on the wider picture so those copies are picture to
/// them rather than transparency.
///
/// At or below 100 % nothing grows — the window only clips, which needs no more
/// room than the frame already has — and Mix 0 grows nothing either, because an
/// identity that reallocated the raster would not be one.
#[test]
fn tile_grows_the_raster_only_above_a_hundred_per_cent() {
    let (w, h) = (32u32, 24u32);
    let of = |ow: f32, oh: f32, mix: f32| {
        let mut t = effects::tile::Tile::read(crate::fx::Params::EMPTY);
        t.tile_centre_x = 16.0;
        t.tile_centre_y = 12.0;
        t.tile_width = w as f32 * 0.5;
        t.tile_height = h as f32 * 0.5;
        // The window is px@comp, and the callers below say what
        // they mean as a share of this raster.
        t.output_width = w as f32 * ow;
        t.output_height = h as f32 * oh;
        t.mix = mix;
        t.packed(w as f32, h as f32)
    };
    assert_eq!(cpu::tile_raster(w, h, &of(1.0, 1.0, 100.0)), (w, h));
    assert_eq!(cpu::tile_raster(w, h, &of(0.6, 0.6, 100.0)), (w, h));
    assert_eq!(cpu::tile_raster(w, h, &of(2.0, 1.5, 100.0)), (64, 36));
    assert_eq!(
        cpu::tile_raster(w, h, &of(2.0, 1.5, 0.0)),
        (w, h),
        "Mix 0 is the identity, and an identity does not reallocate"
    );
    // The ceiling holds a slider drag to a raster every backend can allocate.
    assert_eq!(
        cpu::tile_raster(3840, 2160, &of(5.0, 5.0, 100.0)),
        (cpu::TILE_MAX_RASTER, cpu::TILE_MAX_RASTER),
        "the growth stops at the guaranteed maximum texture side"
    );

    // The margin holds picture, and the frame's own window is untouched: the
    // growth adds, it never moves what was already there.
    let img: Vec<f32> = (0..(w * h * 4))
        .map(|i| {
            if i % 4 == 3 {
                1.0
            } else {
                (i % 17) as f32 / 17.0
            }
        })
        .collect();
    let p = of(2.0, 1.5, 100.0);
    let (ow, oh) = cpu::tile_raster(w, h, &p);
    let mut grown = vec![0.0f32; (ow * oh * 4) as usize];
    cpu::tile_into(&img, w, h, &mut grown, ow, oh, &p);
    let mut flat = img.clone();
    cpu::tile(&mut flat, w, h, &of(1.0, 1.0, 100.0));
    let (ox, oy) = ((ow - w) / 2, (oh - h) / 2);
    for y in 0..h {
        for x in 0..w {
            let a = (((y + oy) * ow + x + ox) * 4) as usize;
            let b = ((y * w + x) * 4) as usize;
            assert_eq!(
                grown[a..a + 4],
                flat[b..b + 4],
                "the window moved at ({x}, {y})"
            );
        }
    }
    let top_row_alpha: f32 = (0..ow).map(|x| grown[(x * 4 + 3) as usize]).sum();
    assert!(
        top_row_alpha > 0.5 * ow as f32,
        "the margin an effect after Tile sees must be picture, not transparency"
    );
}

/// **The output window is centred on the tile centre** (docs/08 §3.39):
/// Output width and height above the tile's own spread half the extra to each
/// side of the stamped rectangle, left *and* right, up *and* down — AE's Motion
/// Tile.
///
/// Centred on the frame instead, a tile cut from anywhere but the middle grew
/// only towards the further edge: with the tile up and left of the middle, the
/// copies appeared to the right and below it and nowhere else, which is the bug
/// this test holds shut. The frame here is 32 × 24 with the tile cut from
/// (8, 6) — one quarter in — and the window two tiles wide and tall, so a
/// symmetric spread must put picture on both sides of the tile centre.
#[test]
fn tile_spreads_its_output_window_evenly_about_the_tile_centre() {
    let (w, h) = (32u32, 24u32);
    let mut t = effects::tile::Tile::read(crate::fx::Params::EMPTY);
    t.tile_centre_x = 8.0;
    t.tile_centre_y = 6.0;
    t.tile_width = w as f32 * 0.25;
    t.tile_height = h as f32 * 0.25;
    t.output_width = w as f32 * 0.5;
    t.output_height = h as f32 * 0.5;
    let p = t.packed(w as f32, h as f32);

    // An opaque frame, so "there is picture here" reads straight off alpha.
    let img: Vec<f32> = (0..(w * h * 4))
        .map(|i| if i % 4 == 3 { 1.0 } else { 0.5 })
        .collect();
    let (ow, oh) = cpu::tile_raster(w, h, &p);
    assert_eq!(
        (ow, oh),
        (w, h),
        "a window this side of the frame's own edges asks for no more raster"
    );
    let mut out = vec![0.0f32; (ow * oh * 4) as usize];
    cpu::tile_into(&img, w, h, &mut out, ow, oh, &p);
    let alpha = |x: u32, y: u32| out[((y * ow + x) * 4 + 3) as usize];

    // The window is 16 x 12 about (8, 6): x from 0 to 16, y from 0 to 12. Both
    // sides of the tile centre hold picture, and past the window there is none.
    for (x, y) in [(2u32, 6u32), (14, 6), (8, 1), (8, 10)] {
        assert!(
            alpha(x, y) > 0.5,
            "the window must reach ({x}, {y}), on its own side of the tile centre"
        );
    }
    for (x, y) in [(20u32, 6u32), (30, 6), (8, 16), (8, 22)] {
        assert!(
            alpha(x, y) < 0.5,
            "past the window at ({x}, {y}) there must be nothing"
        );
    }

    // And the same rule sizes the raster: a window that reaches past the frame
    // from an off-centre tile grows it far enough to hold both ends.
    let mut wide = t;
    wide.output_width = w as f32;
    let (gw, _) = cpu::tile_raster(w, h, &wide.packed(w as f32, h as f32));
    assert_eq!(
        gw,
        w + 16,
        "the raster must reach the further edge of a window centred off the frame"
    );
}

/// The corpus the spatial tests key against: a `w × h` frame that is the
/// screen colour everywhere except a foreground block, in premultiplied RGBA.
#[cfg(test)]
fn keyer_plate(w: u32, h: u32, block: (u32, u32, u32, u32)) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let (bx, by, bw, bh) = block;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let fg = x >= bx && x < bx + bw && y >= by && y < by + bh;
            // The foreground is a plain magenta, well away from the screen; the
            // screen is the effect's own default green.
            let c = if fg { [0.6, 0.1, 0.5] } else { [0.0, 0.6, 0.0] };
            img[i..i + 3].copy_from_slice(&c);
            img[i + 3] = 1.0;
        }
    }
    img
}

/// A base keyer at its defaults, on the Screen matte view so the tests read the
/// matte itself out of the red channel rather than inferring it from a colour.
#[cfg(test)]
fn matte_view_params() -> MatteKeyParams {
    MatteKeyParams {
        view: 1,
        key: [0.0, 0.6, 0.0, 1.0],
        gain: 1.0,
        balance: 0.5,
        despill_bias: [0.5, 0.5, 0.5, 1.0],
        alpha_bias: [0.5, 0.5, 0.5, 1.0],
        spill: 1.0,
        clip_black: 0.0,
        clip_white: 1.0,
        clip_rollback: 0.0,
        pre_blur: 0.0,
        shrink_grow: 0.0,
        softness: 0.0,
        despot_black: 0.0,
        despot_white: 0.0,
        replace_method: 2,
        replace_colour: [0.5, 0.5, 0.5, 1.0],
        mix: 1.0,
    }
}

/// **The spatial controls change nothing until one is asked for**.
///
/// The promise the whole landing rests on: an existing project keys the bytes it
/// always keyed. `matte_key_spatial` hands straight over to the pointwise keyer
/// when nothing spatial is set and neither garbage mask is bound, so this is an
/// equality on the pixels rather than a tolerance.
#[test]
fn the_keyers_defaults_are_the_pointwise_keyer_byte_for_byte() {
    let (w, h) = (16u32, 12u32);
    let img = keyer_plate(w, h, (4, 3, 8, 6));
    let p = MatteKeyParams {
        view: 0,
        ..matte_view_params()
    };
    let blank = cpu::MaskFillParams::blank();
    let mut pointwise = img.clone();
    cpu::matte_key(&mut pointwise, &p);
    let mut staged = img.clone();
    cpu::matte_key_spatial(&mut staged, w, h, &p, &blank, &blank);
    assert_eq!(pointwise, staged, "the defaults took a different path");
}

/// **Shrink and grow march the matte's edge, in opposite directions**.
///
/// Counted rather than sampled: how much of the frame the matte keeps is the
/// one number a morphological pass is supposed to move, and it must move up for
/// a grow and down for a shrink.
#[test]
fn the_screen_shrink_and_grow_march_the_mattes_edge() {
    let (w, h) = (24u32, 24u32);
    let img = keyer_plate(w, h, (8, 8, 8, 8));
    let blank = cpu::MaskFillParams::blank();
    let base = matte_view_params();
    let kept = |amount: f32| {
        let mut out = img.clone();
        cpu::matte_key_spatial(
            &mut out,
            w,
            h,
            &MatteKeyParams {
                shrink_grow: amount,
                ..base
            },
            &blank,
            &blank,
        );
        out.chunks_exact(4).filter(|c| c[0] > 0.5).count()
    };
    let (shrunk, plain, grown) = (kept(-2.0), kept(0.0), kept(2.0));
    assert_eq!(plain, 64, "the block itself is what the key keeps");
    assert!(
        shrunk < plain && plain < grown,
        "shrink {shrunk}, plain {plain}, grow {grown}"
    );
    // A morphological pass is not a blur: the edge that moved is still hard.
    let mut out = img.clone();
    cpu::matte_key_spatial(
        &mut out,
        w,
        h,
        &MatteKeyParams {
            shrink_grow: 2.0,
            ..base
        },
        &blank,
        &blank,
    );
    assert!(
        out.chunks_exact(4).all(|c| c[0] <= 0.001 || c[0] >= 0.999),
        "growing softened the edge, which is Softness' job"
    );
}

/// **The garbage masks force opaque and force transparent**, and an
/// unset one is the no-op.
#[test]
fn the_garbage_masks_hold_the_matte_open_and_shut() {
    let (w, h) = (24u32, 24u32);
    // A frame that is nothing but screen: the key alone keeps none of it.
    let img = keyer_plate(w, h, (0, 0, 0, 0));
    let base = matte_view_params();
    let blank = cpu::MaskFillParams::blank();
    let masks = vec![crate::mask::Mask::rectangle(6.0, 6.0, 8.0, 8.0)];
    let poly = crate::mask::mask_path_at(&masks, None, true, 0.0);
    assert!(
        !poly.is_empty() && poly.closed,
        "a rectangle is a closed path"
    );
    let fill = cpu::mask_fill_params(&poly, 1.0);
    assert!(fill.count >= 4, "the outline flattened to nothing");

    let mut nothing = img.clone();
    cpu::matte_key_spatial(&mut nothing, w, h, &base, &blank, &blank);
    assert!(
        nothing.chunks_exact(4).all(|c| c[0] < 0.01),
        "an all-screen frame keys to nothing"
    );

    // Inside: the rectangle is opaque, and only the rectangle.
    let mut held = img.clone();
    cpu::matte_key_spatial(&mut held, w, h, &base, &fill, &blank);
    let at = |x: u32, y: u32, buf: &[f32]| buf[((y * w + x) * 4) as usize];
    assert!(at(10, 10, &held) > 0.99, "the hold-out is not opaque");
    assert!(at(2, 2, &held) < 0.01, "the hold-out leaked outside itself");

    // Outside: on a frame the key keeps whole, the rectangle is cut away.
    let fg = keyer_plate(w, h, (0, 0, w, h));
    let mut cut = fg.clone();
    cpu::matte_key_spatial(&mut cut, w, h, &base, &blank, &fill);
    assert!(at(10, 10, &cut) < 0.01, "the cut-out is not transparent");
    assert!(at(2, 2, &cut) > 0.99, "the cut-out ate the whole frame");

    // An open path holds nothing out: the row's documented no-op.
    let mut open = poly.clone();
    open.closed = false;
    assert_eq!(cpu::mask_fill_params(&open, 1.0).count, 0);
}

// ------------------------------------------------- the run-time catalogue --

/// A definition that arrived at run time, as a plugin's does. Declared
/// here rather than driven by a real OFX bundle because what is under test is
/// the *seam*: `lumit-core` cannot depend on the plugin host (docs/05), so what
/// it can be shown is that an effect it never compiled in becomes an effect like
/// any other. `lumit-ofx` proves the other half — that a real plugin makes one
/// of these.
struct RegisteredDef {
    schema: &'static EffectSchema,
    /// The frames the instance reads, as `frames_needed` answers them. `None` is
    /// every effect but a retimer.
    window: Option<Vec<i32>>,
}

impl EffectDef for RegisteredDef {
    fn schema(&self) -> &'static EffectSchema {
        self.schema
    }
    fn frames_needed(&self, _inst: &EffectInstance, _lt: f64) -> Option<Vec<i32>> {
        self.window.clone()
    }
}

/// A leaked declaration under a plugin-shaped name, exactly as the OFX host
/// leaks one for a plugin it has just described.
fn a_registered_schema(
    match_name: &'static str,
    temporal: &'static [i32],
) -> &'static EffectSchema {
    Box::leak(Box::new(EffectSchema {
        match_name,
        label: "Registered",
        version: 1,
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            temporal,
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[],
        groups: &[],
        enabled_when: &[],
        matte: MatteRole::None,
    }))
}

/// An instance of a registered effect, in the namespace a plugin's instance
/// carries.
fn a_plugin_instance(match_name: &str) -> EffectInstance {
    let mut inst = instantiate(match_name).expect("the catalogue knows it");
    assert_eq!(
        inst.effect.namespace,
        crate::model::EffectNamespace::Ofx,
        "an ofx: name instantiates in the plugin namespace"
    );
    inst.enabled = true;
    inst
}

/// The whole point of the widening: an effect nobody compiled in is found by
/// the same lookup a built-in is found by, and the built-in menu order is
/// untouched by its arrival.
#[test]
fn a_registered_definition_joins_the_catalogue_behind_the_builtins() {
    let before: Vec<&str> = BUILTIN_DEFS.iter().map(|d| d.schema().match_name).collect();
    let builtin_count = BUILTINS.len();

    let schema = a_registered_schema("ofx:test.core.joins", &[0]);
    let def: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema,
        window: None,
    }));
    assert!(BUILTIN_DEFS.register(def), "it registered");

    // Found by name, and it is the definition that was registered.
    let found = BUILTIN_DEFS
        .get("ofx:test.core.joins")
        .expect("the catalogue answers to it");
    assert_eq!(found.schema().match_name, "ofx:test.core.joins");
    assert!(std::ptr::eq(found.schema(), schema));
    assert!(crate::fx::schema("ofx:test.core.joins").is_some());

    // The built-ins are still the first `BUILTINS.len()` entries, in their own
    // order: the Add-effect menu, the command palette and the preset browser all
    // read that order, and a plugin must never move it.
    let after: Vec<&str> = BUILTIN_DEFS.iter().map(|d| d.schema().match_name).collect();
    assert_eq!(
        &after[..builtin_count],
        &BUILTINS.iter().map(|s| s.match_name).collect::<Vec<_>>()[..],
        "a plugin reordered the built-in menu"
    );
    assert_eq!(&after[..builtin_count], &before[..builtin_count]);
    assert!(after.contains(&"ofx:test.core.joins"));
    assert!(BUILTIN_DEFS.len() > builtin_count);

    // A second registration under the same name is a rescan, not a second
    // effect: nothing is added, and the first definition still answers.
    let twin: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema: a_registered_schema("ofx:test.core.joins", &[0]),
        window: None,
    }));
    assert!(!BUILTIN_DEFS.register(twin), "a duplicate name is refused");
    let still = BUILTIN_DEFS
        .get("ofx:test.core.joins")
        .expect("still there");
    assert!(std::ptr::eq(still.schema(), schema));

    // And a name the built-ins already own is refused too, whichever way round.
    let shadow: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema: a_registered_schema("blur", &[0]),
        window: None,
    }));
    assert!(
        !BUILTIN_DEFS.register(shadow),
        "a built-in cannot be shadowed"
    );
    assert_eq!(
        crate::fx::schema("blur").map(|s| s.match_name),
        Some("blur")
    );
}

/// A retimer's declared frames reach the neighbour window, and therefore the
/// frame key and the prefetch (docs/12 §2.1). The declaration on the
/// schema is the fallback; the instance's own answer wins.
#[test]
fn a_retimers_declared_frames_land_in_the_temporal_window() {
    // Declared ±1 at describe time — the widening the host applies when a plugin
    // says it reads other frames at all.
    let schema = a_registered_schema("ofx:test.core.retimer", &[-1, 0, 1]);
    let def: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema,
        window: Some(vec![-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5]),
    }));
    assert!(BUILTIN_DEFS.register(def));

    let inst = a_plugin_instance("ofx:test.core.retimer");
    let one = std::slice::from_ref(&inst);
    assert_eq!(
        stack_temporal_window(one, true, 0.0),
        vec![-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5],
        "the plugin's own frames are what the layer decodes"
    );
    // The gate in front of it is the declaration, which is why the widening at
    // describe time matters.
    assert!(stack_is_temporal(one, true));

    // Bypassed, disabled, and a plain built-in stack are all untouched.
    assert_eq!(stack_temporal_window(one, false, 0.0), vec![0]);
    let mut off = inst.clone();
    off.enabled = false;
    assert_eq!(
        stack_temporal_window(std::slice::from_ref(&off), true, 0.0),
        vec![0]
    );

    // A registered effect that answers `None` falls back to its declaration,
    // exactly as every built-in does.
    let plain_schema = a_registered_schema("ofx:test.core.plain", &[-1, 0, 1]);
    let plain: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema: plain_schema,
        window: None,
    }));
    assert!(BUILTIN_DEFS.register(plain));
    let plain_inst = a_plugin_instance("ofx:test.core.plain");
    assert_eq!(
        stack_temporal_window(std::slice::from_ref(&plain_inst), true, 0.0),
        vec![-1, 0, 1]
    );
}

/// A plugin instance resolves into the arena beside the built-ins, in stack
/// order, carrying the layer time its parameters were read at.
#[test]
fn a_plugin_instance_resolves_between_two_builtins() {
    let schema = a_registered_schema("ofx:test.core.stack", &[0]);
    let def: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema,
        window: None,
    }));
    assert!(BUILTIN_DEFS.register(def));

    let stack = vec![
        instantiate("invert").expect("a built-in"),
        a_plugin_instance("ofx:test.core.stack"),
        instantiate("exposure").expect("a built-in"),
    ];
    let resolved = super::resolve_stack(
        &stack,
        1.5,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    let names: Vec<&str> = resolved
        .iter()
        .map(|op| op.def.schema().match_name)
        .collect();
    assert_eq!(names, vec!["invert", "ofx:test.core.stack", "exposure"]);
    let plugin_op = resolved.get(1).expect("the middle op");
    assert!(
        (plugin_op.lt - 1.5).abs() < 1e-9,
        "the op carries the layer time its values were read at"
    );
}

/// Pixel sort (docs/08 §3.99) on the CPU alone: the four promises the effect
/// makes about *which* pixels may move, none of which needs a card.
///
/// The §1.6 oracle in `lumit-gpu` holds the kernel to this function, and it
/// skips on a machine with no adapter — so the guarantees themselves are
/// stated here, where they always run.
#[test]
fn pixel_sort_moves_pixels_only_inside_their_own_span() {
    use super::cpu::{self, PixelSortParams};
    use super::effects::pixel_sort::PixelSort;
    use super::{EffectMetadata, Params};

    // A line of distinct greys, so a pixel can be recognised wherever it lands.
    // Luminance is the weighted sum of the three, which for a grey is the grey.
    let grey = |g: f32| [g, g, g, 1.0];
    let line = |w: usize, f: &dyn Fn(usize) -> f32| -> Vec<f32> {
        (0..w).flat_map(|i| grey(f(i))).collect()
    };
    let base = PixelSortParams {
        sort_by: 3,
        vertical: false,
        span_mode: 0,
        reverse: false,
        offset_scale: 1.0,
        min: 0.0,
        max: 1.0,
        stride: 4,
        seed: 20_260_909,
        mix: 1.0,
    };

    // 1. **The cap really caps.** Sixteen greys running downhill, with a span
    //    length of four: no pixel may travel further than a span is long, and
    //    the line as a whole must come back unsorted.
    let w = 16usize;
    let img = line(w, &|i| (16 - i) as f32 / 20.0);
    let mut out = img.clone();
    cpu::pixel_sort(&mut out, w as u32, 1, &base);
    for i in 0..w {
        let v = out[i * 4];
        let was = img
            .chunks_exact(4)
            .position(|c| c[0] == v)
            .expect("every pixel that comes out went in");
        assert!(
            i.abs_diff(was) < base.stride as usize,
            "pixel {was} travelled to {i}, further than a span of {}",
            base.stride
        );
    }
    let mut sorted: Vec<f32> = img.chunks_exact(4).map(|c| c[0]).collect();
    sorted.sort_by(f32::total_cmp);
    let got: Vec<f32> = out.chunks_exact(4).map(|c| c[0]).collect();
    assert_ne!(
        got, sorted,
        "a span of four must not sort a line of sixteen"
    );

    // 2. **It is a rearrangement, never a grade.** Whatever the span mode, the
    //    line comes back holding texels it already had — and under Sort and
    //    Mirror it holds every one of them exactly once.
    for mode in [0u32, 1, 2] {
        let mut out = img.clone();
        cpu::pixel_sort(
            &mut out,
            w as u32,
            1,
            &PixelSortParams {
                span_mode: mode,
                ..base
            },
        );
        for c in out.chunks_exact(4) {
            assert!(
                img.chunks_exact(4).any(|o| o == c),
                "mode {mode} invented a pixel that was not in the line"
            );
        }
        if mode != 1 {
            let mut a: Vec<u32> = out.chunks_exact(4).map(|c| c[0].to_bits()).collect();
            let mut b: Vec<u32> = img.chunks_exact(4).map(|c| c[0].to_bits()).collect();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "mode {mode} must be a permutation of the line");
        }
    }

    // 3. **A pixel outside the band never moves.** The greys either side of
    //    0.15 and 0.85 are in no span at all, so they must come back where they
    //    were whatever the pixels around them did.
    let outside = [0.05f32, 0.95];
    let img = line(w, &|i| {
        if i % 5 == 0 {
            outside[(i / 5) % 2]
        } else {
            0.8 - (i % 5) as f32 * 0.12
        }
    });
    let banded = PixelSortParams {
        min: 0.15,
        max: 0.85,
        stride: 1000,
        ..base
    };
    let mut out = img.clone();
    cpu::pixel_sort(&mut out, w as u32, 1, &banded);
    assert_ne!(out, img, "something inside the band has to have moved");
    for i in (0..w).step_by(5) {
        assert_eq!(
            out[i * 4..i * 4 + 4],
            img[i * 4..i * 4 + 4],
            "the pixel at {i} is outside the band and must not have moved"
        );
    }

    // 4. **Direction is a transpose and nothing else.** Sorting a turned
    //    picture down its columns has to give the turned answer, offsets and
    //    all — which is the whole of what the Direction row is allowed to
    //    change.
    let (w, h) = (7usize, 5usize);
    let flat: Vec<f32> = (0..w * h)
        .flat_map(|i| grey(0.1 + (i * 37 % 61) as f32 / 80.0))
        .collect();
    let turn = |v: &[f32], w: usize, h: usize| -> Vec<f32> {
        let mut t = vec![0.0; v.len()];
        for y in 0..h {
            for x in 0..w {
                t[(x * h + y) * 4..(x * h + y) * 4 + 4]
                    .copy_from_slice(&v[(y * w + x) * 4..(y * w + x) * 4 + 4]);
            }
        }
        t
    };
    let across = PixelSortParams {
        stride: 3,
        ..banded
    };
    let mut rows = flat.clone();
    cpu::pixel_sort(&mut rows, w as u32, h as u32, &across);
    let mut cols = turn(&flat, w, h);
    cpu::pixel_sort(
        &mut cols,
        h as u32,
        w as u32,
        &PixelSortParams {
            vertical: true,
            ..across
        },
    );
    assert_eq!(
        turn(&rows, w, h),
        cols,
        "Vertical on a turned picture must be the turned Horizontal answer"
    );

    // 5. **The ceiling is the declaration's, and it is enforced once.** A span
    //    longer than the workgroup arrays cannot be asked for, and a span of
    //    nothing is a span of one.
    let of = |max_span: f32| {
        let mut s = PixelSort::read(Params::EMPTY);
        s.max_span = max_span;
        s.packed().stride
    };
    assert_eq!(of(5000.0), cpu::PIXEL_SORT_MAX_SPAN);
    assert_eq!(of(0.0), 1);
    assert_eq!(of(300.0), 300);
}
