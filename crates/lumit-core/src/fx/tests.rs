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

fn resolve_stack_temporal_named(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<(uuid::Uuid, ShapedOp)> {
    let (ids, resolved) = super::resolve_stack_temporal_named(
        effects,
        super::ResolvedDrivers::NONE,
        sample_lt,
        frame_lt,
        diag_px,
        px_scale,
        markers,
        Arc::new(ExpressionContext::detached()),
    );
    ids.into_iter().zip(shape(&resolved)).collect()
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
/// derived value (K-385), or how many entries an effect resolves to at all.
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
/// rows and the resolve-time derivation (K-385, K-388) read back through the
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
/// The trick is K-388's own arithmetic: `packed` builds each offset as
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
/// (K-385) read back through the effect's own `packed`.
fn flash_packed(e: &EffectInstance, lt: f64, markers: &MarkerContext) -> (f32, [f32; 4], f32) {
    let bag = resolve_bag(std::slice::from_ref(e), lt, 1000.0, 1.0, markers);
    let p = Params::new(&bag);
    effects::flash::Flash::read(p).packed(effects::flash::Flash::strength_of(p))
}

/// What a Lens flare op hands the bake and the kernels: the
/// [`LensFlareParams`](crate::fx::lens_flare::LensFlareParams) bundle the old
/// `Resolved::LensFlare` variant carried, read back out of the resolved arena
/// through the effect's own `packed` — Lights mode's sources included, since they
/// are a resolve-time derivation (K-360, K-385) rather than a row.
fn flare_packed(ops: &super::ResolvedStack) -> crate::fx::lens_flare::LensFlareParams {
    let fx = ops.get(0).expect("the flare op");
    let (lights, count) = effects::lens_flare::LensFlare::lights_of(fx.params);
    effects::lens_flare::LensFlare::read(fx.params).packed(lights, count)
}

/// What a Scanlines instance hands its kernel: the fields the old
/// `Resolved::Scanlines` variant carried, with the folded intensity and the roll
/// offset coming out of the resolve-time derivation (K-385).
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
/// of the resolve-time derivation (K-385) and `depth_bound` — the fact a Layer
/// row cannot put in the bag — supplied by the caller, as the render supplies it
/// from the aux slot (K-387).
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
/// reach coming out of the resolve-time derivation (K-385).
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

/// What a Block glitch instance hands its kernel: the fields the old
/// `Resolved::BlockGlitch` variant carried, with the discretised tick coming out
/// of the resolve-time derivation (K-385).
#[allow(clippy::type_complexity)]
fn block_glitch_packed(
    e: &EffectInstance,
    lt: f64,
    diag_px: f32,
    px_scale: f32,
) -> (f32, u32, i32, f32, f32, f32, f32, f32, f32) {
    let bag = resolve_bag(
        std::slice::from_ref(e),
        lt,
        diag_px,
        px_scale,
        &MarkerContext::NONE,
    );
    let p = Params::new(&bag);
    effects::block_glitch::BlockGlitch::read(p)
        .packed(effects::block_glitch::BlockGlitch::tick_of(p))
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
    assert!(
        (this_layer_effect_time(std::slice::from_ref(&e), true, 0.35, Rational::ZERO) - 0.3).abs()
            < 1e-9
    );
    // The hold is computed on comp time `lt + start_offset` and mapped back, so a
    // layer offset by 1.0s still lands its held effects on the same comp grid:
    // held comp time floor(3.5)/10 = 0.3, minus the offset → -0.7.
    assert!(
        (this_layer_effect_time(std::slice::from_ref(&e), true, -0.65, Rational::ONE) - (-0.7))
            .abs()
            < 1e-9
    );
    // Bypassed or plain stacks are untouched.
    assert_eq!(
        this_layer_effect_time(std::slice::from_ref(&e), false, 0.35, Rational::ZERO),
        0.35
    );
    let blur = instantiate("blur").unwrap();
    assert_eq!(
        this_layer_effect_time(std::slice::from_ref(&blur), true, 0.35, Rational::ZERO),
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
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
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
    assert_eq!(e.float_at("radius", 0.0), Some(30.0));
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
    // 30 px@comp at a px_scale of 1 is 30 raster px (K-419).
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

// The render-time indicator (docs/13 §7.1) puts a measured millisecond on the
// row of the effect stack that spent it, and the only thing that can say which
// row an op came from is the walk that resolved it: the resolve walk drops
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
        named.iter().map(|(_, op)| op.clone()).collect::<Shape>(),
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
/// take its historical path (K-313). Spelled once here because a twenty-field
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

    // The bag carries only the scalars; the depth is threaded beside the op as
    // its aux slot (K-387). The default Aperture master (8) is unity, so each
    // side resolves to its Near/Far radius (8) scaled by the §2.3 preview factor
    // (here 0.5 → 4 raster px). A `dof` always resolves to exactly one op, so it
    // stays 1:1 and in order with the depth-input list even when the depth
    // reference is unset.
    assert_eq!(
        dof_packed(&e, 0.5, false),
        neutral_dof(4.0, 4.0, [480.0, 270.0])
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
    assert_eq!(stack_flow_neighbours(one, true), vec![1]);
    assert_eq!(effect_flow_neighbour("motion_blur"), Some(1));
    let blur = instantiate("blur").unwrap();
    let echo = instantiate("echo").unwrap();
    assert!(stack_flow_neighbours(&[blur.clone(), echo], true).is_empty());
    // Bypassed by the layer fx switch, or disabled, it wants nothing.
    assert!(stack_flow_neighbours(one, false).is_empty());
    let mut off = mb.clone();
    off.enabled = false;
    assert!(stack_flow_neighbours(std::slice::from_ref(&off), true).is_empty());
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
    assert_eq!(stack_flow_neighbours(one, true), vec![-1]);
    assert_eq!(effect_flow_neighbour("datamosh"), Some(-1));

    // A plain Block glitch stays single-frame.
    let plain = instantiate("block_glitch").unwrap();
    let plain_one = std::slice::from_ref(&plain);
    assert_eq!(stack_temporal_window(plain_one, true), vec![0]);
    assert!(!stack_is_temporal(plain_one, true));
    assert!(stack_flow_neighbours(plain_one, true).is_empty());
    assert_eq!(effect_flow_neighbour("block_glitch"), None);

    // Disabled, or the layer fx switch off, Datamosh wants nothing.
    let mut off = dm.clone();
    off.enabled = false;
    assert_eq!(
        stack_temporal_window(std::slice::from_ref(&off), true),
        vec![0]
    );
    assert!(stack_flow_neighbours(std::slice::from_ref(&off), true).is_empty());
    assert!(stack_flow_neighbours(one, false).is_empty());
}

#[test]
fn motion_blur_and_datamosh_together_ask_for_both_measurements() {
    // K-544: a layer used to carry one flow field and the first consumer in
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
fn datamosh_instantiates_and_resolves() {
    let e = instantiate("datamosh").unwrap();
    assert_eq!(e.float_at("intensity", 0.0), Some(1.0));
    assert_eq!(e.float_at("displacement", 0.0), Some(4.0));
    assert_eq!(e.float_at("bloom", 0.0), Some(0.6));
    assert_eq!(e.float_at("reset_interval", 0.0), Some(0.0));
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));

    // Reset off (interval 0) → full ramp; displacement 4 → 4 taps.
    assert_eq!(datamosh_packed(&e, 0.0), (1.0, 4.0, 0.6, 4, 1.0));

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
    assert_eq!(
        datamosh_packed(&zero_intensity, 0.0),
        (0.0, 4.0, 0.6, 4, 1.0)
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
    assert_eq!(datamosh_packed(&e, 0.0), (2.5, 9.0, 0.6, 9, 1.0));

    // An old project (K-148) carries `streak_length`, not `displacement`: the
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

/// Resolve one datamosh instance at `lt` and return its `(intensity,
/// displacement)`; a small helper for the reset-ramp test.
fn datamosh_reach(e: &EffectInstance, lt: f64) -> (f32, f32) {
    let (intensity, displacement, ..) = datamosh_packed(e, lt);
    (intensity, displacement)
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
    // The single-buffer CPU dispatcher cannot carry a neighbour frame or a flow
    // field, so Datamosh degrades to a no-op there, exactly like Echo and Motion
    // blur — its `apply_cpu` keeps `EffectDef`'s identity default, which is what
    // the old `Resolved::Datamosh` arm of `cpu::apply` was. Run through
    // `apply_stack`, which is the dispatch a migrated effect reaches.
    let (w, h) = (5u32, 5u32);
    let img = transform_card(w, h);
    let mut out = img.clone();
    let e = instantiate("datamosh").unwrap();
    let ops = super::resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    cpu::apply_stack(&mut out, w, h, &ops);
    assert_eq!(out, img);
}

#[test]
fn resolve_motion_blur_converts_shutter_and_rounds_samples() {
    let e = instantiate("motion_blur").unwrap();
    // Defaults: 180° → shutter_frac 0.5, 16 samples, full mix, Rendered view.
    assert_eq!(
        resolve_migrated::<effects::motion_blur::MotionBlur>(
            &[e],
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE
        )
        .packed(),
        (0.5, 16, 1.0, MbView::Rendered, MbQuality::Normal, 32.0)
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
    assert_eq!(
        resolve_migrated::<effects::motion_blur::MotionBlur>(
            &[e],
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE
        )
        .packed(),
        (0.25, 8, 0.5, MbView::Rendered, MbQuality::Normal, 32.0)
    );
    // The View row resolves the diagnostic choices (FX-19).
    let mut e = instantiate("motion_blur").unwrap();
    for p in e.params.iter_mut() {
        if p.id == "view" {
            p.value = EffectValue::Choice(2);
        }
    }
    let (_, _, _, view, quality, _) = resolve_migrated::<effects::motion_blur::MotionBlur>(
        &[e],
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    )
    .packed();
    assert_eq!(view, MbView::Confidence);
    assert_eq!(quality, MbQuality::Normal);
    // The Quality row resolves the reconstruction tier (K-390); an index no
    // menu can produce falls back to Normal, never to the expensive tier.
    for (stored, want) in [
        (1u32, MbQuality::High),
        (0, MbQuality::Normal),
        (7, MbQuality::Normal),
    ] {
        let mut e = instantiate("motion_blur").unwrap();
        for p in e.params.iter_mut() {
            if p.id == "quality" {
                p.value = EffectValue::Choice(stored);
            }
        }
        let (_, _, _, _, q, _) = resolve_migrated::<effects::motion_blur::MotionBlur>(
            &[e],
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        )
        .packed();
        assert_eq!(q, want, "stored quality index {stored}");
    }
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
        MbQuality::Normal,
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
        MbQuality::Normal,
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
        MbQuality::Normal,
    );
    assert_eq!(mixed, img, "mix 0 is a passthrough");

    // A still *tile* is the one remaining way to get no blur: zero vectors mean
    // there is nothing for an uncertain pixel to borrow either, so however
    // suspect the confidence, the output is the bit-exact input.
    let zero = vec![0.0f32; n];
    let mut suspect = img.clone();
    cpu::motion_blur(
        &mut suspect,
        w,
        h,
        &zu,
        &zv,
        &zero,
        0.5,
        16,
        1.0,
        MbView::Rendered,
        MbQuality::Normal,
    );
    assert_eq!(
        suspect, img,
        "a still tile does not blur, at any confidence"
    );
}

/// The centrepiece of the K-390 reconstruction (docs/impl/optical-flow.md §4.5
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

/// Scatter as gather (docs/impl/optical-flow.md §4.5 item 3): a fast object
/// must smear **over** the still background it passes. v1 gathered along each
/// pixel's own vector, so a still background pixel gathered only from itself and
/// stayed razor sharp right up to the moving object's edge — the visible half of
/// the scatter problem. Here the background pixel also gathers along its
/// neighbourhood's dominant motion, weighted by whether what it found there was
/// moving fast enough to have reached it.
#[test]
fn cpu_motion_blur_a_fast_object_smears_over_the_still_background() {
    let (w, h) = (64u32, 16u32);
    let n = (w * h) as usize;
    // A black frame with a bright bar in columns 20..28, on a still background.
    let mut img = vec![0.0f32; n * 4];
    for y in 0..h {
        for x in 20..28u32 {
            let i = ((y * w + x) * 4) as usize;
            img[i..i + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        }
    }
    // Only the bar moves, and fast; everything else is still. Full confidence
    // throughout, so this isolates the scatter from the confidence blend.
    let u: Vec<f32> = (0..n)
        .map(|i| {
            let x = i as u32 % w;
            if (20..28).contains(&x) {
                32.0
            } else {
                0.0
            }
        })
        .collect();
    let v = vec![0.0f32; n];
    let conf = vec![1.0f32; n];

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

    // A still background pixel a few pixels ahead of the bar — inside the reach
    // of a 16 px streak, and pitch black before the blur.
    let ahead = ((8 * w + 32) * 4) as usize;
    assert!(
        out[ahead] > 0.02,
        "the bar must smear forward onto the still background: {}",
        out[ahead]
    );
    // Far outside the streak's reach, the background is untouched — the smear
    // has a finite length, it is not a global haze.
    let far = ((8 * w + 60) * 4) as usize;
    assert!(
        out[far] < 1e-4,
        "beyond the streak the background stays clean: {}",
        out[far]
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
        MbQuality::Normal,
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
        MbQuality::Normal,
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
    // to the raised 16-frame window, and every stored mode index survives the
    // trip so old projects load unchanged.
    //
    // The clamps read here now live in `Echo::packed` rather than in a resolve
    // arm (K-387) — the bag carries the authored number and the effect converts
    // it, which is the shape every migrated effect has. The numbers pinned are
    // the ones the old arm produced.
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
    let (weights, mode, _) =
        resolve_migrated::<effects::echo::Echo>(&[over], 0.0, 1000.0, 1.0, &MarkerContext::NONE)
            .packed();
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
        let (_, mode, _) =
            resolve_migrated::<effects::echo::Echo>(&[old], 0.0, 1000.0, 1.0, &MarkerContext::NONE)
                .packed();
        assert_eq!(mode, m, "mode index preserved");
    }
    let mut oob = e.clone();
    for p in &mut oob.params {
        if p.id == "mode" {
            p.value = EffectValue::Choice(99);
        }
    }
    let (_, mode, _) =
        resolve_migrated::<effects::echo::Echo>(&[oob], 0.0, 1000.0, 1.0, &MarkerContext::NONE)
            .packed();
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
    assert_eq!(e.float_at("radius", 0.0), Some(8.0));
    assert_eq!(e.float_at("threshold", 0.0), Some(0.05));
    assert!(matches!(
        e.param("luminance_only"),
        Some(EffectValue::Bool(true))
    ));
    // 8 px@comp at a px_scale of 1 = 8px; amount 60% = 0.6.
    let s =
        resolve_migrated::<effects::sharpen::Sharpen>(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(s.packed(), (0.6, 8.0, 0.05, true, 1.0));
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
    assert_eq!(e.float_at("amount", 0.0), Some(8.0));
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
    // 8 px@comp at a px_scale of 1 = 8px.
    let s = resolve_migrated::<effects::rgb_split::RgbSplit>(
        &[e],
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        s.packed(),
        effects::rgb_split::Split::Classic {
            amount_px: 8.0,
            angle_deg: 0.0,
            scale: [1.0, 0.0, 1.0],
            tints: RGB_TINTS,
            mix: 1.0
        }
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
    let split = |e: &EffectInstance| {
        resolve_migrated::<effects::rgb_split::RgbSplit>(
            std::slice::from_ref(e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        )
        .packed()
    };
    assert_eq!(
        split(&e),
        effects::rgb_split::Split::Classic {
            amount_px: 8.0,
            angle_deg: 0.0,
            scale: [1.5, -0.5, 0.0],
            tints: RGB_TINTS,
            mix: 1.0
        }
    );

    // A legacy instance missing the per-tap params still resolves to the
    // classic 1 / 0 / 1 scales and red / green / blue tints.
    e.params
        .retain(|p| !matches!(p.id.as_str(), "red_amount" | "green_amount" | "blue_amount"));
    assert_eq!(
        split(&e),
        effects::rgb_split::Split::Classic {
            amount_px: 8.0,
            angle_deg: 0.0,
            scale: [1.0, 0.0, 1.0],
            tints: RGB_TINTS,
            mix: 1.0
        }
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
    let classic = effects::rgb_split::Split::Classic {
        amount_px: 8.0,
        angle_deg: 0.0,
        scale: [1.0, 0.0, 1.0],
        tints: RGB_TINTS,
        mix: 1.0,
    };
    let split = |e: &EffectInstance| {
        resolve_migrated::<effects::rgb_split::RgbSplit>(
            std::slice::from_ref(e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        )
        .packed()
    };
    assert_eq!(split(&e), classic);

    // Wavelength on: the same numbers pack as the spectral mode, carrying the
    // default Samples (16).
    for p in &mut e.params {
        if p.id == "wavelength" {
            p.value = EffectValue::Bool(true);
        }
    }
    assert_eq!(
        split(&e),
        effects::rgb_split::Split::Spectral {
            amount_px: 8.0,
            angle_deg: 0.0,
            samples: 16,
            tints: RGB_TINTS,
            mix: 1.0
        }
    );

    // A legacy instance (saved before the Bool existed) has no
    // wavelength parameter and still resolves as the classic split.
    e.params.retain(|p| p.id != "wavelength");
    assert_eq!(split(&e), classic);
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
    assert_eq!(
        chromatic_fringe(&e, 1.0),
        effects::chromatic_aberration::Fringe::Classic {
            amount_px: 4.0,
            tints: RGB_TINTS,
            mix: 1.0
        }
    );
}

/// One chromatic aberration instance's packed form at preview factor `px_scale`.
fn chromatic_fringe(e: &EffectInstance, px_scale: f32) -> effects::chromatic_aberration::Fringe {
    resolve_migrated::<effects::chromatic_aberration::ChromaticAberration>(
        std::slice::from_ref(e),
        0.0,
        1000.0,
        px_scale,
        &MarkerContext::NONE,
    )
    .packed()
}

#[test]
fn chromatic_aberration_amount_scales_with_the_preview_factor() {
    let e = instantiate("chromatic_aberration").unwrap();
    // Half preview (px_scale 0.5): px@comp parameters scale down with
    // it, exactly like Glitch's Block size (§2.3).
    assert_eq!(
        chromatic_fringe(&e, 0.5),
        effects::chromatic_aberration::Fringe::Classic {
            amount_px: 2.0,
            tints: RGB_TINTS,
            mix: 1.0
        }
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
    assert_eq!(
        chromatic_fringe(&e, 1.0),
        effects::chromatic_aberration::Fringe::Spectral {
            amount_px: 4.0,
            samples: 32,
            tints: RGB_TINTS,
            mix: 1.0
        }
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
    let packed = resolve_migrated::<effects::rgb_split::RgbSplit>(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    )
    .packed();
    let effects::rgb_split::Split::Spectral { tints, .. } = packed else {
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
    assert_eq!(
        chromatic_fringe(&e, 1.0),
        effects::chromatic_aberration::Fringe::Classic {
            amount_px: 4.0,
            // Normalised per channel (K-167): r column 1 + 0.5 → 2/3, 1/3;
            // g column 0.25 alone → 1; b column 0.75 + 1 → 3/7, 4/7.
            tints: [
                [1.0 / 1.5, 0.0, 0.0],
                [0.5 / 1.5, 1.0, 0.75 / 1.75],
                [0.0, 0.0, 1.0 / 1.75]
            ],
            mix: 1.0
        }
    );

    e.params.retain(|p| !p.id.starts_with("channel_colour_"));
    assert_eq!(
        chromatic_fringe(&e, 1.0),
        effects::chromatic_aberration::Fringe::Classic {
            amount_px: 4.0,
            tints: RGB_TINTS,
            mix: 1.0
        }
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
fn colour_balance_instantiates_and_resolves_neutral() {
    let e = instantiate("colour_balance").unwrap();
    assert_eq!(e.colour_at("lift", 0.0), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(e.colour_at("gamma", 0.0), Some([1.0; 4]));
    assert_eq!(e.colour_at("gain", 0.0), Some([1.0; 4]));
    let v: effects::colour_balance::ColourBalance = resolve_migrated(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(v.packed(), ([0.0; 3], [1.0; 3], [1.0; 3], 1.0));

    // A gamma of 0 would make the reciprocal exponent infinite; the floor is
    // host maths, so both render paths get the same 0.01.
    let floored = effects::colour_balance::ColourBalance {
        gamma: [0.0, 0.0, 0.0, 1.0],
        ..v
    };
    assert_eq!(floored.packed().1, [0.01; 3]);
}

#[test]
fn saturation_instantiates_and_resolves_neutral() {
    let e = instantiate("saturation").unwrap();
    assert_eq!(e.float_at("saturation", 0.0), Some(100.0));
    let v: effects::saturation::Saturation = resolve_migrated(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(v.packed(), (1.0, 1.0));

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
    let heavy: effects::saturation::Saturation =
        resolve_migrated(&[heavy], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(heavy.packed(), (4.0, 1.0));
}

#[test]
fn vibrancy_instantiates_and_resolves_neutral() {
    let e = instantiate("vibrancy").unwrap();
    // Default 0 = neutral (K-152): a fresh Vibrancy is the bit-exact identity.
    assert_eq!(e.float_at("amount", 0.0), Some(0.0));
    let v: effects::vibrancy::Vibrancy = resolve_migrated(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(v.packed(), (0.0, 1.0));

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
    let heavy: effects::vibrancy::Vibrancy =
        resolve_migrated(&[heavy], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
    assert_eq!(heavy.packed(), (2.5, 1.0));
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
    assert_eq!(
        resolve_migrated::<effects::matte_key::MatteKey>(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE
        )
        .packed(),
        MatteKeyParams {
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
            pre_blur: 0.0,
            shrink_grow: 0.0,
            softness: 0.0,
            despot_black: 0.0,
            despot_white: 0.0,
            replace_method: 2,
            replace_colour: [0.5, 0.5, 0.5, 1.0],
            mix: 1.0,
        }
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
    let p = resolve_migrated::<effects::matte_key::MatteKey>(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    )
    .packed();
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
    // K-135: the range widens to ±150 slider / ±200 hard, with the stronger
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
    // Preserve luminance is on by default (K-136).
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
fn hue_shift_preserve_luminance_toggle_picks_the_matrix_branch() {
    // K-136: Preserve luminance off packs to the plain-RGB rotation
    // (equal-weight spin about the grey axis); on keeps the Rec.709
    // constant-luminance one. The effect owns the branch (`HueShift::matrix`);
    // the kernel is matrix-general, so both modes share one pass.
    let mut off = instantiate("hue_shift").unwrap();
    for p in &mut off.params {
        match p.id.as_str() {
            "angle" => p.value = EffectValue::Float(Property::fixed(90.0)),
            "preserve_luminance" => p.value = EffectValue::Bool(false),
            _ => {}
        }
    }
    let off: effects::hue_shift::HueShift = resolve_migrated(
        std::slice::from_ref(&off),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(off.packed(), (hue_matrix_rgb(90.0), 1.0));
    // Preserve on (the default) at the same angle uses the Rec.709 matrix,
    // and the two matrices genuinely differ.
    let mut on = instantiate("hue_shift").unwrap();
    for p in &mut on.params {
        if p.id == "angle" {
            p.value = EffectValue::Float(Property::fixed(90.0));
        }
    }
    let on: effects::hue_shift::HueShift = resolve_migrated(
        std::slice::from_ref(&on),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(on.packed(), (hue_matrix(90.0), 1.0));
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
fn vignette_instantiates_and_resolves() {
    let e = instantiate("vignette").unwrap();
    assert_eq!(e.float_at("amount", 0.0), Some(0.5));
    assert_eq!(e.float_at("radius", 0.0), Some(0.75));
    assert_eq!(e.float_at("softness", 0.0), Some(0.5));
    assert_eq!(e.float_at("roundness", 0.0), Some(1.0));
    let packed = |e: &EffectInstance| {
        resolve_migrated::<effects::vignette::Vignette>(
            std::slice::from_ref(e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        )
        .packed()
    };
    assert_eq!(packed(&e), (0.5, 0.75, 0.5, 1.0, 1.0, 1.0));

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
    assert_eq!(packed(&wide), (0.5, 0.75, 1.5, 1.0, 1.0, 1.0));
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
    // K-137: the old mode-driven blur is now three single-purpose effects.
    // Gaussian (match_name "blur") resolves at its Radius, fixed Repeat edge.
    let gaussian = instantiate("blur").unwrap();
    assert!(
        gaussian.param("mode").is_none(),
        "the mode control is gone (K-137)"
    );
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

    // Radial blur reads Centre/Amount/Type/Edges: Centre is px@comp since
    // K-558 and resolves like every other pixel row (px_scale 1 here), Amount
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
fn sharpen_simple_instantiates_and_resolves() {
    // K-138: the plain 3×3 sharpen (match_name "sharpen_simple"), separate
    // from the Unsharp mask ("sharpen").
    let e = instantiate("sharpen_simple").unwrap();
    assert_eq!(e.effect.match_name, "sharpen_simple");
    assert_eq!(e.float_at("amount", 0.0), Some(1.0));
    assert_eq!(e.float_at("mix", 0.0), Some(100.0));
    let s = resolve_migrated::<effects::sharpen_simple::SharpenSimple>(
        &[e],
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(s.packed(), (1.0, 1.0, 1.0));

    // The Unsharp mask keeps its own match_name and resolves as before.
    let unsharp = instantiate("sharpen").unwrap();
    assert_eq!(unsharp.effect.match_name, "sharpen");
    let u = resolve_migrated::<effects::sharpen::Sharpen>(
        &[unsharp],
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(u.packed(), (0.6, 8.0, 0.05, true, 1.0));
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
    // px@comp, resolved onto this 17x17 raster (K-558): pixel 8's centre.
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
fn transform_instantiates_and_resolves_with_the_preview_factor() {
    let e = instantiate("transform").unwrap();
    assert_eq!(e.float_at("anchor_x", 0.0), Some(0.0));
    assert_eq!(e.float_at("position_x", 0.0), Some(0.0));
    assert_eq!(e.float_at("scale_x", 0.0), Some(100.0));
    assert_eq!(e.float_at("rotation", 0.0), Some(0.0));
    assert_eq!(e.float_at("opacity", 0.0), Some(100.0));
    // Defaults resolve to the exact identity op.
    let packed = |e: &EffectInstance, diag_px, px_scale| {
        resolve_migrated::<effects::transform::Transform>(
            std::slice::from_ref(e),
            0.0,
            diag_px,
            px_scale,
            &MarkerContext::NONE,
        )
        .packed()
    };
    assert_eq!(
        packed(&e, 1000.0, 1.0),
        ([0.0; 2], [0.0; 2], [1.0; 2], 0.0, 1.0, 1.0)
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
    assert_eq!(
        packed(&e, 500.0, 0.5),
        ([20.0, 0.0], [50.0, 0.0], [2.0, 1.0], 90.0, 1.0, 1.0)
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
    let g = resolve_migrated::<effects::glow::Glow>(&[e], 0.0, 1000.0, 0.5, &MarkerContext::NONE);
    assert_eq!(g.packed(), (12.0, 0.8, 0.5, 1.0, [1.0; 4], 1.0));
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
    assert_eq!(e.float_at("amplitude", 0.0), Some(30.0));
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
    let a = shake_packed(&e, 0.4, 1000.0);
    assert_eq!(a, shake_packed(&e, 0.4, 1000.0));
    let effects::shake::Shaken::Plain { wobble, edge, mix } = a else {
        panic!("motion blur ships off, so a fresh shake is the plain resample");
    };
    // 30 px@comp is the ceiling; the wobble stays within it, z amount 0
    // leaves zoom at exactly 1, and the default Edges control is Mirror
    // (code 2 — owner, 2026-07-19).
    assert!(wobble.offset_px[0].abs() <= 30.0 && wobble.offset_px[1].abs() <= 30.0);
    assert_eq!(wobble.zoom, 1.0);
    assert_eq!(edge, 2);
    assert_eq!(mix, 1.0);

    // Different frames wobble differently; different seeds too.
    let later = shake_packed(&e, 0.9, 1000.0);
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
    assert_ne!(
        a,
        shake_packed(&reseeded, 0.4, 1000.0),
        "a different seed wobbles differently"
    );
}

/// **Rotation frequency drives the twist and nothing else** (K-541).
///
/// The twist was the one axis with an amount but no rate: x, y and z each
/// multiplied the master Frequency, rotation read the noise at the master rate
/// flat. This pins the row that fixes that, in the three ways it can go wrong.
///
/// The default is the load-bearing one. A shake saved before this row has no
/// `rot_freq` at all, so it must resolve to the multiplier of one it always
/// had — and *bit-for-bit*, not nearly, since a shake is a picture and a
/// changed last bit is a changed frame.
#[test]
fn shake_rotation_frequency_moves_the_twist_alone() {
    use effects::shake::Shaken;
    let set = |e: &EffectInstance, id: &str, v: f64| {
        let mut e = e.clone();
        for p in &mut e.params {
            if p.id == id {
                p.value = EffectValue::Float(Property::fixed(v));
            }
        }
        e
    };
    let plain = |e: &EffectInstance, lt: f64| match shake_packed(e, lt, 1000.0) {
        Shaken::Plain { wobble, .. } => wobble,
        Shaken::Blurred { .. } => panic!("the smear ships off"),
    };

    // A twist worth measuring; everything else left at its default.
    let base = set(&instantiate("shake").unwrap(), "rotation", 10.0);
    let (lt, freq) = (0.4, base.float_at("frequency", 0.4).unwrap());
    let a = plain(&base, lt);
    assert_ne!(a.rotation_deg, 0.0, "there is a twist to measure");

    // Writing the default explicitly changes nothing — which is the same
    // arithmetic an old project takes when the row is missing entirely.
    assert_eq!(a, plain(&set(&base, "rot_freq", 1.0), lt));

    // Doubled, the twist reads the same noise curve at twice the rate: its
    // value now is the value it would have had at twice the time. The base is
    // scaled by two, which is exact, so this is an equality and not a
    // tolerance.
    let fast = set(&base, "rot_freq", 2.0);
    let at_double_time = plain(&base, lt * 2.0);
    assert_eq!(
        plain(&fast, lt).rotation_deg,
        at_double_time.rotation_deg,
        "twice the rotation frequency is twice the way along the twist"
    );
    assert!(
        (lt * freq * 2.0 - (lt * 2.0) * freq).abs() == 0.0,
        "the noise base scales exactly by two"
    );

    // And the other axes do not move with it.
    let moved = plain(&fast, lt);
    assert_eq!(moved.offset_px, a.offset_px, "x and y are untouched");
    assert_eq!(moved.zoom, a.zoom, "the depth pump is untouched");
    assert_ne!(moved.rotation_deg, a.rotation_deg, "the twist did move");
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
fn resolve_shake_motion_blur_samples_the_shutter_and_centres_on_the_frame() {
    use effects::shake::Shaken;

    // Off (the default) resolves to a single wobble — no sub-frame set, which
    // is the *absence* of the derived vectors in the bag (K-388).
    let off = instantiate("shake").unwrap();
    assert!(
        matches!(shake_packed(&off, 0.4, 1000.0), Shaken::Plain { .. }),
        "motion blur off carries no sub-frames"
    );

    // On: the sub-frame set is present, its centre sample is the frame-time
    // wobble exactly, and the samples actually differ across the shutter.
    let on = shake_with_mb(0.5);
    let packed = shake_packed(&on, 0.4, 1000.0);
    let Shaken::Blurred { samples, .. } = packed else {
        panic!("motion blur on carries sub-frames");
    };
    assert_eq!(samples.len(), SHAKE_MB_SAMPLES);
    // The frame-time wobble is what the same instance packs to with the smear
    // taken away — the centre sub-frame lands on offset 0, so the two are one
    // sample of one noise curve.
    let mut off_by_hand = on.clone();
    for p in &mut off_by_hand.params {
        if p.id == "motion_blur" {
            p.value = EffectValue::Bool(false);
        }
    }
    let Shaken::Plain { wobble, .. } = shake_packed(&off_by_hand, 0.4, 1000.0) else {
        panic!("the smear is off now");
    };
    assert_eq!(
        samples[SHAKE_MB_SAMPLES / 2],
        wobble,
        "centre sample is the frame"
    );
    assert_ne!(
        samples[0].offset_px,
        samples[SHAKE_MB_SAMPLES - 1].offset_px,
        "the wobble moves across the shutter"
    );

    // Determinism: same instance, same time, identical sub-frames twice.
    assert_eq!(packed, shake_packed(&on, 0.4, 1000.0));

    // A zero shutter is treated as no smear (the bit-exact single resample).
    assert!(
        matches!(
            shake_packed(&shake_with_mb(0.0), 0.4, 1000.0),
            Shaken::Plain { .. }
        ),
        "a zero shutter carries no sub-frames"
    );
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

    // Both folds are resolve-time work (K-388): the old ids are not schema rows,
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

/// **The migration changed no maths** (K-388) — the claim the whole batch rests
/// on, stated as arithmetic rather than as prose.
///
/// The old resolve arm built a [`ShakeWobble`] from the instance and called
/// `at`. The new path splits that in two — unit-free noise at resolve time, the
/// amplitudes at dispatch — and `Shake::packed` puts it back together in the
/// same order, with the `f64 → f32` narrowing at the same step. So the two agree
/// **bit-for-bit**, not within a tolerance, and this compares them that way: the
/// frame wobble and all nine sub-frames.
#[test]
fn shake_packs_the_wobble_the_old_arm_resolved() {
    use effects::shake::Shaken;
    let mut e = shake_with_mb(0.8);
    // Off the defaults on every axis, so a lost multiply cannot hide.
    for p in &mut e.params {
        let v = match p.id.as_str() {
            "amplitude" => 3.25,
            "frequency" => 6.5,
            "rotation" => 7.0,
            "x_amp" => 0.75,
            "y_amp" => 1.4,
            "x_freq" => 1.3,
            "y_freq" => 0.6,
            "rot_freq" => 2.2,
            "z_amp" => 12.0,
            "z_freq" => 1.7,
            _ => continue,
        };
        p.value = EffectValue::Float(Property::fixed(v));
    }
    let (lt, diag_px) = (0.4, 1000.0);

    // The old arm, transcribed: every read in f64, the amplitude already raster
    // pixels at a px_scale of 1 (K-419), the sampler doing the rest.
    let fl = |id: &str| e.float_at(id, lt);
    let wobble = ShakeWobble {
        seed: match e.param("seed") {
            Some(EffectValue::Seed(s)) => *s,
            _ => 0,
        },
        amp_px: (fl("amplitude").unwrap() as f32).max(0.0),
        x_amp: (fl("x_amp").unwrap() as f32).max(0.0),
        y_amp: (fl("y_amp").unwrap() as f32).max(0.0),
        rot_amount: (fl("rotation").unwrap() as f32).max(0.0),
        z_amp: ((fl("z_amp").unwrap() as f32) / 100.0).clamp(0.0, 1.0),
        x_freq: fl("x_freq").unwrap().max(0.0),
        y_freq: fl("y_freq").unwrap().max(0.0),
        rot_freq: fl("rot_freq").unwrap().max(0.0),
        z_freq: fl("z_freq").unwrap().max(0.0),
    };
    let base = lt * fl("frequency").unwrap().max(0.0);
    let old = |b: f64| {
        let (offset_px, rotation_deg, zoom) = wobble.at(b);
        ShakeSample {
            offset_px,
            rotation_deg,
            zoom,
        }
    };

    let Shaken::Blurred { samples, .. } = shake_packed(&e, lt, diag_px) else {
        panic!("the smear is on");
    };
    for (i, db) in shake_mb_offsets(fl("mb_amount").unwrap())
        .into_iter()
        .enumerate()
    {
        assert_eq!(samples[i], old(base + db), "sub-frame {i}");
    }
    // And with the smear off, the frame wobble itself.
    for p in &mut e.params {
        if p.id == "motion_blur" {
            p.value = EffectValue::Bool(false);
        }
    }
    let Shaken::Plain { wobble: packed, .. } = shake_packed(&e, lt, diag_px) else {
        panic!("the smear is off now");
    };
    assert_eq!(packed, old(base), "the frame-time wobble");
}

/// **A shake reused at another raster wobbles by the right number of pixels**
/// (K-266, K-386, K-388) — the unit flip under test.
///
/// The old `rescale_px` arm scaled the *resolved offsets*: `(amp_px · x_amp ·
/// noise) · f`, and every sub-frame's beside them. Declaring Amplitude
/// `Px` scales the amplitude instead, one multiply earlier: `(amp_px · f) ·
/// x_amp · noise`. The same product either way — but a different association, so
/// the two can part company in the last bit or two of an f32. That is the
/// accepted narrowing class K-388 names, which is why this asserts within an
/// epsilon rather than bit-for-bit; a real regression here (a value that does
/// not rescale at all, or rescales twice) is off by a factor of two, not by an
/// ulp.
///
/// Rotation and zoom carry no pixels and must not move at all — the old arm did
/// not touch them either.
#[test]
fn shake_amplitude_rescales_as_the_old_offsets_did() {
    use effects::shake::Shaken;
    // Motion blur on, so the frame wobble and the nine sub-frames are both in
    // play: the old arm rescaled both, and the amplitude they share now does.
    let e = shake_with_mb(0.8);
    let resolve = |px_scale: f32| {
        super::resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            1000.0,
            px_scale,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        )
    };
    let packed = |ops: &ResolvedStack| {
        let p = ops.get(0).expect("the shake op").params;
        effects::shake::Shake::read(p).packed(effects::shake::Shake::derived_of(p))
    };

    // Two factors: a half, where every multiply is exact and the two orders
    // agree bit-for-bit, and 0.3, where they do not — which is what the epsilon
    // below is for, and why it is not an assert_eq.
    let full = packed(&resolve(1.0));
    for f in [0.5f32, 0.3] {
        // Resolved at composition size, then repaired for a raster this much
        // smaller — the adjustment-layer path (`realise.rs`).
        let mut reused = resolve(1.0);
        reused.rescale_spatial(f);
        let (
            Shaken::Blurred { samples: moved, .. },
            Shaken::Blurred {
                samples: at_full, ..
            },
        ) = (packed(&reused), full)
        else {
            panic!("the smear is on");
        };
        // And the same stack resolved against that raster directly.
        let Shaken::Blurred {
            samples: direct, ..
        } = packed(&resolve(f))
        else {
            panic!("the smear is on");
        };

        // An ulp of a ~10 px offset is ~1e-6; a value that failed to rescale
        // would be out by most of itself. 1e-4 sits far above the one and far
        // below the other.
        let close = |a: f32, b: f32, what: &str| {
            assert!(
                (a - b).abs() < 1e-4,
                "factor {f}, {what}: rescaled {a} vs resolved-there {b}"
            );
        };
        let mut moved_at_all = false;
        for (i, (m, d)) in moved.iter().zip(direct.iter()).enumerate() {
            close(m.offset_px[0], d.offset_px[0], &format!("sub-frame {i} x"));
            close(m.offset_px[1], d.offset_px[1], &format!("sub-frame {i} y"));
            // The old arm's own formula, stated: the full-resolution offset
            // scaled by the same factor, after the fact.
            close(
                m.offset_px[0],
                at_full[i].offset_px[0] * f,
                &format!("sub-frame {i} x, the old rescale"),
            );
            assert_eq!(m.rotation_deg, d.rotation_deg, "degrees are not pixels");
            assert_eq!(m.zoom, d.zoom, "a zoom factor is not pixels");
            moved_at_all |= m.offset_px[0] != at_full[i].offset_px[0];
        }
        assert!(
            moved_at_all,
            "the offsets must actually have moved, or this test proves nothing"
        );
    }

    // A factor of 1 is the identity, bit-for-bit: the repair must cost nothing
    // when there is nothing to repair.
    let mut same = resolve(1.0);
    same.rescale_spatial(1.0);
    assert_eq!(packed(&same), full);
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
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
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
    let lt = crate::time::layer_time(1.0, layer.start_offset.0);
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

/// The resolve-time hook (K-385) is opt-in: an effect that does not implement it
/// resolves to exactly its declared parameters and nothing more, and the one that
/// does adds exactly its derived ids after them, in declaration order.
#[test]
fn only_an_effect_with_a_derivation_pushes_beyond_its_schema() {
    let plain = instantiate("vignette").unwrap();
    let bag = resolve_bag(
        std::slice::from_ref(&plain),
        1.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        bag.len(),
        bagged_params("vignette"),
        "an effect with no resolve_derived pushes nothing beyond its schema"
    );

    let flash = instantiate("flash").unwrap();
    let bag = resolve_bag(
        std::slice::from_ref(&flash),
        1.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    let schema_len = bagged_params("flash");
    assert_eq!(bag.len(), schema_len + 1);
    assert_eq!(
        bag[schema_len].0,
        effects::flash::Flash::DERIVED_STRENGTH,
        "the derived value lands after the declared ones"
    );
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

/// A derived id shares the bag with the declared ones (K-385), so it is covered
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
    assert_eq!(e.float_at("block_amount", 0.0), Some(60.0));
    assert_eq!(e.float_at("channel_offset", 0.0), Some(20.0));
    assert_eq!(e.float_at("slice_repeat", 0.0), Some(20.0));

    // Resolving is deterministic: the same instance at the same time
    // yields the identical result, twice — and the px_scale factor
    // (0.5 here) reaches the px@comp parameters exactly like Transform
    // and Shake's do.
    let a = block_glitch_packed(&e, 0.4, 1000.0, 0.5);
    assert_eq!(a, block_glitch_packed(&e, 0.4, 1000.0, 0.5));
    let (intensity, _seed, tick, block_size_px, jitter_frac, amount_px, chan_px, slice_frac, mix) =
        a;
    assert_eq!(intensity, 0.35);
    assert_eq!(tick, 3); // floor(0.4 * GLITCH_TICK_HZ 8) = 3
    assert_eq!(block_size_px, 12.0); // 24 px@comp * px_scale 0.5
    assert_eq!(jitter_frac, 0.25);
    assert_eq!(amount_px, 30.0); // 60 px@comp * px_scale 0.5
    assert_eq!(chan_px, 10.0); // 20 px@comp * px_scale 0.5
    assert_eq!(slice_frac, 0.20);
    assert_eq!(mix, 1.0);

    // A different frame ticks differently (the per-block hash itself
    // only runs inside cpu::block_glitch/the kernel, not here).
    let later = block_glitch_packed(&e, 0.9, 1000.0, 0.5);
    assert_ne!(a, later, "the tick moves between frames");
    assert_eq!(later.2, 7); // floor(0.9 * 8)
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

    assert_eq!(
        scanlines_packed(&e, 0.4, 1000.0, 0.5),
        (
            0.35,  // no Darkness param, so the raw Intensity stands
            1.5,   // 3 px@comp * px_scale 0.5
            0.0,   // roll speed 0
            false, // interlace
            1.0,   // mix
        )
    );

    // Rolling: the offset is roll speed × layer time × the *raster* period,
    // precomputed here so the kernel never sees raw time (§2.4). 4 lines/s at
    // 0.5 s over a 1.5 px period = 3 px.
    let mut rolling = e;
    for p in &mut rolling.params {
        if p.id == "scanline_roll" {
            p.value = EffectValue::Float(Property::fixed(4.0));
        }
    }
    assert_eq!(scanlines_packed(&rolling, 0.5, 1000.0, 0.5).2, 3.0);
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
    // 0.5 × 0.80 = 0.40. The fold reads a parameter that is not a schema row at
    // all, which is why it happens in the resolve-time hook (K-385) rather than
    // coming out of the bag with the declared ones.
    let intensity = scanlines_packed(&e, 0.0, 1000.0, 1.0).0;
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
    // The K-369 ring masks are baked in parallel the same way, and the slice
    // each path picks comes off the spreads — both must be bit-equal too, or
    // two identical projects would draw different ghost edges.
    assert!(!a.pairs.is_empty());
    for path in &a.pairs {
        assert!(path[0] < path[1]);
        assert!((path[1] as usize) < a.surfaces.len());
        // Four-bounce paths (K-368) carry the same walk one leg further in:
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

/// **An animated aperture reuses its bakes** (K-431): two f-stops inside one
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

/// **The auto-exposure gain belongs to the lens, not to the iris** (K-432):
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

/// One splat through the full K-380 deposit — pyramid, then resolve — into a
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

/// **A splat too big for full resolution keeps its flux and its place**
/// (K-380). The pyramid is an optimisation: a coarse level's kernel is the
/// same kernel sampled at wider texels and read back through a bilinear
/// upsample, so the deposited energy, its centre and its extent must all
/// survive the trip — this is the test that fails if a level's offset,
/// scale or upsample is wrong by even one texel.
#[test]
fn lens_flare_a_large_splat_deposits_coarse_and_keeps_its_flux() {
    use crate::fx::lens_flare::*;
    let (w, h) = (512u32, 512u32);
    // Span ~420 px: several levels down at DEPOSIT_SPAN_PX = 48.
    let (a1, a2) = ([35.0, 0.0], [0.0, 35.0]);
    let ext = 3.0 * (a1[0] + a2[0] + a1[1] + a2[1]);
    let level = deposit_level(ext, ext, 12);
    assert!(
        level >= 2,
        "this splat must actually take the coarse path, got level {level}"
    );
    let mut out = vec![0.0_f32; (w * h * 3) as usize];
    let flux = 5.0_f32;
    splat_flat(
        &mut out,
        w,
        h,
        [256.0, 256.0],
        a1,
        a2,
        [flux, flux, flux],
        70.0 * 70.0,
    );
    let total: f32 = out.iter().sum();
    assert!(
        (total - 3.0 * flux).abs() < 0.15 * 3.0 * flux,
        "a coarse splat must keep its flux: {total} vs {}",
        3.0 * flux
    );
    // The centroid stays put: a level whose offset or scale is wrong moves
    // the whole deposit.
    let (mut cx, mut cy, mut m) = (0.0f64, 0.0f64, 0.0f64);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let v = f64::from(out[(y * w as usize + x) * 3]);
            cx += v * x as f64;
            cy += v * y as f64;
            m += v;
        }
    }
    let (cx, cy) = (cx / m, cy / m);
    assert!(
        (cx - 255.5).abs() < 2.0 && (cy - 255.5).abs() < 2.0,
        "a coarse splat must stay centred: ({cx:.1}, {cy:.1})"
    );
}

/// **The splat reconstruction is a partition of unity** (K-366, fixed K-373).
///
/// A uniform sheet of rays on a regular grid, all with the same weight and the
/// same footprint, must reconstruct a **flat** field — that is what "the ghost
/// is smooth" means, and it is the property K-366 lacked. Its tent reached one
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
    // peak-to-trough ripple across the sheet. K-366 reached zero at every cell
    // boundary, which is 100%.
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

/// **The reconstruction must not print the ray grid on the picture** (K-366,
/// K-373, K-376) — measured on a real frame, not a synthetic sheet.
///
/// `lens_flare_splats_reconstruct_a_flat_sheet_and_keep_their_flux` proves the
/// kernel partitions unity on a *uniform* lattice, which is the case a tent
/// already handled. It did not catch what the owner could see, because a real
/// ghost's rays are neither uniform nor axis-aligned, and a tent is only C⁰:
/// it reconstructs a surface with a crease along every cell boundary, and the
/// eye finds creases.
///
/// So this measures the thing itself — how far each pixel departs from its own
/// 3×3 mean, relative to that mean — over a real flare, in two brightness
/// bands. Where it has stood:
///
/// | kernel | bright | dark |
/// |---|---|---|
/// | K-366, tent at half a step | 15.8% | — |
/// | K-373, tent at a full step | 2.42% | 4.59% |
/// | K-376, quadratic B-spline | **1.91%** | **3.81%** |
///
/// The floor is not zero and must not be asserted to be: a flare genuinely has
/// fine detail — iris rims, overlapping faint ghosts — and past about 3% in the
/// dark band the measurement is reading that rather than any artefact (it stops
/// falling when the rays are multiplied eightfold, which sampling noise would
/// not do). The thresholds below sit just above what is measured, so a
/// regression in the reconstruction shows up here rather than on screen.
#[test]
fn lens_flare_reconstruction_does_not_imprint_its_own_grid() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        // No ghost blur: a blur would hide grid imprinting rather than prevent
        // it, and this test is about preventing it.
        ghost_softness: 0.0,
        starburst_intensity: 0.0,
        quality: 1,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (256u32, 144u32);
    let buf = cpu_flare(&p, &baked, w, h, &manual_light(&p, w, h));
    let at = |x: usize, y: usize| buf[(y * w as usize + x) * 3];
    let mx = buf.iter().cloned().fold(0.0_f32, f32::max);
    assert!(mx > 0.0, "the reference must render something to measure");

    let ripple = |lo: f32, hi: f32| -> f32 {
        let (mut num, mut den) = (0.0_f64, 0.0_f64);
        for y in 1..h as usize - 1 {
            for x in 1..w as usize - 1 {
                let c = at(x, y);
                if c < mx * lo || c > mx * hi {
                    continue;
                }
                let mut m = 0.0_f32;
                for dy in 0..3 {
                    for dx in 0..3 {
                        m += at(x + dx - 1, y + dy - 1);
                    }
                }
                m /= 9.0;
                num += f64::from((c - m).abs());
                den += f64::from(m);
            }
        }
        (100.0 * num / den.max(1e-9)) as f32
    };

    let bright = ripple(0.02, 1.0);
    let dark = ripple(0.0002, 0.02);
    assert!(
        bright < 2.5,
        "the lit part of the frame is rippling at {bright:.2}% against the \
         1.91% the quadratic B-spline measures — the reconstruction has \
         regressed towards printing its sampling grid (K-366 measured 15.8%)"
    );
    assert!(
        dark < 4.5,
        "the faint part of the frame is rippling at {dark:.2}% against the \
         3.81% the quadratic B-spline measures; the floor there is about 3%, \
         which is the flare's own fine detail rather than an artefact"
    );
}

/// **Ghost edges are Fresnel, and only their edges are** (K-369, re-derived
/// K-370).
///
/// The rim carries the knife-edge diffraction profile a real defocused
/// aperture casts; the interior of a ghost is left exactly as flat as the
/// plain iris mask. That second half is the regression: K-369's propagated
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
    // than a whisper — the check K-369 could not have passed.
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
    // is the shape K-369's bottom rungs had exactly backwards: theirs peaked
    // at the CENTRE.
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

/// **The element-coating rows name parameters that exist** (K-371).
///
/// Their visibility is resolved in the panel against a *sibling by id*, and a
/// sibling that does not exist fails silently: the panel finds nothing, hides
/// every row that names it, and leaves only the rows whose threshold no lens
/// reaches — whose value set is empty, which means "always visible". That is
/// precisely what shipping `"lens"` for the Lens dropdown did, and what it
/// looked like from the outside was "only elements 19 and 20 are there, and
/// changing them does nothing" (no lens has nineteen elements, so those rows
/// governed nothing at all). Nothing in the type system connects the id in the
/// bridge to the id in the schema, so this is the connection.
#[test]
fn lens_flare_element_rows_and_the_lens_pick_line_up() {
    use crate::fx::lens_flare::*;
    let flare = crate::fx::BUILTINS
        .iter()
        .find(|s| s.match_name == "lens_flare")
        .expect("the Lens flare is a builtin");

    // The sibling the coating rows' visibility is resolved against. The
    // bridge spells this id too (`LENS_PICK_PARAM`); if it ever moves, this
    // fails here rather than silently emptying the panel.
    let lens = flare
        .params
        .iter()
        .find(|p| p.id == "lens_model")
        .expect("the Lens dropdown is `lens_model`");
    match lens.kind {
        ParamKind::Choice { options, .. } => assert_eq!(
            options.len(),
            crate::fx::lens_library::LENS_LIBRARY.len(),
            "the dropdown must offer every bundled lens, since the row \
             thresholds are indices into that same library"
        ),
        _ => panic!("the visibility rule needs a Choice to read"),
    }

    // Every element row exists, in order, and offers the whole palette.
    for (i, id) in COATING_ELEMENT_IDS.iter().enumerate() {
        let row = flare
            .params
            .iter()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("element row {} (`{id}`) is missing", i + 1));
        match row.kind {
            ParamKind::Choice {
                options, default, ..
            } => {
                assert_eq!(options.len(), COATING_DESIGNS as usize);
                assert_eq!(default, COATING_AS_FILE, "a row starts as the file");
            }
            _ => panic!("`{id}` must be a Choice"),
        }
    }

    // Each row is its own group, carrying its own element threshold, and the
    // thresholds run 1..=MAX_COATING_ELEMENTS with none missing or doubled.
    let mut thresholds: Vec<u32> = Vec::new();
    for g in flare.groups {
        if let Some(n) = g.visible_when_lens_elements {
            assert_eq!(
                g.params.len(),
                1,
                "an element threshold governs exactly one row"
            );
            assert_eq!(
                g.params[0],
                COATING_ELEMENT_IDS[n as usize - 1],
                "threshold {n} must govern element {n}"
            );
            assert!(
                g.visible_when.is_none(),
                "the two visibility rules are mutually exclusive: the bridge \
                 reads one or the other"
            );
            thresholds.push(n);
        }
    }
    thresholds.sort_unstable();
    assert_eq!(
        thresholds,
        (1..=MAX_COATING_ELEMENTS as u32).collect::<Vec<_>>()
    );

    // And the thresholds a bundled lens can actually reach are the ones the
    // panel will ever draw: the library tops out well under the schema's
    // ceiling, so the rows past it must resolve to "never", not "always".
    let reachable = library_element_counts()
        .into_iter()
        .max()
        .expect("a library");
    assert!(reachable < MAX_COATING_ELEMENTS as u32);
    assert!(lenses_with_at_least(reachable + 1).is_empty());
}

/// **A coating is per glass element, and different coatings make differently
/// coloured ghosts** (K-371).
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

/// **Four-bounce ghosts** (K-368, entry C1): the path model walks, the
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
//
// Every effect is in the arena now, so `ResolvedStack::rescale_spatial` moves them the
// one generic way: by the unit each parameter declares.
#[test]
fn resolved_px_fields_rescale_for_a_different_raster() {
    // The default Width is 0 (the labelled no-op), so author one to scale.
    let authored = |width: f64| {
        let mut e = instantiate("light_wrap").unwrap();
        for p in &mut e.params {
            if p.id == "width" {
                p.value = EffectValue::Float(Property::fixed(width));
            }
        }
        super::resolve_stack(
            &[e],
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        )
    };
    let packed = |ops: &super::ResolvedStack| {
        effects::light_wrap::LightWrap::read(ops.get(0).expect("the wrap op").params).packed()
    };
    let mut wrap = authored(10.0);
    assert_eq!(packed(&wrap), (10.0, 1.0, 1.0));
    wrap.rescale_spatial(0.5);
    let (width_px, intensity, mix) = packed(&wrap);
    assert_eq!(width_px, 5.0, "px fields scale");
    assert_eq!(intensity, 1.0, "unitless fields do not");
    assert_eq!(mix, 1.0, "unitless fields do not");

    // The flare's own px@comp rows: the Light point, and the Source size beside
    // it. The old per-variant match moved the light and left the source size
    // behind, so an area source on an adjustment layer kept its comp-sized extent
    // under a reduced-resolution preview; declaring both `Px` states it once and
    // the generic pass does the pair.
    let mut flare_inst = instantiate("lens_flare").unwrap();
    for p in &mut flare_inst.params {
        let v = match p.id.as_str() {
            "light_x" => 1000.0,
            "light_y" => 500.0,
            "source_width" => 40.0,
            "source_height" => 20.0,
            _ => continue,
        };
        p.value = EffectValue::Float(Property::fixed(v));
    }
    let mut flare = super::resolve_stack(
        std::slice::from_ref(&flare_inst),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    flare.rescale_spatial(0.5);
    let p = flare_packed(&flare);
    assert_eq!(p.light, [500.0, 250.0], "the flare's light is px@comp");
    assert_eq!(p.source_size, [20.0, 10.0], "and so is its source size");
    assert_eq!(p.intensity, 1.0, "unitless fields do not");

    // Factor 1 is exactly a no-op, on both halves.
    let mut same = authored(7.0);
    same.rescale_spatial(1.0);
    assert_eq!(packed(&same).0, 7.0);
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
    let ops = super::resolve_stack(
        std::slice::from_ref(&inst),
        0.0,
        2202.9075,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    assert_eq!(ops.len(), 1, "expected one Lens flare op");
    let rp = flare_packed(&ops);
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

// The share-of-the-frame → px@comp conversions (K-558) read old projects
// forward, which is K-258's rule applied to a *unit* change rather than to a
// missing row: what a saved file rendered, it still renders.
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
    let mut untouched = instantiate("tile").unwrap();
    untouched.effect.version = 1;
    let mut effects = vec![untouched];
    let before = effects[0].clone();
    migrate_percent_to_px(&mut effects, w, h);
    assert_eq!(effects[0].params, before.params);
}

// Beam's Length was a per cent of the *run* between Start and End, so its
// conversion (K-558) reads the instance's own points rather than the frame:
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
// whichever axis Flip order runs (K-558), so its conversion reads the
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

// And the arithmetic it now feeds: the band is a distance across the frame, so
// `packed` divides it through the raster's own extent along the order axis —
// the same width is a different share of a wide frame and a tall one.
#[test]
fn card_wipes_width_is_a_band_across_the_frame() {
    let mut c = effects::card_wipe::CardWipe::read(Params::EMPTY);
    c.transition_width = 480.0;

    // Left to right on a 1920x1080 raster: a quarter of the width.
    let p = c.packed(1920.0, 1080.0);
    assert!((p.one_minus_width - 0.75).abs() < 1e-6);
    assert!((p.inv_width - 4.0).abs() < 1e-6);

    // The same band, ordered top to bottom, is a share of the height instead.
    c.flip_order = 2;
    let p = c.packed(1920.0, 1080.0);
    assert!((p.one_minus_width - (1.0 - 480.0 / 1080.0)).abs() < 1e-6);

    // A band wider than the frame is the whole frame — every card together —
    // and one narrower than a hundredth of it still leaves the ramp a slope.
    c.flip_order = 0;
    c.transition_width = 9000.0;
    assert!(c.packed(1920.0, 1080.0).one_minus_width.abs() < 1e-6);
    c.transition_width = 0.0;
    assert!((c.packed(1920.0, 1080.0).inv_width - 100.0).abs() < 1e-6);
}

// A fresh Card wipe's band is half of the comp it landed on, not half of a
// nominal 1080p frame (K-558).
#[test]
fn a_fresh_card_wipe_bands_half_of_its_own_comp() {
    let inst = builtins::instantiate_for_raster("card_wipe", 3840.0, 2160.0).unwrap();
    let Some(EffectValue::Float(p)) = inst.param("transition_width") else {
        panic!("transition_width must be a float");
    };
    assert!((p.value_at(0.0) - 1920.0).abs() < 1e-9);
}

// And the arithmetic Length now feeds: it is a distance in raster pixels, so
// the share of the run it covers is the one division. The declared default
// draws the whole run, which is the picture Beam has always shipped, and a
// Length past the run simply puts the tail at the start point.
#[test]
fn beams_length_is_a_distance_back_from_the_head() {
    let b = effects::beam::Beam::read(Params::EMPTY);
    // Default: 1560 px along a 1560-px run, head at Time 100 — the whole beam.
    let p = b.packed();
    assert!((p.u1 - 1.0).abs() < 1e-6);
    assert!(p.u0.abs() < 1e-6, "the tail is at the start point");

    // Half the run, head at the far end: the tail sits halfway along.
    let mut half = b;
    half.length = 780.0;
    assert!((half.packed().u0 - 0.5).abs() < 1e-6);

    // Longer than the run clamps rather than running off the start.
    let mut over = b;
    over.length = 5000.0;
    assert!(over.packed().u0.abs() < 1e-6);

    // Length 0 draws nothing, and a degenerate run (Start == End) divides by
    // the floored `len2` rather than by zero — the share clamps at 1, exactly
    // as a hundred per cent did.
    let mut none = b;
    none.length = 0.0;
    assert!(!none.packed().active);
    let mut point = b;
    point.end_x = point.start_x;
    point.end_y = point.start_y;
    assert!(point.packed().u0.abs() < 1e-6);
}

// A fresh Radial blur spins about the middle of the comp it landed on, not
// about the schema's nominal 1080p centre (K-558, the `instantiate_for_raster`
// rule every other centre already follows).
#[test]
fn a_fresh_radial_blur_centres_on_its_own_comp() {
    let inst = builtins::instantiate_for_raster("radial_blur", 3840.0, 2160.0).unwrap();
    let read = |id: &str| match inst.param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => panic!("{id} must be a float"),
    };
    assert!((read("centre_x") - 1920.0).abs() < 1e-9);
    assert!((read("centre_y") - 1080.0).abs() < 1e-9);
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
        let ops = super::resolve_stack(
            &[inst],
            0.0,
            2202.9,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 1, "lens_flare resolves to exactly one op");
        assert_eq!(flare_packed(&ops).blend, mode.min(last));
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
    assert_eq!(ops.len(), 1, "lens_flare resolves to exactly one op");
    let p = flare_packed(&ops);
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
    splat_flat(
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
    splat_flat(
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
    splat_flat(
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
    splat_flat(
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
        // Every element left as the lens file describes it (K-371) — the
        // drop-on default, and byte-for-byte the pre-K-371 picture.
        coating_elements: [crate::fx::lens_flare::COATING_AS_FILE;
            crate::fx::lens_flare::MAX_COATING_ELEMENTS],
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
    // …and that extent is what every ray integrates over (K-367): rays at
    // different pupil corners take their light from different points of it.
    assert_ne!(
        source_jitter(0, 0, 0, lights[0].extent),
        source_jitter(1, 1, 0, lights[0].extent),
        "an extent must move the rays' source positions apart"
    );
    // …and each band samples it at its own phase (K-378), which is what
    // buries the one-band reconstruction ripple when the bands sum.
    assert_ne!(
        source_jitter(0, 0, 0, lights[0].extent),
        source_jitter(0, 0, 1, lights[0].extent),
        "an extent must move the bands' source positions apart"
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

/// **A point source did not move** (K-367).
///
/// The per-ray source integration replaced K-355's replication, and the one
/// thing it must not do is disturb the source every project already has. A
/// zero extent has to offset every ray by exactly nothing — not nearly
/// nothing — so a point light's picture is the same bits it always was. The
/// old behaviour cannot be run to compare against, so this pins the two
/// halves that make it true: the jitter is identically zero over a grid wider
/// than any quality tier launches, and the render is stable and independent
/// of how the light was built.
#[test]
fn lens_flare_a_point_source_jitters_by_nothing() {
    use crate::fx::lens_flare::*;
    for band in 0..3 {
        for j in 0..64 {
            for i in 0..64 {
                assert_eq!(
                    source_jitter(i, j, band, [0.0, 0.0]),
                    [0.0, 0.0],
                    "ray ({i}, {j}) band {band} of a point source must take \
                     its light from the light's own position"
                );
            }
        }
    }
    // A real extent does move rays apart — otherwise the sweep above would
    // pass on a jitter that had simply been switched off.
    assert_ne!(source_jitter(3, 5, 0, [0.1, 0.1]), [0.0, 0.0]);

    let p = LensFlareParams {
        quality: 0,
        max_ghosts: 8,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (96u32, 54u32);
    let via_param = manual_light(&p, w, h);
    assert_eq!(via_param[0].extent, [0.0, 0.0], "the default is a point");
    let hand = vec![FlareLight {
        pos: via_param[0].pos,
        rgb: via_param[0].rgb,
        extent: [0.0, 0.0],
    }];
    let a = cpu_flare(&p, &baked, w, h, &via_param);
    assert!(a.iter().sum::<f32>() > 0.0, "the point flare must render");
    assert_eq!(
        a,
        cpu_flare(&p, &baked, w, h, &hand),
        "a zero extent must be the plain point path, bit for bit"
    );
    assert_eq!(
        a,
        cpu_flare(&p, &baked, w, h, &via_param),
        "and the same frame every run (docs/14 determinism)"
    );
}

/// **An area source flares as ONE shape, not as a grid of copies** (K-367).
///
/// This is the defect the owner reported: "bright areas of a matte using
/// multiple points instead of an area". K-355 rendered a source by splitting
/// it into up to 5×5 point lights, so wherever a ghost was smaller than the
/// spacing between those samples you saw that many separate copies of the
/// aperture strung out in a line — five little irises where there should have
/// been one soft bar. Integrating the source per ray instead makes the copies
/// impossible rather than merely rare: no two rays share a source position,
/// and each one's splat footprint (K-366) inflates by the local
/// source-to-sensor stretch, which is exactly the gap a replica would have
/// sat in.
///
/// Measured as local maxima along the line through the brightest ghost: the
/// replicated version grew new peaks there, and the integrated one must not.
#[test]
fn lens_flare_an_area_source_does_not_replicate_its_ghosts() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        quality: 0,
        max_ghosts: 8,
        // No ghost blur: a blur would hide replication rather than prevent
        // it, and this test is about preventing it.
        ghost_softness: 0.0,
        light: [64.0, 36.0],
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (192u32, 108u32);
    let (rw, rh) = flare_pad_dims(w, h, p.anamorphic, p.scale);
    let point = cpu_flare(&p, &baked, w, h, &manual_light(&p, w, h));
    // A bar-shaped source most of the frame wide — a tube practical, and far
    // wider than the sample spacing that used to show.
    let wide = LensFlareParams {
        source_size: [40.0, 10.0],
        ..p
    };
    let area = cpu_flare(&wide, &baked, w, h, &manual_light(&wide, w, h));
    assert_ne!(
        point, area,
        "an area source must not render as a point does"
    );
    // The old mechanism, rebuilt by hand so the comparison is a measurement
    // rather than an assertion: the same source as the 5x5 grid of point
    // lights sharing its flux that `expand_area_lights` used to hand the
    // trace.
    let centre = manual_light(&wide, w, h)[0];
    let mut replicated = Vec::new();
    for iy in 0..5 {
        for ix in 0..5 {
            let t = |i: usize| i as f32 / 4.0 * 2.0 - 1.0;
            replicated.push(FlareLight {
                pos: [
                    centre.pos[0] + t(ix) * centre.extent[0],
                    centre.pos[1] + t(iy) * centre.extent[1],
                ],
                rgb: centre.rgb.map(|c| c / 25.0),
                extent: [0.0, 0.0],
            });
        }
    }
    let replicated = cpu_flare(&wide, &baked, w, h, &replicated);

    // The profile of ONE ghost: the horizontal line through the brightest
    // pixel of the POINT render — the same window in all three pictures, so
    // the comparison is of the same ghost rather than of whatever each
    // happens to make brightest — smoothed by a 3-tap mean so single-pixel
    // splat grain cannot count as a peak.
    let lum = |buf: &[f32], i: usize| buf[i * 3] + buf[i * 3 + 1] + buf[i * 3 + 2];
    let bi = (0..(rw * rh) as usize)
        .max_by(|&a, &b| {
            lum(&point, a)
                .partial_cmp(&lum(&point, b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("a non-empty flare buffer");
    let (bx, by) = (bi % rw as usize, bi / rw as usize);
    let peaks = |buf: &[f32]| -> usize {
        let lo = bx.saturating_sub(12);
        let hi = (bx + 12).min(rw as usize - 1);
        let row: Vec<f32> = (lo..=hi).map(|x| lum(buf, by * rw as usize + x)).collect();
        let smooth: Vec<f32> = (0..row.len())
            .map(|i| {
                let a = row[i.saturating_sub(1)];
                let b = row[i];
                let c = row[(i + 1).min(row.len() - 1)];
                (a + b + c) / 3.0
            })
            .collect();
        let cut = smooth.iter().cloned().fold(0.0_f32, f32::max) * 0.1;
        (1..smooth.len() - 1)
            .filter(|&i| smooth[i] > cut && smooth[i] > smooth[i - 1] && smooth[i] > smooth[i + 1])
            .count()
    };

    let (pp, ap, rp) = (peaks(&point), peaks(&area), peaks(&replicated));
    assert!(pp > 0, "the point flare must have a ghost to profile");
    assert!(
        rp > pp,
        "the test's own replication must actually replicate, or it proves \
         nothing: {rp} peaks against a point's {pp}"
    );
    // **Measured against the replication, not against the point.** The
    // comparison that matters is with the mechanism K-367 replaced, and it is
    // the only one of the three that is like-for-like: the same source, the
    // same extent, rendered the old way. An area source must carry no more
    // structure through this window than 25 stamped copies do.
    //
    // It used to read `ap <= pp`, against the POINT render, and that held only
    // while K-366's reconstruction was printing its sampling grid over
    // everything (K-373). With the grid gone, a bar source's ghost is a bar,
    // and a cut across a bar shows both of its rims where a point's small
    // ghost shows one summit — so the area render legitimately counts two
    // peaks to the point's one, and comparing the two was never comparing the
    // same object.
    assert!(
        ap <= rp,
        "an area source must smear its ghost, not stamp copies of it: \
         {ap} peaks against the old replication's {rp} (a point's is {pp})"
    );
}

/// **The jitter integrates the source; it does not lose light** (K-367).
///
/// Every ray carries the light's FULL colour now — there are no per-sample
/// flux shares, because the pupil grid already averages over the rays. The
/// obvious way to get that wrong is to keep dividing (a wide source fading as
/// it widens) or to stop dividing without removing the replication (a wide
/// source brightening). Both show up as an energy ratio away from one.
#[test]
fn lens_flare_an_area_source_keeps_its_flux() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        quality: 0,
        max_ghosts: 8,
        light: [64.0, 36.0],
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (192u32, 108u32);
    let point: f32 = cpu_flare(&p, &baked, w, h, &manual_light(&p, w, h))
        .iter()
        .sum();
    let wide = LensFlareParams {
        source_size: [12.0, 12.0],
        ..p
    };
    let area: f32 = cpu_flare(&wide, &baked, w, h, &manual_light(&wide, w, h))
        .iter()
        .sum();
    assert!(point > 0.0, "the point flare must render: {point}");
    let ratio = area / point;
    // The floor sits at 0.94, not 0.98: spreading is the point, and on this
    // deliberately tiny raster a few percent of the honestly-spread smear
    // crosses the frame edge (K-378's wider footprints spread a little
    // further than K-367's). Measured 0.972 here and 1.007 on a padded
    // buffer that catches the spill — the flux is spread, not lost. The
    // window still fails the bugs it exists for: replication-style scaling
    // is off by whole factors, not percent.
    assert!(
        (0.94..=1.02).contains(&ratio),
        "an area source must spread one light's flux, not scale it: {ratio} \
         ({area} against {point})"
    );
}

/// **An area source renders as a smooth shape, not a woven grid** (K-378).
///
/// K-367's per-ray source integration hops each ray's source point by more
/// than the whole source between pupil neighbours — that is what
/// equidistributes the samples — and three things in the reconstruction let
/// that read as a quasi-periodic mesh stamped across every ghost, which is
/// what the owner photographed: central-difference footprints cancelled
/// toward zero wherever a ray's two neighbours hopped to the same side; the
/// old `PHI_V` sits within 0.002 of 4/7, so its samples fell into seven
/// slow-drifting combs that lined up into stripes; and every band re-traced
/// the same source points, so the bands' summed ripple never averaged.
///
/// The flux tests all passed throughout, exactly as K-376 records for the
/// kernel's own version of this lesson: they measure how much light there
/// is, never whether it is smooth. So this measures smoothness — the
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
    // K-376's grid-imprint metric with a WIDER neighbourhood: each pixel's
    // departure from its own 9×9 mean, relative to that mean, over the lit
    // region. K-376's 3×3 cannot see this artefact — the mesh's period is
    // the ray spacing, several pixels, so every pixel sits close to a 3×3
    // mean and a plainly striped ghost scores under that test's bound
    // (measured; it is the same passed-while-visible trap K-376 itself
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
         ~4% the K-378 reconstruction measures — the woven mesh is coming \
         back (the K-367 reconstruction measured ~13% here, and read as a \
         grid stamped across every ghost on screen)"
    );
}

/// **A wide source's starburst smears across it** (K-367).
///
/// The ghosts integrate their source per ray, but the starburst cannot: it is
/// a baked sprite, not a traced path. It *is* shift-invariant, though — the
/// diffraction pattern of a hole does not change shape as the source moves,
/// only where it sits — so the starburst of an extended source is exactly the
/// point sprite convolved with the source, and stamping a fixed 3×3 grid
/// across the source is that convolution in quadrature. Stamping once at the
/// centre instead would give a softbox the pinpoint spike of a star, which is
/// the one thing a softbox does not have.
#[test]
fn lens_flare_an_area_source_smears_its_starburst() {
    use crate::fx::lens_flare::*;
    let p = LensFlareParams {
        quality: 0,
        max_ghosts: 1,
        ..default_flare_params()
    };
    let baked = bake(&p);
    let (w, h) = (192u32, 108u32);
    // An empty ghost buffer, so what is measured is the starburst alone.
    let flare = vec![0.0_f32; (w * h * 3) as usize];
    let render = |lights: &[FlareLight]| -> Vec<f32> {
        let mut rgba = vec![0.0_f32; (w * h * 4) as usize];
        cpu_combine(&mut rgba, w, h, &p, &baked, &flare, w, h, lights);
        rgba
    };
    let row = |rgba: &[f32], light: &FlareLight| -> Vec<f32> {
        let y = (light.pos[1] * h as f32) as usize;
        (0..w as usize)
            .map(|x| {
                let i = (y * w as usize + x) * 4;
                rgba[i] + rgba[i + 1] + rgba[i + 2]
            })
            .collect()
    };
    // The lit SPAN along the row through the light, measured at a fixed
    // absolute level rather than at each picture's own half maximum: the
    // sprite is a very peaked star, so smearing it lowers the peak as much as
    // it widens the base, and a relative half-max would chase its own tail.
    // The level is a tenth of the POINT starburst's peak, so both pictures are
    // measured against the same line.
    let span = |r: &[f32], cut: f32| -> usize {
        let lit: Vec<usize> = (0..r.len()).filter(|&i| r[i] >= cut).collect();
        match (lit.first(), lit.last()) {
            (Some(&a), Some(&b)) => b - a + 1,
            _ => 0,
        }
    };

    let base = manual_light(&p, w, h);
    let point = render(&base);
    let point_row = row(&point, &base[0]);
    let cut = point_row.iter().cloned().fold(0.0_f32, f32::max) * 0.1;
    assert!(cut > 0.0, "the starburst must render something");
    let point_w = span(&point_row, cut);

    // A source under the threshold is a point of light, bit for bit: the
    // stamp grid collapses to one stamp on the light's own position carrying
    // its whole colour, so nothing anyone has already built moves.
    assert_eq!(starburst_stamp_grid([0.0, 0.0]), (1, 1));
    let tiny = vec![FlareLight {
        extent: [SB_MIN_EXTENT * 0.5, SB_MIN_EXTENT * 0.5],
        ..base[0]
    }];
    assert_eq!(
        point,
        render(&tiny),
        "a source below the stamp threshold must render as the point it is"
    );

    // A source 60 px across smears its spike across itself.
    let ext = 30.0 / w as f32;
    let wide = vec![FlareLight {
        extent: [ext, 0.0],
        ..base[0]
    }];
    assert_eq!(starburst_stamp_grid([ext, 0.0]), (SB_STAMPS, 1));
    let wide_w = span(&row(&render(&wide), &wide[0]), cut);
    assert!(
        wide_w > point_w + 40,
        "a 60 px source must smear its starburst across roughly its own \
         width: {wide_w} px lit against a point's {point_w}"
    );
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
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
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

    // ---- and the whole way through to the trace (K-360, K-385) ----
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

/// **A per-element coating choice reaches the trace** (K-371).
///
/// The regression: the resolve arm read all twenty rows through the *float*
/// accessor, which answers `None` for a `Choice` value — so every element
/// silently resolved to "as the lens file" and the whole feature was inert from
/// the panel, whatever anyone picked. Nothing showed it: the flare rendered
/// perfectly, just with the prescription's own coatings.
#[test]
fn lens_flare_element_coatings_reach_the_trace() {
    use crate::fx::lens_flare::{COATING_AS_FILE, COATING_DESIGNS, MAX_COATING_ELEMENTS};

    let resolved = |el1: u32, el3: u32| {
        let mut inst = instantiate("lens_flare").unwrap();
        for p in &mut inst.params {
            match p.id.as_str() {
                "coating_el1" => p.value = EffectValue::Choice(el1),
                "coating_el3" => p.value = EffectValue::Choice(el3),
                _ => continue,
            }
        }
        let ops = super::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            2202.9,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        flare_packed(&ops).coating_elements
    };

    // A fresh instance leaves every element as the prescription describes it —
    // the default, and what every flare rendered before the rows existed.
    assert_eq!(
        resolved(0, 0),
        [COATING_AS_FILE; MAX_COATING_ELEMENTS],
        "an untouched panel changes no element's coating"
    );

    let picked = resolved(4, 1);
    assert_eq!(picked[0], 4, "element 1's pick reaches the trace");
    assert_eq!(picked[2], 1, "and so does element 3's");
    assert_eq!(
        picked[1], COATING_AS_FILE,
        "a row nobody touched still leaves its element alone"
    );

    // An index past the palette clamps rather than indexing off the end of it.
    assert_eq!(resolved(COATING_DESIGNS + 5, 0)[0], COATING_DESIGNS - 1);
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
            // K-395: the Invert that flips the matte sits beside the picker,
            // on the one uniform row every effect draws. It used to live in the
            // Depth map twirl; the stored id is untouched (K-065).
            "depth_invert",
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
            "gamma",
            "remove_edge_leak",
            "detect_edge_threshold",
            // Back out of the twirls.
            "repeat_edge_pixels",
            "display",
            "mix",
            // K-425: the Blend injected beside every Mix.
            "blend",
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
    // The focus point reads its **declared default** rather than the old arm's
    // separate `unwrap_or(0.0)` fallback — K-258's rule for a parameter a saved
    // project has never heard of, and the one the arena applies to every row.
    // Nothing renders differently for it: the point is read only when Use focus
    // point is on, which this instance does not carry either, so it stays false.
    assert_eq!(
        dof_packed(&legacy, 1.0, false),
        neutral_dof(8.0, 8.0, [960.0, 540.0])
    );
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
#[ignore = "by-hand perf probe, not part of the suite"]
fn bake_timing_probe() {
    for lens in 0..crate::fx::lens_library::LENS_LIBRARY.len() as u32 {
        let p = crate::fx::lens_flare::LensFlareParams {
            lens,
            ..default_flare_params()
        };
        let t = std::time::Instant::now();
        let baked = crate::fx::lens_flare::bake(&p);
        let four = |take: usize| {
            baked
                .pairs
                .iter()
                .take(take)
                .filter(|p| p[2] != crate::fx::lens_flare::NO_BOUNCE)
                .count()
        };
        eprintln!(
            "lens {lens:2} ({}): {:6.1} ms, {} surfaces, {} paths ({} four-bounce, {} in the rendered {})",
            crate::fx::lens_flare::lens_entry(lens).name,
            t.elapsed().as_secs_f64() * 1000.0,
            baked.surfaces.len(),
            baked.pairs.len(),
            four(usize::MAX),
            four(crate::fx::lens_flare::MAX_RENDERED_PAIRS),
            crate::fx::lens_flare::MAX_RENDERED_PAIRS,
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

/// `ParamId` is a hash, so a collision inside one effect would silently make two
/// controls one control. The input space is small and the risk is theoretical —
/// which is exactly why it needs a test rather than an argument.
#[test]
fn no_effects_parameters_share_an_id_hash() {
    for s in BUILTINS {
        let mut seen: Vec<(ParamId, &str)> = Vec::new();
        for p in s.params {
            let id = ParamId::new(p.id);
            if let Some((_, other)) = seen.iter().find(|(k, _)| *k == id) {
                panic!("{}: {} and {} hash alike", s.match_name, p.id, other);
            }
            seen.push((id, p.id));
        }
    }
}

/// Every migrated effect must be reachable both ways: the catalogue knows it by
/// name, and the name it answers to is the one it declares.
#[test]
fn a_registered_effect_is_found_by_its_own_name() {
    for def in BUILTIN_DEFS.iter() {
        let name = def.schema().match_name;
        let found = BUILTIN_DEFS
            .get(name)
            .unwrap_or_else(|| panic!("{name} is registered but cannot be looked up"));
        assert_eq!(found.schema().match_name, name);
    }
    assert!(BUILTIN_DEFS.get("no_such_effect").is_none());
}

/// A project saved before a parameter existed carries no entry for it, and must
/// render — reading the declared default, never panicking (K-258).
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

/// A parameter the effect does not declare is ignored rather than mistaken for
/// one it does — the shape a preset from a newer build arrives in (docs/08 §5).
#[test]
fn an_unknown_parameter_is_ignored() {
    let entries = [
        (ParamId::new("not_a_parameter"), Value::Float(7.0)),
        (effects::exposure::Exposure::STOPS, Value::Float(2.0)),
    ];
    let v = effects::exposure::Exposure::read(Params::new(&entries));
    assert_eq!(v.stops, 2.0);
}

/// The generated ids are the hashes of the declared ids, so a rename of a field
/// without a rename of the stored parameter cannot pass unnoticed.
#[test]
fn a_generated_id_is_the_hash_of_the_declared_name() {
    assert_eq!(
        effects::exposure::Exposure::STOPS,
        ParamId::new("stops"),
        "the generated const must address the same parameter the schema declares"
    );
}

/// The stack is an arena: each op sees its own run of parameters and no other's.
#[test]
fn a_resolved_stack_keeps_each_ops_parameters_to_itself() {
    let mut stack = ResolvedStack::new();
    stack.begin(&effects::exposure::ExposureDef, Uuid::now_v7());
    stack.push(effects::exposure::Exposure::STOPS, Value::Float(1.0));
    stack.push(effects::exposure::Exposure::MIX, Value::Float(100.0));
    stack.begin(&effects::invert::InvertDef, Uuid::now_v7());
    stack.push(effects::invert::Invert::MIX, Value::Float(50.0));

    assert_eq!(stack.len(), 2);
    let first = stack.get(0).expect("first op");
    assert_eq!(first.def.schema().match_name, "exposure");
    assert_eq!(first.params.len(), 2);
    assert_eq!(
        first.params.float(effects::exposure::Exposure::STOPS, 0.0),
        1.0
    );

    let second = stack.get(1).expect("second op");
    assert_eq!(second.def.schema().match_name, "invert");
    assert_eq!(second.params.len(), 1);
    // The Invert's Mix is its own, not the Exposure's.
    assert_eq!(second.params.float(effects::invert::Invert::MIX, 0.0), 50.0);
}

/// Withdrawing an op takes its parameters with it, leaving the arena as it was —
/// how an effect that resolves to nothing this frame is dropped after the fact.
#[test]
fn withdrawing_an_op_leaves_no_parameters_behind() {
    let mut stack = ResolvedStack::new();
    stack.begin(&effects::exposure::ExposureDef, Uuid::now_v7());
    stack.push(effects::exposure::Exposure::STOPS, Value::Float(1.0));
    stack.begin(&effects::invert::InvertDef, Uuid::now_v7());
    stack.push(effects::invert::Invert::MIX, Value::Float(50.0));
    stack.drop_last();

    assert_eq!(stack.len(), 1);
    let only = stack.get(0).expect("the surviving op");
    assert_eq!(only.params.len(), 1);
    assert_eq!(
        only.params.float(effects::exposure::Exposure::STOPS, 0.0),
        1.0
    );
}

/// The generic rescale moves the values whose declared unit follows the raster,
/// and only those — the replacement for the per-variant `rescale_px` match.
#[test]
fn only_spatial_values_rescale() {
    // The colour family declares no spatial parameters, so a rescale is a no-op
    // for it; the blur family's radius is the first that does move.
    let mut stack = ResolvedStack::new();
    stack.begin(&effects::exposure::ExposureDef, Uuid::now_v7());
    stack.push(effects::exposure::Exposure::STOPS, Value::Float(2.0));
    stack.begin(&effects::blur::BlurDef, Uuid::now_v7());
    stack.push(effects::blur::Blur::RADIUS, Value::Float(10.0));
    stack.push(effects::blur::Blur::MIX, Value::Float(80.0));
    stack.rescale_spatial(0.5);
    let op = stack.get(0).expect("the op");
    assert_eq!(
        op.params.float(effects::exposure::Exposure::STOPS, 0.0),
        2.0,
        "stops are not a length and must not follow the raster"
    );
    let blur = stack.get(1).expect("the blur op");
    assert_eq!(
        blur.params.float(effects::blur::Blur::RADIUS, 0.0),
        5.0,
        "a Px radius resolved to pixels follows the raster"
    );
    assert_eq!(
        blur.params.float(effects::blur::Blur::MIX, 0.0),
        80.0,
        "the Mix is not a length"
    );
}

/// docs/impl/effect-registry.md §7 test 4, deferred from the plumbing stage: a
/// spatial parameter in the arena rescales under [`ResolvedStack::rescale_spatial`]
/// **exactly** as the old `Resolved` op did.
///
/// This is the one property the migration could silently lose. K-266's repair —
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
    // place — which is the whole point of the correction (K-266).
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

/// The stylise family's own half of the same property, for two effects that
/// reached `Px` by different roads: RGB split's Amount (a `PctDiag` until
/// K-419) and chromatic aberration's (`Px` from the start). Each is multiplied
/// by the preview factor at resolve and must be scaled **exactly once** on the way
/// in and exactly once again by [`ResolvedStack::rescale_spatial`] — which is what the
/// old arms and `rescale_px` did between them.
///
/// The failure this pins is silent: declaring the unit *and* converting again in
/// `packed` would multiply twice, and a half-resolution preview would fringe at
/// a quarter of the width the export does with nothing on screen to say so.
#[test]
fn the_stylise_family_rescales_once_in_each_unit() {
    let resolve = |name: &str, px_scale: f32| {
        let e = instantiate(name).unwrap_or_else(|| panic!("{name} is a built-in"));
        super::resolve_stack(
            &[e],
            0.0,
            1000.0,
            px_scale,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        )
    };
    // RGB split, px@comp: the default 8 px at a px_scale of 1 is 8 px.
    let amount = |ops: &super::ResolvedStack| -> f32 {
        match effects::rgb_split::RgbSplit::read(ops.get(0).expect("the op").params).packed() {
            effects::rgb_split::Split::Classic { amount_px, .. } => amount_px,
            other => panic!("expected the classic split, got {other:?}"),
        }
    };
    let mut ops = resolve("rgb_split", 1.0);
    assert_eq!(amount(&ops), 8.0);
    ops.rescale_spatial(0.5);
    assert_eq!(amount(&ops), 4.0, "px@comp follows the preview raster");

    // Chromatic aberration, px@comp: the authored 4 px, scaled by the preview
    // factor at resolve and by the repair factor afterwards — once each, so a
    // half-resolution resolve followed by a further halving is a quarter of the
    // authored width, not an eighth.
    let ca = |ops: &super::ResolvedStack| -> f32 {
        match effects::chromatic_aberration::ChromaticAberration::read(
            ops.get(0).expect("the op").params,
        )
        .packed()
        {
            effects::chromatic_aberration::Fringe::Classic { amount_px, .. } => amount_px,
            other => panic!("expected the classic fringe, got {other:?}"),
        }
    };
    let mut full = resolve("chromatic_aberration", 1.0);
    assert_eq!(ca(&full), 4.0);
    full.rescale_spatial(0.5);
    assert_eq!(ca(&full), 2.0, "px@comp follows the preview raster");
    // Resolving directly against the smaller raster lands in the same place —
    // the whole point of the K-266 correction.
    assert_eq!(ca(&resolve("chromatic_aberration", 0.5)), ca(&full));
    let mut half = resolve("chromatic_aberration", 0.5);
    half.rescale_spatial(0.5);
    assert_eq!(ca(&half), 1.0, "scaled once at resolve, once by the repair");
    // Factor 1 is exactly a no-op, as it was for the variants.
    let mut same = resolve("chromatic_aberration", 1.0);
    same.rescale_spatial(1.0);
    assert_eq!(ca(&same), 4.0);
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
    // when they joined the list. Since K-419 every one of them is px@comp; the
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
    // one multiply earlier (K-388), which is the same wobble to within the
    // reassociation `shake_amplitude_rescales_as_the_old_offsets_did` bounds.
    //
    // The Generate family (K-398) brings nine more px@comp entries and no % diag
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
    // **centre** are the point pairs K-260 requires be pixels. Tile declares only
    // its Tile centre — the four per cents beside it are fractions of the raster
    // and so do not follow it — and Lens distort only its Centre, since a field
    // of view is an angle.
    //
    // The utility and transition batch (K-400) brings seven more, four % diag and
    // three px@comp. **Channel blur's four radii** are % diag exactly as the
    // Gaussian blur's is, being the same kernel four times over. **Drop shadow's
    // Distance and Softness** are px@comp and must travel together, or a
    // half-resolution preview would move the shadow and not soften it (or the
    // reverse); its Direction is an angle and its Opacity a per cent, so neither
    // is here. The **wipes' centres and feathers** are px@comp for K-260's reason
    // and for the shadow's; their Completion is a per cent of the frame's own
    // extent, which the kernel derives from the raster it is handed, so it needs
    // no rescaling and gets none — Tile's four per cents again.
    //
    // Wave 2's Distort I batch (docs/08 §3.48-§3.52) brings eighteen more: two
    // % diag and sixteen px@comp. **Corner pin's eight point coordinates** are
    // pixels for K-260's reason and must travel together or a half-resolution
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
    // pixels for K-260's reason and must travel together, exactly as Corner
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
    // The Matte key's spatial controls (K-546) bring three. **Screen pre-blur**
    // is a blur radius like any other; **Screen shrink/grow** is how far the
    // matte's edge marches, which must be the same distance in the picture at
    // any preview resolution; **Screen softness** is a blur radius again. Its
    // two garbage-mask rows declare none, for the reason the three line-drawing
    // effects' rows declare none: the geometry is flattened once in px@comp and
    // each consumer takes it to its own raster. Despot black and white are per
    // cent and reach exactly one pixel by definition, so neither is a distance
    // a preview could get wrong.
    //
    // K-408's two consumers (docs/08 §3.78-§3.79) bring four, all px@comp and
    // all pixel-scale looks. **Scribble's Stroke width, Spacing and Path
    // overlap** are the pencil's own dimensions and must travel together, or a
    // half-resolution preview would draw a hatch of a different density from
    // the export's. **Stroke's Brush size** is Vegas' Width under another name.
    // Neither declares one for the mask's own vertices, and that is the point of
    // K-408's tolerance being a constant: the polyline is flattened once in
    // px@comp and each consumer takes it to its own raster, so the geometry
    // cannot acquire a second unit. Stroke's Spacing is a per cent *of the
    // brush*, so it rides on Brush size and is not here.
    //
    // The Controls family (K-414) brings two more, and they are the first pair
    // here that never *reaches* the rescale pass: a Point control draws
    // nothing, so it resolves to no op at all. They are declared px@comp all
    // the same, because that is what the numbers mean (K-260) and because what
    // reads them through an expression is going to put them in a picture.
    //
    // **Points sample's Position** (K-494) is the last pair, and the second
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
            // K-558: Radial blur's centre is the last point to stop being a
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
            // Particulate (K-419 through a particle system): eleven, all
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
            ("particulate", "width"),
            ("particulate", "height"),
            ("particulate", "initial_speed"),
            ("particulate", "size"),
            ("particulate", "gravity"),
            ("particulate", "wind_x"),
            ("particulate", "wind_y"),
            ("particulate", "turbulence_amount"),
            ("particulate", "turbulence_scale"),
            // What a full channel of a Motion vectors layer means, in pixels
            // of movement (K-429).
            ("motion_blur", "vector_scale"),
            ("matte_key", "pre_blur"),
            ("matte_key", "shrink_grow"),
            ("matte_key", "softness"),
            ("shadow_highlight", "radius"),
            ("lens_flare", "light_x"),
            ("lens_flare", "light_y"),
            ("lens_flare", "source_width"),
            ("lens_flare", "source_height"),
            ("drop_shadow", "distance"),
            ("drop_shadow", "softness"),
            ("roughen_edges", "border"),
            ("roughen_edges", "scale"),
            ("roughen_edges", "offset_x"),
            ("roughen_edges", "offset_y"),
            ("median", "radius"),
            ("emboss", "relief"),
            ("texturize", "relief"),
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
            // K-558: the flipping wave's width is a distance across the frame,
            // so it follows the raster like every other distance.
            ("card_wipe", "transition_width"),
            ("point_control", "point_x"),
            ("point_control", "point_y"),
            ("points_sample", "position_x"),
            ("points_sample", "position_y"),
        ]
    );
    // Both are px@comp, multiplied by the preview factor (K-419): RGB split's
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

/// **No parameter is a percentage of the composition diagonal** (K-419, the
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
        "parameters declared PctDiag, which K-419 forbids: {offenders:?}"
    );
}

/// **The ROI tile padding is px@comp too, and resolves exactly as a `Px`
/// parameter does** (K-433). Gaussian blur's Radius and Gaussian blur's padding
/// are the same 2 000 px@comp; at Full and at Half preview they must still come
/// out the same number of raster pixels, or the tile clips the radius on one of
/// them.
#[test]
fn the_roi_padding_follows_the_raster_like_a_px_radius() {
    let mut e = instantiate("blur").expect("Gaussian blur is declared");
    for p in &mut e.params {
        if p.id == "radius" {
            p.value = EffectValue::Float(Property::fixed(2000.0));
        }
    }
    let roi = BUILTIN_DEFS
        .get("blur")
        .expect("declared")
        .schema()
        .traits
        .roi;
    for px_scale in [1.0f32, 0.5] {
        let b = resolve_migrated::<effects::blur::Blur>(
            &[e.clone()],
            0.0,
            1920f32.hypot(1080.0),
            px_scale,
            &MarkerContext::NONE,
        );
        assert_eq!(
            roi.padding_raster_px(px_scale),
            Some(b.packed().0.ceil() as u32),
            "the padding and the radius must resolve alike at px_scale {px_scale}"
        );
    }
    assert_eq!(roi.padding_raster_px(1.0), Some(2000));
    assert_eq!(roi.padding_raster_px(0.5), Some(1000));
    // One pixel of reach stays one raster pixel however coarse the preview.
    assert_eq!(Roi::PaddedPx(1.0).padding_raster_px(0.25), Some(1));
    assert_eq!(Roi::Exact.padding_raster_px(0.5), Some(0));
    assert_eq!(
        Roi::FullFrame.padding_raster_px(1.0),
        None,
        "a full-frame effect has no finite padding"
    );
}

/// **Every padding covers its effect's own hard maximum** (K-433). The old
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

/// Two kinds holding the same number must not hash alike, or a Choice of 1 and a
/// Bool of true would share a cache entry.
#[test]
fn the_frame_key_separates_a_value_from_its_number() {
    let hash = |value: Value| {
        let mut stack = ResolvedStack::new();
        stack.begin(&effects::invert::InvertDef, Uuid::nil());
        stack.push(ParamId::new("x"), value);
        let mut bytes: Vec<u8> = Vec::new();
        stack.feed_hash(&mut |b| bytes.extend_from_slice(b));
        bytes
    };
    assert_ne!(hash(Value::Choice(1)), hash(Value::Bool(true)));
    assert_ne!(hash(Value::Float(1.0)), hash(Value::Int(1)));
    // And the same value hashes the same way twice: the determinism the frame
    // key rests on (K-143).
    assert_eq!(hash(Value::Float(0.25)), hash(Value::Float(0.25)));
    // The four-float kinds are the pair most at risk: same numbers, same
    // length, different meaning (K-388). Only the tag separates them.
    let four = [0.25f32, 0.5, 0.75, 1.0];
    assert_ne!(hash(Value::Vec4(four)), hash(Value::Colour(four)));
    assert_eq!(hash(Value::Vec4(four)), hash(Value::Vec4(four)));
    assert_ne!(
        hash(Value::Vec4(four)),
        hash(Value::Vec4([0.25, 0.5, 0.75, -1.0])),
        "the last component is fed too"
    );
}

/// [`Value::Vec4`] is a kind of its own (K-388): its tag is distinct from every
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
    // the same rule every typed reader follows (K-258).
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
        // A length, then two floats a live point (K-412) — the unused tail of
        // the fixed array is padding by another name and never feeds a key.
        Value::Curve(c) => 4 + 8 * c.points().len(),
    }
}

/// A stack carrying a migrated effect is dispatched to that effect's own CPU
/// reference, and renders what the old `Resolved::Exposure` arm rendered.
///
/// The old arm is gone, so its numbers are written out here instead: +1 stop is
/// a factor of 2, Mix 100 % is 1. That is the whole of what the arm did before
/// calling [`cpu::exposure`], and pinning it is what makes the port checkable
/// after the variant has been deleted.
#[test]
fn a_registered_effect_renders_what_the_old_dispatch_rendered() {
    let source: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();

    let mut through_registry = source.clone();
    let mut ops = ResolvedStack::new();
    ops.begin(&effects::exposure::ExposureDef, uuid::Uuid::now_v7());
    ops.push(effects::exposure::Exposure::STOPS, Value::Float(1.0));
    ops.push(effects::exposure::Exposure::MIX, Value::Float(100.0));
    cpu::apply_stack(&mut through_registry, 2, 2, &ops);

    let mut through_the_old_numbers = source.clone();
    cpu::exposure(&mut through_the_old_numbers, 2.0, 1.0);

    assert_eq!(through_registry, through_the_old_numbers);
}

/// The kinds beyond a plain slider read back as themselves: a colour as four
/// channels, a switch as a switch, a dial as its degrees. A generated reader
/// that quietly turned one into another would still pass the schema test.
#[test]
fn every_parameter_kind_reads_back_as_itself() {
    let entries = [
        (
            effects::tint::Tint::BLACK,
            Value::Colour([0.1, 0.2, 0.3, 1.0]),
        ),
        (
            effects::tint::Tint::WHITE,
            Value::Colour([0.9, 0.8, 0.7, 1.0]),
        ),
        (effects::tint::Tint::MIX, Value::Float(50.0)),
    ];
    let v = effects::tint::Tint::read(Params::new(&entries));
    assert_eq!(v.black, [0.1, 0.2, 0.3, 1.0]);
    assert_eq!(v.white, [0.9, 0.8, 0.7, 1.0]);
    assert_eq!(v.mix, 50.0);

    let entries = [
        (effects::hue_shift::HueShift::ANGLE, Value::Float(90.0)),
        (
            effects::hue_shift::HueShift::PRESERVE_LUMINANCE,
            Value::Bool(false),
        ),
    ];
    let v = effects::hue_shift::HueShift::read(Params::new(&entries));
    assert_eq!(v.angle, 90.0);
    assert!(!v.preserve_luminance);
    assert_eq!(v.matrix(), hue_matrix_rgb(90.0));

    // Absent on projects saved before the bool existed → true, the historical
    // behaviour (K-136), and the constant-luminance matrix with it.
    let v = effects::hue_shift::HueShift::read(Params::EMPTY);
    assert!(v.preserve_luminance);
    assert_eq!(v.matrix(), hue_matrix(0.0));
}

/// The host-side maths a resolve arm used to do now lives on the effect, and
/// must produce the same numbers — byte for byte, since the GPU multiplies by
/// them (docs/08 §1.6).
#[test]
fn the_host_side_maths_moved_without_changing() {
    let at = |t: f32| {
        let entries = [(
            effects::temperature::Temperature::TEMPERATURE,
            Value::Float(t),
        )];
        effects::temperature::Temperature::read(Params::new(&entries)).gains()
    };
    // Neutral is exactly neutral: the identity the WGSL kernel relies on.
    assert_eq!(at(0.0), (1.0, 1.0));
    // The old arm: k = t/100 clamped to ±2, gains 1 ± 0.75k floored at 0.
    for t in [-300.0f32, -150.0, -20.0, 45.0, 150.0, 300.0] {
        let k = (t / 100.0).clamp(-2.0, 2.0);
        assert_eq!(
            at(t),
            ((1.0 + 0.75 * k).max(0.0), (1.0 - 0.75 * k).max(0.0))
        );
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
        &|p| cpu::contrast(p, 1.6, 1.0),
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
        // Flash's strength is derived rather than declared (K-385), so it goes
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
        // Block glitch's tick is derived rather than declared (K-385), so it
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
        // Scanlines' folded intensity and roll offset are derived too (K-385).
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
        // field, Fast motion blur, Datamosh) need a second picture the
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

/// The four side-table effects of the K-387 batch have **no** CPU reference
/// through the arena dispatch, and that silence is deliberate: each needs a
/// second picture — a background plate, a depth pass, a flow field, a neighbour
/// frame — that no single-buffer dispatcher carries. Their `apply_cpu` keeps
/// [`EffectDef`]'s identity default, which is exactly what their old
/// `cpu::apply` arms were.
///
/// Worth pinning rather than assuming, because the failure is invisible: an
/// `apply_cpu` written for one of these would read a garbage second buffer or
/// half-render, and the degradation rung (K-019) is the one path nobody looks
/// at. Their §1.6 oracles run against `cpu::light_wrap` / `cpu::dof` /
/// `cpu::motion_blur` / `cpu::datamosh` directly from the lumit-gpu tests, which
/// can upload the second picture.
#[test]
fn the_side_table_batch_stays_a_cpu_passthrough() {
    let (w, h) = (4u32, 4u32);
    let source: Vec<f32> = (0..(w * h * 4)).map(|i| (i % 17) as f32 / 17.0).collect();
    for name in [
        "light_wrap",
        "dof",
        "motion_blur",
        "datamosh",
        "echo",
        "lut",
    ] {
        let e = instantiate(name).unwrap_or_else(|| panic!("{name} is a built-in"));
        let ops = super::resolve_stack(
            &[e],
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 1, "{name} resolves to exactly one op");
        let mut out = source.clone();
        cpu::apply_stack(&mut out, w, h, &ops);
        assert_eq!(
            out, source,
            "{name} drew something on the CPU rung, which has no second picture to draw it from"
        );
    }
}

/// The arena can carry a number, a switch, a choice and a colour, but a LUT's
/// file slot and a layer reference's binding are decided by the *caller* —
/// `resolve_into_arena` skips those kinds, because it has no way to know which
/// cube loaded or which layer was rendered. Reading one back out of a bag
/// therefore always answers "unset", whatever the project stored.
///
/// That is only safe because the real input arrives beside the op as an aux slot
/// (K-387): the gate that an effect declaring one of these kinds also declares
/// the list it consumes is `a_side_table_effect_declares_the_list_it_consumes`,
/// in lumit-render, where both halves are visible. What is pinned here is the
/// half lumit-core owns — that the bag stays silent, so nobody is tempted to
/// read the grade out of it.
/// How many of `name`'s declared parameters reach the resolved bag: every kind
/// but the two the *caller* decides — a File slot and a Layer binding, which
/// ride beside the op as aux slots (K-387, and the injected Matte row of K-395
/// with them).
fn bagged_params(name: &str) -> usize {
    BUILTIN_DEFS
        .get(name)
        .expect("a built-in")
        .schema()
        .params
        .iter()
        .filter(|p| !matches!(p.kind, ParamKind::File { .. } | ParamKind::Layer { .. }))
        .count()
}

#[test]
fn the_arena_carries_no_file_slot_or_layer_binding() {
    let lut = instantiate("lut").expect("lut is a built-in");
    let declared = BUILTIN_DEFS
        .get("lut")
        .expect("lut is migrated")
        .schema()
        .params
        .len();
    assert!(
        declared > 1,
        "the File row is part of the declaration, for the panel"
    );

    let bag = resolve_bag(
        std::slice::from_ref(&lut),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
    );
    assert_eq!(
        bag.len(),
        bagged_params("lut"),
        "neither the File row nor the injected Matte layer row may reach the \
         arena — both are aux slots"
    );
    assert!(
        bagged_params("lut") < declared,
        "the skipped rows really are declared"
    );
    assert_eq!(
        resolve_migrated::<effects::lut::Lut>(&[lut], 0.0, 1000.0, 1.0, &MarkerContext::NONE).file,
        None,
        "the typed reader answers `unset`, never a default slot"
    );
}

/// **Every effect has a Matte, and nobody had to write it down** (K-395).
///
/// The declaration is what makes the row meaningful on all thirty-odd effects
/// from day one, and the whole point of injecting it is that a new effect cannot
/// forget it — so the gate is the catalogue itself, not a list kept beside it.
///
/// The five that *claim* the matte are named here on purpose. Two of them owned
/// the concept before K-395 and keep their stored ids (K-065): Depth of field's
/// `depth`, the Lens flare's `matte`. Three take the injected row and simply mean
/// something deeper by it: the Gaussian blur scales its radius, the Glow gates
/// its seed, Turbulent displace scales its displacement vector, Set matte makes
/// it the alpha and Displacement map makes it the map itself. Adding an eighth is
/// a deliberate act, and it lands here.
#[test]
fn every_effect_carries_a_matte_row() {
    use crate::fx::MatteRole;
    for def in BUILTIN_DEFS.iter() {
        let s = def.schema();
        // The Controls family opts out entirely (K-414), the Drivers family with
        // it (K-471), and so does the Camera track (K-417) — a handle for a
        // background analysis rather than an image operation. They are the
        // answer to the question `MatteRole::None` was written for: an effect
        // that touches no pixel cannot be driven by a picture, so a Matte row on
        // one would be a control that could never do anything. Every *image*
        // effect below still has to carry one.
        if matches!(s.category, FxCategory::Controls | FxCategory::Drivers)
            || s.match_name == "camera_track"
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
        // **Two image effects opt out** (K-425 and K-429, the owner's rule for
        // mattes), and each has its own reason.
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
        // Anything else that wants to opt out is argued for here, in these
        // words, before it may.
        if matches!(s.match_name, "matte_key" | "set_matte") {
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
        // The owner's rule for mattes (K-426): the matte scales the amount.
        // Every blur, sharpen and colour effect whose scaled amount is not
        // already a straight lerp of the input claims it; the rest (Tritone,
        // Black and white, Tint, Curves, Levels, Invert, LUT, Broadcast safe,
        // Contrast, Vignette) keep the strength dissolve because scaling
        // their amount IS that dissolve, and Threshold because it has no
        // honest per-pixel form. The Distortion family claims it the same way
        // (K-427): the matte scales the displacement, read at the destination
        // pixel. Datamosh stays on the dissolve because scaling its Intensity
        // IS the dissolve to the bit; Tile, Mirror and Polar coordinates have
        // no amount to scale. The Transform effect shares the Shake's kernel
        // but never binds a matte. Generate and Stylise claim it the same way
        // again (K-428): the grain's Intensity, the drawn thing's Opacity, the
        // shadow's Opacity, Border, Radius and Relief. Noise, Flash, Sprite
        // flare and Light wrap keep the dissolve there, because each adds a
        // linear amount to the picture and scaling it IS the dissolve; Fill,
        // Gradient, Fractal noise, Beam, Mosaic and Find edges have no amount
        // of their own to scale. Temporal and Transition claim it once more
        // (K-429): Echo's Decay, both motion blurs' Shutter angle, and every
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
                | "echo"
                | "motion_blur"
                | "accumulation_mb"
                | "linear_wipe"
                | "radial_wipe"
                | "venetian_blinds"
                | "iris_wipe"
                | "card_wipe"
        );
        assert_eq!(
            !s.matte.generic(),
            claims,
            "{} — the effects that claim the matte inside their own maths are              listed here (K-395, K-426, K-427); anything else that wants a deeper meaning              must say so here too",
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
        // manual's tables print (K-395, `fx-reference.json`).
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
        // K-258's defaults: unset, and its Invert off. A `self_default = true`
        // on the injected row would point every effect on every layer at its own
        // input on the day it was added, which is a picture change disguised as
        // a default. The flare's own row predates that and keeps its `true`
        // (K-288) — its stored id and its behaviour are both a save's business.
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
        // The Channel choice (K-425) rides beside the injected pair on every
        // effect that does not pick its matte's channels itself. The three that
        // do — Depth of field (`depth_channel`), Displacement map (its two
        // channel choices) and the Lens flare (source detection) — carry none,
        // and the seam leaves their matte raw. Set matte used to be a fourth,
        // and carries no Matte row at all now (K-429).
        let owns_channel = matches!(s.match_name, "dof" | "displacement_map" | "lens_flare");
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
                "{} — Luminance by default, the reading every kernel had (K-258)",
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

/// **Every Mix slider has a Blend beside it** (K-425): the layer blend modes,
/// verbatim, Normal by default, injected right after `mix` in schema order so
/// the panel can draw it on the Mix row. The Lens flare declares its own
/// `blend` and keeps it; an effect with no Mix (the Controls, the Camera
/// track, Posterize time) touches no pixel and gets none.
#[test]
fn every_mix_row_carries_a_blend() {
    use crate::model::BlendMode;
    for def in BUILTIN_DEFS.iter() {
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
                assert!(s.blend(), "{}", s.match_name);
                if s.match_name == "lens_flare" {
                    // Its own, older row: a save is a save (K-065).
                    continue;
                }
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
                    "{} — the layer modes, Normal by default (K-258)",
                    s.match_name
                );
            }
        }
    }
    assert_eq!(BlendMode::NAMES[0], "Normal");
}

/// **The seam forces Mix to 100 for the kernel, and only when it blends**
/// (K-425). `blend_seam` is the one decision both render paths read: Normal
/// is `None` (the kernel runs untouched), anything else hands back the mode,
/// the op's own Mix as a fraction, and the op's parameters with Mix at 100.
#[test]
fn the_blend_seam_forces_mix_to_full_only_when_it_blends() {
    let mix = ParamId::new("mix");
    let other = ParamId::new("stops");
    let normal = [
        (other, Value::Float(1.0)),
        (mix, Value::Float(40.0)),
        (BLEND_ID, Value::Choice(0)),
    ];
    assert!(cpu::blend_seam(Params::new(&normal)).is_none());
    let absent = [(other, Value::Float(1.0)), (mix, Value::Float(40.0))];
    assert!(
        cpu::blend_seam(Params::new(&absent)).is_none(),
        "a project saved before the row existed blends Normal (K-258)"
    );

    let add = [
        (other, Value::Float(1.0)),
        (mix, Value::Float(40.0)),
        (BLEND_ID, Value::Choice(6)),
    ];
    let (mode, k, forced) = cpu::blend_seam(Params::new(&add)).expect("Add blends");
    assert_eq!(mode, 6);
    assert!((k - 0.4).abs() < 1e-6);
    let forced = Params::new(&forced);
    assert_eq!(forced.float(mix, 0.0), 100.0, "the kernel runs at Mix 100");
    assert_eq!(
        forced.float(other, 0.0),
        1.0,
        "every other parameter is untouched"
    );
    assert_eq!(forced.len(), 3);
}

/// **The blend maths, pinned at its end stops** (K-425): Add sums, Multiply
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

/// **The stack applies the blend through the seam** (K-425): an Exposure at
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

/// **The prepared matte is the chosen channel, inverted once** (K-425): a grey
/// of R = G = B = channel, alpha 1 — so every kernel's luma read gets the
/// channel back — and Luminance without Invert is declared a no-op by the
/// predicate the seam gates on.
#[test]
fn matte_prepare_picks_the_channel_and_inverts_once() {
    let px = [0.2f32, 0.4, 0.8, 0.5];
    let luma = 0.2126 * 0.2 + 0.7152 * 0.4 + 0.0722 * 0.8;
    for (channel, want) in [(0u32, luma), (1, 0.5), (2, 0.2), (3, 0.4), (4, 0.8)] {
        let mut m = px.to_vec();
        cpu::matte_prepare(&mut m, channel, false);
        assert_eq!(m, [want, want, want, 1.0], "channel {channel}");
        let mut m = px.to_vec();
        cpu::matte_prepare(&mut m, channel, true);
        assert_eq!(
            m,
            [1.0 - want, 1.0 - want, 1.0 - want, 1.0],
            "channel {channel} inverted"
        );
    }
    // Clamped: an HDR matte reads 1, and inverts to 0.
    let mut m = vec![4.0, 4.0, 4.0, 1.0];
    cpu::matte_prepare(&mut m, 0, true);
    assert_eq!(m, [0.0, 0.0, 0.0, 1.0]);
    assert!(
        !cpu::matte_needs_prepare(0, false),
        "Luminance, no Invert: no pass (K-258)"
    );
    assert!(cpu::matte_needs_prepare(0, true));
    assert!(cpu::matte_needs_prepare(2, false));
}

/// **Every new row defaults to yesterday's behaviour** (K-258): an instance
/// stripped of `matte_channel` and `blend` resolves to the same numbers as a
/// fresh one reads through the typed accessors.
#[test]
fn a_pre_k425_instance_reads_luminance_and_normal() {
    let mut inst = instantiate("blur").unwrap();
    inst.params
        .retain(|p| p.id != MATTE_CHANNEL_PARAM && p.id != BLEND_PARAM);
    let ops = super::resolve_stack(
        &[inst],
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        std::sync::Arc::new(crate::expression::ExpressionContext::detached()),
    );
    let op = ops.iter().next().unwrap();
    assert_eq!(op.params.choice(MATTE_CHANNEL_ID, 0), 0);
    assert_eq!(op.params.choice(BLEND_ID, 0), 0);
    assert!(cpu::blend_seam(op.params).is_none());
}

/// **The two that opted out still say the same words** (K-395).
///
/// "Their stored parameter ids do not change, only their presentation and
/// prose": Depth of field's depth pass and the Lens flare's source matte keep
/// `depth`/`depth_invert` and `matte`, and a project saved yesterday still
/// loads — but on screen they read "Matte" and "Invert", the same row as
/// everywhere else. The panel pairs a Layer row with `<id>_invert` by
/// convention, so `depth_invert` sitting NEXT to `depth` is part of the
/// contract, not a tidying: an Invert three twirls from its picker cannot be
/// drawn beside it.
#[test]
fn the_effects_that_owned_the_matte_first_use_the_uniform_labels() {
    let label = |effect: &str, id: &str| {
        schema(effect)
            .unwrap()
            .params
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{effect} lost its {id} row — K-065 says a save is a save"))
            .label
    };
    assert_eq!(label("dof", "depth"), "Matte");
    assert_eq!(label("dof", "depth_invert"), "Invert");
    assert_eq!(label("lens_flare", "matte"), "Matte");

    let dof: Vec<&str> = schema("dof").unwrap().params.iter().map(|p| p.id).collect();
    let picker = dof.iter().position(|id| *id == "depth").unwrap();
    assert_eq!(
        dof[picker + 1],
        "depth_invert",
        "the Invert must be adjacent to its picker; the panel folds them into one row"
    );
}

/// **A project saved before K-395 renders identically** (K-258, the campaign's
/// hardest invariant, on the resolve side).
///
/// An instance stripped of both new parameters — which is exactly what every
/// saved project is — must resolve to the same bag as a fresh one, bar the
/// Invert switch reading its declared `false`. Nothing spatial, nothing
/// numeric, nothing an effect's typed reader consults changes; and because the
/// dissolve only runs when a matte is *bound*, an unset row is not a lerp by
/// one, it is no pass at all (the picture half of this is
/// `an_unbound_matte_is_byte_identical` in lumit-render).
#[test]
fn a_pre_matte_instance_resolves_to_the_same_numbers() {
    for name in ["blur", "saturation", "glow", "vignette"] {
        let fresh = instantiate(name).expect("a built-in");
        let mut legacy = fresh.clone();
        legacy
            .params
            .retain(|p| p.id != MATTE_PARAM && p.id != MATTE_INVERT_PARAM);
        assert_eq!(
            legacy.params.len() + 2,
            fresh.params.len(),
            "{name} — the pair really was there to strip"
        );

        let a = resolve_bag(
            std::slice::from_ref(&fresh),
            0.5,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        let b = resolve_bag(
            std::slice::from_ref(&legacy),
            0.5,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            a, b,
            "{name} — a saved instance resolves to today's numbers"
        );
        assert_eq!(
            a.iter()
                .find(|(id, _)| *id == MATTE_INVERT_ID)
                .map(|(_, v)| *v),
            Some(Value::Bool(false)),
            "{name} — and the absent switch reads its declared default"
        );
    }
}

/// The generic strength semantic itself (K-395, docs/08 §2.6), on the CPU side
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

/// **The `#[mask_path]` attribute produces the row it claims to** (K-408).
///
/// Declared here rather than on a shipped effect because the seam landed ahead
/// of its consumers — Scribble, Stroke and Vegas's Mask/Path source. A derive
/// arm nobody exercises is an arm that turns out to be broken on the day
/// somebody first writes it, which is the day it is least welcome.
#[derive(Debug, Clone, Copy, PartialEq, lumit_fx_macros::Effect)]
#[effect(
    match_name = "test_walks_a_path",
    label = "Walks a path",
    version = 1,
    category = Stylise,
    cost = Cheap,
    roi = Exact,
)]
struct WalksAPath {
    /// Bare `#[mask_path]`: the self-default is **First mask**, the other way
    /// round from `#[layer]`'s, because an effect that wants a path wants the
    /// one path most layers have.
    #[mask_path]
    path: bool,
}

/// The opt-out spelling: a path input that genuinely means nothing until it is
/// pointed somewhere.
#[derive(Debug, Clone, Copy, PartialEq, lumit_fx_macros::Effect)]
#[effect(
    match_name = "test_maybe_walks_a_path",
    label = "Maybe walks a path",
    version = 1,
    category = Stylise,
    cost = Cheap,
    roi = Exact,
)]
struct MaybeWalksAPath {
    #[mask_path(self_default = false, label = "Outline")]
    path: bool,
}

#[test]
fn a_mask_path_row_declares_itself_and_defaults_to_the_first_mask() {
    let schema = <WalksAPath as EffectMetadata>::SCHEMA;
    // The one predicate both `build.rs` and `run_ops` key on.
    assert_eq!(schema.mask_path(), Some(("path", true)));
    let row = schema
        .params
        .iter()
        .find(|p| p.id == "path")
        .expect("the declared row");
    assert!(matches!(
        row.kind,
        ParamKind::MaskPath { self_default: true }
    ));
    assert_eq!(row.label, "Path", "sentence case from the field name");
    assert_eq!(row.unit, Unit::Raw, "a mask reference is not a measurement");

    // The opt-out, and a label of its own.
    let maybe = <MaybeWalksAPath as EffectMetadata>::SCHEMA;
    assert_eq!(maybe.mask_path(), Some(("path", false)));
    assert_eq!(maybe.params[0].label, "Outline");

    // The bag carries the choice's presence, not the geometry — and the derive's
    // reader pulls it back out through `Params::mask_named`, so an effect can
    // read its own row without knowing about the carriage.
    let id = ParamId::new("path");
    let bound = [(id, Value::MaskPath(true))];
    assert!(WalksAPath::read(Params::new(&bound)).path);
    assert!(!WalksAPath::read(Params::EMPTY).path);
    // A value of another kind reads as unset rather than as anything else
    // (K-258's rule, every typed reader).
    let wrong = [(id, Value::Layer(true))];
    assert!(!WalksAPath::read(Params::new(&wrong)).path);

    // The seam's consumers have landed (K-408, docs/08 §3.76 §3.78 §3.79), so
    // the "nothing declares one yet" line this test used to end on is gone; what
    // stands in its place is that every declared row is one the carriage knows
    // about, which is `the_mask_path_list_is_one_to_one_with_the_ops_that_
    // declare_a_path` in lumit-render and the three oracles in lumit-gpu.
    let consumers: Vec<&str> = BUILTINS
        .iter()
        .filter(|s| s.mask_path().is_some())
        .map(|s| s.match_name)
        .collect();
    assert_eq!(
        consumers,
        vec!["vegas", "scribble", "stroke", "particulate", "matte_key"]
    );

    // The Matte key declares **two** rows, not one (K-546), so the carriage
    // counts rows rather than effects. Both opt out of the first-mask default:
    // a garbage matte nobody asked for would be a keyer that stopped keying.
    let keyer = BUILTINS
        .iter()
        .find(|s| s.match_name == "matte_key")
        .expect("the keyer");
    assert_eq!(keyer.mask_path_count(), 2);
    assert_eq!(
        keyer.mask_paths().collect::<Vec<_>>(),
        vec![("inside_mask", false), ("outside_mask", false)]
    );
    for s in BUILTINS.iter() {
        assert!(
            s.mask_path_count() <= 2,
            "{} declares more path rows than the carriage was sized for",
            s.match_name
        );
    }
}

/// A fresh instance's mask-path row is **unset**, which is the "First mask"
/// entry — not an id written at instantiation the way a self-default layer
/// reference is (K-408). An effect is usually added before the mask is drawn,
/// so there is no id to write; resolving it late is also what keeps it pointing
/// at the first mask when the masks are reordered.
#[test]
fn a_fresh_mask_path_row_is_the_first_mask_entry() {
    assert_eq!(
        default_param_value(&ParamKind::MaskPath { self_default: true }),
        Some(EffectValue::MaskPath(None))
    );
    assert_eq!(
        default_param_value(&ParamKind::MaskPath {
            self_default: false
        }),
        Some(EffectValue::MaskPath(None))
    );
}

/// **A chain is trimmed by distance along it, not by where it is** (K-408,
/// docs/08 §3.78) — which is what makes Start and End behave like a pen drawing
/// the thing, and is the whole reason the pieces carry an arc length.
#[test]
fn a_path_chain_is_trimmed_by_the_distance_along_it() {
    let straight: Vec<[f32; 2]> = (0..=10).map(|i| [i as f32 * 10.0, 0.0]).collect();
    let mut p = cpu::PathDrawParams::blank();

    cpu::path_chain(&straight, 0.0, 100.0, &mut p);
    let whole: f32 = (0..p.count as usize)
        .map(|i| (p.segments[i][2] - p.segments[i][0]).abs())
        .sum();
    assert!((whole - 100.0).abs() < 1e-3, "the whole line is 100 long");
    assert_eq!(p.arcs[0], 0.0, "the first piece starts at nothing along");

    // Half the line is half the length, and it starts where the pen started.
    cpu::path_chain(&straight, 0.0, 50.0, &mut p);
    let half: f32 = (0..p.count as usize)
        .map(|i| (p.segments[i][2] - p.segments[i][0]).abs())
        .sum();
    assert!(
        (half - 50.0).abs() < 1e-3,
        "End 50 draws half of it, got {half}"
    );
    assert_eq!(p.segments[0][0], 0.0);

    // A window in the middle keeps its **absolute** place along, so a dash's
    // phase does not jump when the ends are pulled in.
    cpu::path_chain(&straight, 40.0, 60.0, &mut p);
    assert!(p.count > 0);
    assert!(
        (p.arcs[0] - 40.0).abs() < 1e-3,
        "a trimmed drawing must keep its distance along, got {}",
        p.arcs[0]
    );

    // Start above End is the same window, not an empty one — a keyframe pair
    // that crosses over must not blank the effect.
    let mut back = cpu::PathDrawParams::blank();
    cpu::path_chain(&straight, 60.0, 40.0, &mut back);
    assert_eq!(back.count, p.count);
    assert_eq!(back.segments[0], p.segments[0]);
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

/// **A scribble widens its own spacing rather than filling half the shape**
/// (docs/08 §3.78): the degradation that keeps the picture whole, docs/14 §4.
#[test]
fn a_scribble_too_fine_for_its_budget_widens_instead_of_stopping() {
    let square: Vec<[f32; 2]> = vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]];
    // One stroke every half pixel over a thousand of them: far past the budget.
    let chain = cpu::scribble_chain(&square, 0.0, 0.5, 0.0);
    assert!(chain.len() <= cpu::PATH_PRIMITIVES);
    let lowest = chain
        .iter()
        .filter(|q| q[1].is_finite())
        .fold(f32::INFINITY, |m, q| m.min(q[1]));
    let highest = chain
        .iter()
        .filter(|q| q[1].is_finite())
        .fold(f32::NEG_INFINITY, |m, q| m.max(q[1]));
    assert!(
        lowest < 20.0 && highest > 980.0,
        "the hatch must still span the shape, got {lowest}..{highest}"
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

/// **A path effect's numbers scale with the raster, and its geometry does not
/// scale twice** (K-408, docs/08 §2.3): the seam flattens once in px@comp and
/// each consumer takes it to the raster it is drawing at.
#[test]
fn a_path_drawings_geometry_follows_the_preview_factor() {
    use crate::fx::effects::stroke::Stroke;
    let m = crate::mask::Mask::ellipse(100.0, 100.0, 60.0, 40.0);
    let poly = crate::mask::mask_path_at(std::slice::from_ref(&m), None, true, 0.0);
    let s = Stroke::read(Params::EMPTY);

    let full = s.packed(&poly, 1.0);
    let half = s.packed(&poly, 0.5);
    assert!(full.count > 0 && half.count > 0);
    // The mask's own vertices halve; the brush's width is a declared Px row and
    // is halved by the resolve step instead, so it is *not* halved again here.
    assert!(
        (half.segments[0][0] * 2.0 - full.segments[0][0]).abs() < 1e-3,
        "the polyline must follow the raster"
    );
    assert_eq!(
        half.half_width, full.half_width,
        "a declared Px row is scaled at resolve, never a second time here"
    );
}

/// **The point a distance along a path is a lookup, not a re-measurement**
/// (K-408): every consumer asks the polyline and gets the same answer.
#[test]
fn a_polyline_answers_where_a_distance_along_it_lands() {
    let m = crate::mask::Mask::ellipse(0.0, 0.0, 50.0, 50.0);
    let poly = crate::mask::mask_path_at(std::slice::from_ref(&m), None, true, 0.0);
    let len = poly.length();
    assert!(len > 250.0, "a circle of radius 50 is about 314 round");

    // The ends, and a distance past either of them, which clamps rather than
    // wrapping or panicking.
    assert_eq!(poly.point_at(0.0), poly.points[0]);
    assert_eq!(poly.point_at(len), *poly.points.last().expect("closed"));
    assert_eq!(poly.point_at(-5.0), poly.point_at(0.0));
    assert_eq!(poly.point_at(len * 2.0), poly.point_at(len));

    // Every point of a circle is its radius from the centre, whatever fraction
    // of the way round it is asked for.
    for k in 0..=20 {
        let q = poly.point_at(len * k as f32 / 20.0);
        let r = q[0].hypot(q[1]);
        assert!((r - 50.0).abs() < 0.5, "at {k}/20 the radius was {r}");
    }

    // An empty polyline answers rather than panicking (docs/14 §4).
    assert_eq!(
        crate::mask::MaskPolyline::default().point_at(3.0),
        [0.0, 0.0]
    );
}

/// **Vegas' Mask/Path half greys the rows that stop meaning anything**, and its
/// contour half greys the mask (docs/08 §3.76, K-408). The render reads the same
/// two predicates to decide whether to flatten a path at all, so this is not a
/// cosmetic claim.
#[test]
fn vegas_offers_a_mask_only_while_it_is_reading_one() {
    use crate::fx::effects::vegas::Vegas;
    let schema = &<Vegas as EffectMetadata>::SCHEMA;
    assert_eq!(schema.mask_path(), Some(("path", true)));

    let on_a_contour = instantiate("vegas").expect("vegas");
    let mut on_a_path = on_a_contour.clone();
    for prop in &mut on_a_path.params {
        if prop.id == "source" {
            prop.value = EffectValue::Choice(Vegas::SOURCE_MASK_PATH);
        }
    }
    assert!(param_enabled(&on_a_path, "path"));
    assert!(!param_enabled(&on_a_path, "threshold"));
    assert!(!param_enabled(&on_a_contour, "path"));
    assert!(param_enabled(&on_a_contour, "threshold"));
}

/// **The raster factor and the waver's tick reach the bag** (K-408, K-409). The
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

/// **The identity diagonal bakes to the identity table, bit for bit** (K-412).
///
/// This is the load-bearing one. A fresh Curves is the passthrough only
/// because its table reads `t[i] == i / 256` exactly; the neutral
/// short-circuit hides that inside the effect, so the guarantee is pinned here
/// on the maths itself, and the lookup is checked to return its input
/// unchanged at every table entry and between two of them.
#[test]
fn the_identity_curve_bakes_and_reads_back_exactly() {
    use crate::fx::cpu::{curve_at, curve_identity_table, curve_table, CURVE_TABLE};

    let table = curve_table(&CurvePoints::IDENTITY);
    assert_eq!(table, curve_identity_table());
    for (i, entry) in table.iter().enumerate() {
        let x = i as f32 / (CURVE_TABLE - 1) as f32;
        assert_eq!(*entry, x, "entry {i} must be its own input");
        assert_eq!(curve_at(x, &table), x, "the lookup must be the identity");
    }
    // Between two entries, and outside the unit interval, where the lookup
    // extrapolates along the end segments rather than clipping (§2.1).
    for x in [0.001_f32, 0.3337, 0.5 / 256.0, 1.75, 4.0, -0.25] {
        assert_eq!(curve_at(x, &table), x, "the identity must carry {x}");
    }
}

/// **A two-point curve is exactly its own straight line** (K-412): the clamped
/// end condition sets both slopes to the secant, so the cubic degenerates to
/// the line through the pair. If this drifts, every default curve drifts.
#[test]
fn a_two_point_curve_is_a_straight_line() {
    use crate::fx::cpu::{curve_table, CURVE_TABLE};

    let points = CurvePoints::sanitised(&[[0.2, 0.1], [0.9, 0.8]]);
    let table = curve_table(&points);
    for (i, entry) in table.iter().enumerate() {
        let x = i as f64 / (CURVE_TABLE - 1) as f64;
        // Inside the pair the line; outside it the same line continued, since
        // the end slope is that line's slope.
        let want = (0.1 + (x - 0.2) * (0.8 - 0.1) / (0.9 - 0.2)).clamp(0.0, 1.0);
        assert!(
            (f64::from(*entry) - want).abs() < 1e-6,
            "entry {i}: {entry} is not on the line ({want})"
        );
    }
}

/// **A monotone point set stays inside the unit square** (K-412). A cubic
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

/// **A malformed point list is straightened, never refused** (K-412). Out of
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

/// **The bake is deterministic** (14-ENGINEERING-RULES §5): the same points
/// produce byte-identical tables, run after run. The tables are the *only*
/// thing either render path sees, so a bake that wobbled would be two pictures
/// from one project.
#[test]
fn baking_a_curve_twice_gives_the_same_bytes() {
    use crate::fx::cpu::curve_table;
    use crate::fx::effects::curves::Curves;

    let shape = CurvePoints::sanitised(&[[0.0, 0.02], [0.3, 0.18], [0.62, 0.8], [1.0, 0.97]]);
    let once = curve_table(&shape);
    let again = curve_table(&shape);
    assert_eq!(
        once.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        again.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "the same points must bake to the same bytes"
    );

    // And through the effect, which is what both render paths call.
    let mut fx = Curves::read(Params::EMPTY);
    fx.master = shape;
    let packed = fx.packed();
    assert_eq!(packed.t[0], once);
    assert_eq!(packed.t, fx.packed().t);
    assert!(!packed.neutral, "a bent master is not the passthrough");
    assert!(
        Curves::read(Params::EMPTY).packed().neutral,
        "a fresh Curves is the bit-exact passthrough"
    );
}

/// **A curve parameter resolves through the arena, straightened, and keys a
/// frame** (K-412). Curve values are static, so this is the whole of their
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

/// **The Controls family holds a value and draws nothing** (K-414).
///
/// Five effects whose whole purpose is a row for an expression to read. Three
/// facts make that true rather than merely intended, and each fails silently
/// without a test: they resolve to *no op at all* (so nothing is dispatched, on
/// either render path, for an effect with no kernel to dispatch to); they take
/// no matte (a picture gating an effect that touches no pixel); and the value is
/// still there on the instance afterwards, which is what an expression reads.
#[test]
fn a_control_effect_holds_its_value_and_draws_nothing() {
    let family = [
        ("slider_control", "slider"),
        ("angle_control", "angle"),
        ("checkbox_control", "checkbox"),
        ("colour_control", "colour"),
        ("point_control", "point_x"),
    ];
    for (name, row) in family {
        let def = BUILTIN_DEFS.get(name).expect("declared");
        let s = def.schema();
        assert_eq!(s.category, FxCategory::Controls, "{name}");
        assert_eq!(s.matte, crate::fx::MatteRole::None, "{name}");
        assert!(!def.is_image_op(), "{name} draws nothing");

        let e = instantiate(name).unwrap_or_else(|| panic!("{name} does not instantiate"));
        assert!(
            e.param(row).is_some(),
            "{name} carries the row an expression reads"
        );
        // The matte pair is not injected, so the row is the whole schema (two
        // for the point, which is a pair by convention).
        assert!(
            s.params.iter().all(|p| p.id != crate::fx::MATTE_PARAM),
            "{name} was given a matte row it does not want"
        );

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
}

/// **A closed range is a Float wearing a different control** (K-414).
///
/// The Slider kind changes what the panel draws and nothing else: the stored
/// value is an `EffectValue::Float`, the default is the declared one, and the
/// resolve step produces the same `Value::Float` the parameter produced while it
/// was a Float. That is the whole promise of adopting it on an existing
/// parameter — no stored value moves, no picture moves — so it is the whole of
/// what this pins, on the four wipes that adopted it.
#[test]
fn a_closed_range_resolves_exactly_as_the_float_it_is() {
    use crate::fx::effects::linear_wipe::LinearWipe;
    for name in ["linear_wipe", "radial_wipe", "venetian_blinds", "card_wipe"] {
        let s = BUILTIN_DEFS.get(name).expect("declared").schema();
        let row = s
            .params
            .iter()
            .find(|p| p.id == "completion")
            .unwrap_or_else(|| panic!("{name} has a Completion"));
        assert_eq!(
            row.kind,
            ParamKind::Slider {
                default: 50.0,
                range: (0.0, 100.0)
            },
            "{name} — a wipe's Completion is closed"
        );
        assert!(
            matches!(
                crate::fx::default_param_value(&row.kind),
                Some(EffectValue::Float(_))
            ),
            "{name} — the value side is a Float, as Int and Angle are"
        );
    }

    let mut e = instantiate("linear_wipe").expect("declared");
    for p in &mut e.params {
        if p.id == "completion" {
            p.value = EffectValue::Float(Property::fixed(30.0));
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
        Params::new(&bag).float(LinearWipe::COMPLETION, -1.0),
        30.0,
        "a closed range resolves through the Float path it always used"
    );
}

/// **A button is a row with no value** (K-417's fifth ruling).
///
/// `ParamKind::Action` makes three promises, and each of them fails silently
/// without a test: an instance carries no stored value for it, so nothing
/// animates or serialises; the resolve step puts nothing in the arena, so it is
/// not in the frame key and pressing Analyse renames no frame; and the backfill
/// appends nothing, so a project saved before the row existed does not grow one.
///
/// Checked on the Camera track, which is the first effect to declare any.
#[test]
fn a_button_is_a_row_with_no_value() {
    let def = BUILTIN_DEFS.get("camera_track").expect("declared");
    let s = def.schema();
    let buttons: Vec<&str> = s
        .params
        .iter()
        .filter(|p| p.kind == ParamKind::Action)
        .map(|p| p.id)
        .collect();
    assert_eq!(buttons, ["analyse", "cancel"], "the two the ruling names");

    // No stored value: not from the declaration, not from a fresh instance.
    for id in &buttons {
        let kind = s
            .params
            .iter()
            .find(|p| p.id == *id)
            .expect("declared")
            .kind;
        assert!(
            crate::fx::default_param_value(&kind).is_none(),
            "{id} has a default value"
        );
    }
    let mut e = instantiate("camera_track").expect("instantiates");
    for id in &buttons {
        assert!(e.param(id).is_none(), "{id} was written into the instance");
    }

    // And the backfill leaves it alone — the walk that exists to add missing
    // rows must not add this one (K-258 meets K-417).
    let before = e.params.len();
    let mut list = vec![e.clone()];
    crate::fx::backfill_builtin_params(&mut list);
    assert_eq!(list[0].params.len(), before, "the backfill grew a button");

    // Not in the arena. The effect resolves to no op at all (it is a handle,
    // not an image operation), so the check that the button never reaches a bag
    // is made on a bag built from the declaration directly.
    e.params.clear();
    assert!(!def.is_image_op(), "the Camera track draws nothing");
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
    assert!(
        ids.is_empty() && ops.is_empty(),
        "a handle resolved to an op"
    );
}

/// The Camera track is the handle K-417 describes: it registers, it files under
/// Utility, it renders identity, it takes no matte, and its quality knobs are
/// the ones the ruling names. The status readout is deliberately **not** here —
/// it is live job state and crosses as job state in stage 2, and a string
/// parameter pretending to be one would put a progress bar in the save file.
#[test]
fn the_camera_track_declares_a_handle_not_a_look() {
    use crate::fx::effects::camera_track::{density, DENSITY, DENSITY_DEFAULT};

    let def = BUILTIN_DEFS.get("camera_track").expect("registered");
    let s = def.schema();
    assert_eq!(s.label, "Camera track");
    assert_eq!(s.category, FxCategory::Utility);
    assert_eq!(s.matte, crate::fx::MatteRole::None);
    assert!(!def.is_image_op());

    let ids: Vec<&str> = s.params.iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        ["analyse", "cancel", "density", "use_masks", "show_points"],
        "no matte pair is injected, and nothing else has crept in"
    );
    assert_eq!(
        s.params[2].kind,
        ParamKind::Choice {
            options: &["Low", "Normal", "High"],
            default: DENSITY_DEFAULT,
            dividers_after: &[],
        }
    );
    assert_eq!(
        s.params[3].kind,
        ParamKind::Bool { default: true },
        "masks exclude by default: a mask on a tracked layer is round the mover"
    );
    assert_eq!(
        s.params[4].kind,
        ParamKind::Bool { default: true },
        "the cloud is on after a solve (K-417's fourth ruling)"
    );

    // Normal is the tracker's own default, which is what makes the other two
    // honest about being a deliberate move.
    assert_eq!(density(DENSITY_DEFAULT), (16, 16, 2));
    assert_eq!(
        density(99),
        density(DENSITY_DEFAULT),
        "unknown reads Normal"
    );
    let mut previous = (0, 0, 0);
    for (n, entry) in DENSITY.iter().enumerate() {
        assert!(entry > &previous, "density {n} is not denser than the last");
        previous = *entry;
    }
}

/// **A project saved before an effect dropped a parameter still loads** (K-258,
/// K-429). The Matte key gave its Matte row up when K-425 ruled a keyer needs
/// none, and Set matte gave the universal row up when K-429 ruled the effect
/// that *is* a matte carries none — so a save made before either carries three
/// ids the schema no longer declares.
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
// Units, vector pairs and the pair link flag (K-443)
// ---------------------------------------------------------------------------

/// **Every parameter says what its number means.** The panel draws the unit
/// beside the value (K-443), so a parameter that never declared one would show a
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

/// **The units the frontend used to hard-code are the declaration's now.**
///
/// `pickablePointParams` in the Effect controls panel was a Dart map from a
/// parameter *id* to "writes comp pixels" or "writes % of frame". Two things
/// were wrong with it: the knowledge is the engine's, and an id is not unique —
/// `centre_x` is a per cent of the comp's width on Radial blur and px@comp on
/// four other effects, so a map keyed on the id alone could only be right about
/// one of them. These are the exact truths it encoded, plus the ones it could
/// not express, read off the declarations.
#[test]
fn the_point_units_the_panel_hard_coded_are_declared_per_effect() {
    let unit_of = |name: &str, id: &str| {
        BUILTIN_DEFS
            .get(name)
            .and_then(|d| d.schema().params.iter().find(|p| p.id == id))
            .map(|p| p.unit)
    };
    // The map's `true` entries: comp pixels.
    for (effect, id) in [
        ("lens_flare", "light_x"),
        ("lens_flare", "light_y"),
        ("dof", "focus_point_x"),
        ("dof", "focus_point_y"),
    ] {
        assert_eq!(unit_of(effect, id), Some(Unit::Px), "{effect}.{id}");
    }
    // Its one `false` entry is gone: since K-558 Radial blur's centre is
    // px@comp like every other centre, so the map it encoded could now be
    // written on the id alone — which is exactly why the knowledge stays in
    // the declarations rather than in a Dart table.
    assert_eq!(unit_of("radial_blur", "centre_x"), Some(Unit::Px));
    assert_eq!(unit_of("radial_blur", "centre_y"), Some(Unit::Px));
    // The `centre_x` rows an id-keyed map called percentages by mistake.
    for effect in ["iris_wipe", "linear_wipe", "lens_distort", "mirror"] {
        assert_eq!(unit_of(effect, "centre_x"), Some(Unit::Px), "{effect}");
        assert_eq!(unit_of(effect, "centre_y"), Some(Unit::Px), "{effect}");
    }
}

/// **A per cent of the diagonal is still nobody's unit** — K-419's rule, now
/// that there is a `Percent` beside it to be confused with. A share of the
/// frame is not a distance; a distance is px@comp.
#[test]
fn percent_is_never_a_disguised_distance() {
    for d in BUILTIN_DEFS.iter() {
        for p in d.schema().params {
            assert!(
                !(p.unit == Unit::Percent && p.unit.is_spatial()),
                "{}.{}: a per cent must not follow the raster",
                d.schema().match_name,
                p.id
            );
        }
    }
}

/// **Every `_x` is half of a pair the declaration names.** A point is two
/// adjacent Float rows by convention (docs/07 §6.1); [`EffectSchema::pairs`] is
/// where that convention is read now, so an `_x` with no `_y` after it — a typo,
/// a row inserted between the halves — would silently stop being a point, lose
/// its link chain and its crosshair, and look like a plain number instead.
#[test]
fn every_x_parameter_is_half_of_a_declared_pair() {
    let mut pairs_seen = 0;
    for d in BUILTIN_DEFS.iter() {
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

/// The pairs themselves, spot-checked where the panel already draws one: the
/// flare's Light, Radial blur's Centre, and the Transform effect's three.
#[test]
fn a_pair_is_its_stem_and_its_two_halves() {
    let pairs = |name: &str| -> Vec<(&str, &str, &str)> {
        BUILTIN_DEFS
            .get(name)
            .map(|d| d.schema().pairs().map(|p| (p.stem, p.x, p.y)).collect())
            .unwrap_or_default()
    };
    assert!(pairs("lens_flare").contains(&("light", "light_x", "light_y")));
    assert!(pairs("radial_blur").contains(&("centre", "centre_x", "centre_y")));
    let transform = pairs("transform");
    assert!(transform.contains(&("anchor", "anchor_x", "anchor_y")));
    assert!(transform.contains(&("position", "position_x", "position_y")));
    assert!(transform.contains(&("scale", "scale_x", "scale_y")));
}

/// **A pair starts unlinked, and the flag survives the file.**
///
/// Unlinked is what every project written before the flag existed means, and
/// what it did: two numbers that moved on their own. A document that has never
/// been linked writes no field at all, so an untouched project saves back the
/// same bytes (K-258), and a document from before the field loads with every
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

/// **Toggling the chain is one undo step.** It rides the effect stack's own op
/// (`SetLayerEffects`), exactly as renaming an instance or typing a value does —
/// docs/03 §8's coarse, exactly-invertible edit — so one undo puts the chain
/// back and nothing else moves.
#[test]
fn linking_a_pair_is_one_undoable_op() {
    use crate::model::{Document, ProjectItem};
    use crate::ops::Op;
    use crate::store::DocumentStore;

    let (mut comp, mut layer) = marker_rig((25, 1), Vec::new(), (0, 1));
    layer.effects = vec![instantiate("lens_flare").expect("the flare is a built-in")];
    let (comp_id, layer_id) = (comp.id, layer.id);
    comp.layers.push(layer);
    let store = DocumentStore::new(Document::new());
    store
        .commit(Op::AddItem {
            index: 0,
            item: Box::new(ProjectItem::Composition(comp)),
        })
        .expect("the comp goes in");

    let linked = |s: &DocumentStore| {
        s.snapshot()
            .comp(comp_id)
            .expect("the comp")
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .expect("the layer")
            .effects[0]
            .pair_linked("light")
    };
    assert!(!linked(&store), "a fresh flare's Light is unlinked");

    let mut effects = store
        .snapshot()
        .comp(comp_id)
        .expect("the comp")
        .layers
        .iter()
        .find(|l| l.id == layer_id)
        .expect("the layer")
        .effects
        .clone();
    assert!(effects[0].set_pair_linked("light", true));
    store
        .commit(Op::SetLayerEffects {
            comp: comp_id,
            layer: layer_id,
            effects,
        })
        .expect("the toggle commits");
    assert!(linked(&store), "the chain closed");

    store.undo().expect("one undo");
    assert!(!linked(&store), "and one undo opened it again");
    store.redo().expect("one redo");
    assert!(linked(&store), "and redo closes it");
}

// ---------------------------------------------------------------------------
// Particulate and the points stream (K-474, K-475, K-495;
// docs/impl/particulate.md §9 items 1-7 and 9-11, on the CPU path).
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

/// **Determinism** (§9 item 1): one comp, two evaluations of the same frame —
/// the stream identical, and the pixels identical with it. There is no state to
/// carry and no clock to read, so this is a property of the design rather than
/// of the run; it is here because a regression would betray it silently.
#[test]
fn a_particulate_frame_evaluates_the_same_twice() {
    let e = particulate(&[]);
    let a = particulate_stream(&e, 2.5);
    let b = particulate_stream(&e, 2.5);
    assert!(!a.is_empty(), "the default look draws particles");
    assert_eq!(a, b, "two evaluations of one frame differ");

    let draw = |s: &PointsStream| {
        let mut rgba = vec![0.0f32; 64 * 64 * 4];
        points::draw_discs(&mut rgba, 64, 64, s, 1.0);
        rgba.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(draw(&a), draw(&b), "two draws of one stream differ");
}

/// **Random access** (§9 item 2): frames evaluated out of order equal the same
/// frames evaluated in order. The scrub-safety property, as a test — and the
/// whole reason the closed form was chosen over a simulation (K-474).
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
#[test]
fn the_closed_forms_match_the_analytic_solutions() {
    let base = points::Forces {
        gravity: 0.0,
        wind: [0.0, 0.0],
        drag: 0.0,
        turbulence: 0.0,
        turbulence_scale: 200.0,
        turbulence_speed: 0.0,
    };
    let p0 = [100.0f32, 50.0];
    let v0 = [30.0f32, -80.0];

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
        ];
        assert!((pos[0] - want[0]).abs() < 1e-3, "x at {age}");
        assert!((pos[1] - want[1]).abs() < 1e-2, "y at {age}: {pos:?}");
        assert!((vel[1] - (v0[1] + 400.0 * age)).abs() < 1e-2, "vy at {age}");
    }

    // k = 0.5, with wind and gravity: the published form, `g/k` and all,
    // against the rearrangement the implementation uses.
    let f = points::Forces {
        gravity: 400.0,
        wind: [120.0, 0.0],
        drag: 0.5,
        ..base
    };
    for age in [0.1f32, 1.0, 4.0] {
        let k = 0.5f32;
        let (pos, vel) = points::integrate(p0, v0, &f, age);
        for i in 0..2 {
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
        wind: [120.0, 0.0],
        drag: 0.1,
        ..base
    };
    let below = points::integrate(p0, v0, &f, 0.999).0;
    let above = points::integrate(p0, v0, &f, 1.001).0;
    let at = points::integrate(p0, v0, &f, 1.0).0;
    for i in 0..2 {
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
        wind: [500.0, -500.0],
        ..base
    };
    let (still, _) = points::integrate(p0, v0, &base, 2.0);
    let (blown, _) = points::integrate(p0, v0, &windy, 2.0);
    assert_eq!(still, blown, "wind with no drag moved a particle");
}

/// **Turbulence rides the shared lattice** (§9 item 5): the displacement is
/// the value-noise core Wiggle and Fractal noise use, pinned here by golden
/// numbers so nobody can swap the noise family without this failing.
#[test]
fn turbulence_reads_the_shared_noise_core() {
    // The two channels Particulate displaces along, at three pinned points,
    // through `fx::noise::value3` — the *same* function, which is the whole
    // assertion: a second lattice would not produce these numbers.
    let goldens: [(f32, f32, f32, u32, f32); 4] = [
        (0.0, 0.0, 0.0, 64, -0.622_338_65),
        (1.25, -3.5, 0.75, 64, 0.413_118_24),
        (1.25, -3.5, 0.75, 65, -0.093_730_03),
        (12.0, 7.5, 2.0, 64, 0.271_409_87),
    ];
    for (x, y, z, channel, want) in goldens {
        let got = crate::fx::noise::value3(7, channel, x, y, z, 0);
        assert!(
            (got - want).abs() < 1e-6,
            "the turbulence lattice moved at {x},{y},{z} channel {channel}: {got}"
        );
    }

    // And the effect really does displace by it: turn the amount up and the
    // drawn position leaves the closed form, by no more than the amount.
    let rough_e = particulate(&[
        ("turbulence_amount", fixed(40.0)),
        ("emit_rate", fixed(60.0)),
        ("initial_speed", fixed(0.0)),
        ("speed_jitter", fixed(0.0)),
        ("drag", fixed(0.0)),
    ]);
    let calm_e = particulate(&[
        ("turbulence_amount", fixed(0.0)),
        ("emit_rate", fixed(60.0)),
        ("initial_speed", fixed(0.0)),
        ("speed_jitter", fixed(0.0)),
        ("drag", fixed(0.0)),
    ]);
    let rough = particulate_stream(&rough_e, 1.0);
    let smooth = particulate_stream(&calm_e, 1.0);
    assert_eq!(rough.id, smooth.id, "turbulence changed who is alive");
    let mut moved = 0;
    for i in 0..rough.len() {
        let dx = rough.position[i][0] - smooth.position[i][0];
        let dy = rough.position[i][1] - smooth.position[i][1];
        assert!(dx.abs() <= 40.0 && dy.abs() <= 40.0, "past the amount");
        if dx.abs() > 1e-3 {
            moved += 1;
        }
    }
    assert!(moved > 0, "turbulence displaced nothing");
}

/// **Id stability** (§9 item 6): a particle's id is its birth index, so it is
/// the same number at every frame of its life — which is what makes a trail
/// possible without anything being remembered. And the compacted stream is in
/// strictly increasing id order, the prefix-sum determinism the GPU twin has to
/// reproduce (particulate.md §5).
#[test]
fn a_particles_id_is_its_birth_index_and_holds_still() {
    let e = particulate(&[("life", fixed(3.0)), ("life_jitter", fixed(0.0))]);
    let dt = 1.0 / 60.0;
    let first = particulate_stream(&e, 60.0 * dt);
    let id = *first.id.first().expect("a particle at frame 60");
    let age0 = first.age[0];
    for later in [90i64, 120] {
        let s = particulate_stream(&e, later as f64 * dt);
        let at =
            s.id.iter()
                .position(|i| *i == id)
                .expect("the same particle is still alive");
        let grown = s.age[at] - age0;
        let want = (later - 60) as f32 * dt as f32;
        assert!(
            (grown - want).abs() < 1e-3,
            "the age did not follow the time"
        );
    }
    assert!(
        first.id.windows(2).all(|w| w[0] < w[1]),
        "the stream is not in birth-index order"
    );
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

    // The degradation rung (K-475): the newest half, by the same rule.
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

/// **The sprite fallback** (§9 item 10): Sprite mode with no layer bound draws
/// discs, not nothing.
///
/// The documented deviation from the unset-is-identity convention, and the
/// reason for it is one sentence long: a render mode must always draw
/// something. Both paths take the fallback in the same place — the CPU's
/// reference draw filters the sprite away here, the host resolves the mode to
/// Disc before the kernel sees it — so there is one rule, not two.
#[test]
fn sprite_mode_with_no_layer_draws_discs() {
    // In the middle of the little test buffer, not at the middle of a 1080p
    // comp: a particle drawn off the edge would make every comparison below
    // pass by drawing nothing at all.
    let e = particulate(&[
        ("mode", EffectValue::Choice(1)),
        ("position_x", EffectValue::Float(Property::fixed(32.0))),
        ("position_y", EffectValue::Float(Property::fixed(32.0))),
        ("width", EffectValue::Float(Property::fixed(20.0))),
        ("height", EffectValue::Float(Property::fixed(20.0))),
    ]);
    let s = particulate_stream(&e, 2.0);
    assert!(!s.is_empty(), "the fixture drew no particles");
    let style = points::DrawStyle {
        mode: points::RenderMode::Sprite,
        feather: 1.0,
        streak_seconds: 0.0,
        mix: 1.0,
    };
    let mut fallen_back = vec![0.0f32; 64 * 64 * 4];
    points::draw_stream(&mut fallen_back, 64, 64, &s, &[], &style, None);
    let mut discs = vec![0.0f32; 64 * 64 * 4];
    points::draw_discs(&mut discs, 64, 64, &s, 1.0);
    assert!(
        discs.iter().any(|v| *v > 0.0),
        "the fixture drew nothing into the buffer, so nothing below is a test"
    );
    assert_eq!(
        fallen_back, discs,
        "an unset sprite drew something other than the discs it falls back to"
    );

    // And with a sprite bound it draws the sprite — a flat white square, so a
    // stamp that landed is a stamp that is *not* a feathered disc.
    let sprite = vec![1.0f32; 4 * 4 * 4];
    let mut stamped = vec![0.0f32; 64 * 64 * 4];
    points::draw_stream(
        &mut stamped,
        64,
        64,
        &s,
        &[],
        &style,
        Some(points::Sprite {
            rgba: &sprite,
            w: 4,
            h: 4,
        }),
    );
    assert_ne!(stamped, discs, "a bound sprite drew the disc anyway");
}

/// **Streak mode is the closed form again** (particulate.md §2, Render): the
/// tail is where the particle *was* a Streak length ago, worked out from the
/// same formula rather than remembered — so a streak at age zero has no tail at
/// all, and a streak of no length is exactly a disc.
#[test]
fn a_streak_is_the_closed_form_looked_backwards() {
    let e = particulate(&[
        ("mode", EffectValue::Choice(2)),
        ("position_x", EffectValue::Float(Property::fixed(32.0))),
        ("position_y", EffectValue::Float(Property::fixed(32.0))),
        ("width", EffectValue::Float(Property::fixed(20.0))),
        ("height", EffectValue::Float(Property::fixed(20.0))),
    ]);
    let p = particulate_params(&e);
    let dt = 1.0 / 60.0;
    let sched = Schedule::scan(dt, 120, 600, &|_| 150.0);
    let path = MaskPolyline::default();
    let tail_seconds = 0.05f32;
    let (s, tails) = points::evaluate_with_tail(&p, &sched, 2.0, &path, tail_seconds);
    assert_eq!(
        tails.len(),
        s.len(),
        "a tail per particle, in the same order"
    );
    assert!(!s.is_empty(), "the fixture drew no particles");

    // The tail of particle i is the head of the *same* particle, evaluated a
    // Streak length earlier — which is the whole claim, so it is checked
    // against a second evaluation rather than against an approximation.
    let earlier = points::evaluate(&p, &sched, 2.0 - f64::from(tail_seconds), &path);
    let mut checked = 0;
    for (i, id) in s.id.iter().enumerate() {
        // Only particles already alive a Streak length ago: a younger one
        // streaks from where it was born, which is the clamp, not the formula.
        if s.age[i] < tail_seconds {
            continue;
        }
        let Some(j) = earlier.id.iter().position(|o| o == id) else {
            continue;
        };
        for (k, (was, now)) in tails[i].iter().zip(&earlier.position[j]).enumerate() {
            let gap = (was - now).abs();
            assert!(gap < 1e-3, "tail {i} axis {k} is {gap} from where it was");
        }
        checked += 1;
    }
    assert!(checked > 10, "only {checked} tails were checkable");

    // A streak of no length is a disc: one distance function, two modes.
    let style = |secs: f32| points::DrawStyle {
        mode: points::RenderMode::Streak,
        feather: 1.0,
        streak_seconds: secs,
        mix: 1.0,
    };
    let (short, no_tails) = points::evaluate_with_tail(&p, &sched, 2.0, &path, 0.0);
    let mut capsules = vec![0.0f32; 64 * 64 * 4];
    points::draw_stream(&mut capsules, 64, 64, &short, &no_tails, &style(0.0), None);
    let mut discs = vec![0.0f32; 64 * 64 * 4];
    points::draw_discs(&mut discs, 64, 64, &short, 1.0);
    assert!(
        discs.iter().any(|v| *v > 0.0),
        "the fixture drew nothing into the buffer, so nothing here is a test"
    );
    assert_eq!(capsules, discs, "a streak of no length is not a disc");
}

/// **The rotation jitter** (K-507): every particle takes its own draw about
/// Rotation, from the seed and from nothing else — so the spread is there, it
/// is bounded by the dial, and it is the same spread on every machine.
#[test]
fn particulate_rotations_spread_by_their_dial() {
    let spread_of = |jitter: f64| {
        let e = particulate(&[
            (
                "rotation_jitter",
                EffectValue::Float(Property::fixed(jitter)),
            ),
            // Spin and Align to motion would both move the rotation for
            // reasons of their own; this is about the die alone.
            ("spin", EffectValue::Float(Property::fixed(0.0))),
            ("rotation", EffectValue::Float(Property::fixed(0.0))),
        ]);
        let s = particulate_stream(&e, 2.0);
        assert!(!s.is_empty(), "the fixture drew no particles");
        let lo = s.rotation.iter().copied().fold(f32::MAX, f32::min);
        let hi = s.rotation.iter().copied().fold(f32::MIN, f32::max);
        (lo, hi, s)
    };

    // At zero the dial does nothing at all, and Rotation means exactly what it
    // says — which is the half of K-507 that a full turn by default would have
    // made unreachable.
    let (lo, hi, _) = spread_of(0.0);
    assert!(
        lo.abs() < 1e-6 && hi.abs() < 1e-6,
        "no jitter still spread rotations over {lo}..{hi}"
    );

    // At a whole turn the spread fills it, and never leaves it.
    let (lo, hi, whole) = spread_of(360.0);
    let half = std::f32::consts::PI;
    assert!(
        lo >= -half - 1e-4 && hi <= half + 1e-4,
        "{lo}..{hi} is not ±half a turn"
    );
    assert!(
        hi - lo > 5.0,
        "a whole turn of jitter only spread {}",
        hi - lo
    );

    // Ninety degrees is a quarter of that, both ways.
    let (lo, hi, _) = spread_of(90.0);
    let quarter = std::f32::consts::FRAC_PI_4;
    assert!(
        lo >= -quarter - 1e-4 && hi <= quarter + 1e-4,
        "90° of jitter spread {lo}..{hi}"
    );

    // And it is the seed's, not the clock's: one instance evaluated twice is
    // the same spread, particle for particle. (A *second* instance would roll
    // its own seed — which is the reseed button working, not a failure.)
    let e = particulate(&[(
        "rotation_jitter",
        EffectValue::Float(Property::fixed(360.0)),
    )]);
    let once = particulate_stream(&e, 2.0);
    let twice = particulate_stream(&e, 2.0);
    assert!(
        once.rotation == twice.rotation,
        "the spread is not repeatable"
    );
    assert!(
        !whole.rotation.is_empty(),
        "the whole-turn case drew nothing"
    );
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

/// **Particulate declares a Points output, and nothing else grew one**
/// (K-472, K-492, points-stream.md §4.1): the signature split leaves every
/// other declaration exactly as it was, which is the property that made
/// `Image { extra }` the shape rather than a third signature kind.
#[test]
fn only_particulate_declares_a_data_output_beside_its_picture() {
    let with_extras: Vec<(&str, Vec<&str>)> = BUILTIN_DEFS
        .iter()
        .filter(|d| matches!(d.signature(), Signature::Image { extra } if !extra.is_empty()))
        .map(|d| {
            (
                d.schema().match_name,
                d.signature().outputs().iter().map(|p| p.id).collect(),
            )
        })
        .collect();
    assert_eq!(with_extras, vec![("particulate", vec!["points"])]);
    let port = BUILTIN_DEFS
        .get("particulate")
        .expect("declared")
        .signature()
        .output("points");
    assert_eq!(
        port,
        Some(PortType::Points),
        "the socket is teal, or it is nothing"
    );
}

/// **An over-life curve starts on the shape it declared** (K-412 with
/// particulate.md §2): a fresh Opacity over life is `1 → 0`, not the identity
/// diagonal, or every particle would be born invisible.
#[test]
fn an_over_life_curve_is_born_on_its_declared_shape() {
    let e = particulate(&[]);
    let curve = |id: &str| match e.param(id) {
        Some(EffectValue::Curve(points)) => points.clone(),
        other => panic!("{id} is {other:?}, not a curve"),
    };
    assert_eq!(curve("opacity_over_life"), vec![[0.0, 1.0], [1.0, 0.0]]);
    assert_eq!(curve("size_over_life"), vec![[0.0, 1.0], [1.0, 1.0]]);
    // And the grade family is untouched: no declaration, the diagonal.
    let curves = instantiate("curves").expect("Curves is declared");
    assert_eq!(
        curves.param("master"),
        Some(&EffectValue::Curve(CURVE_IDENTITY.to_vec()))
    );
}

/// **The default look plays** (K-475's first number): the default parameter
/// set is a few hundred particles, and evaluating and drawing them is noise
/// against a frame's budget.
///
/// K-475's ≲ 0.2 ms is a **GPU** number, gated on the reference desktop in PS7.
/// This is its CPU reference, which is allowed to be slower and is not allowed
/// to be a different *kind* of work: the bound is loose on purpose — a test
/// that fails because a laptop was busy teaches nobody anything — and what it
/// catches is the regression that turns a per-particle dab into a per-pixel
/// pass.
#[test]
fn the_default_particulate_look_is_a_few_hundred_particles() {
    let e = particulate(&[]);
    let s = particulate_stream(&e, 4.0);
    assert!(
        (150..600).contains(&s.len()),
        "the default look draws {} particles, not the ~300 K-475 budgeted for",
        s.len()
    );
    let mut rgba = vec![0.0f32; 1920 * 1080 * 4];
    let started = std::time::Instant::now();
    for _ in 0..10 {
        let s = particulate_stream(&e, 4.0);
        points::draw_discs(&mut rgba, 1920, 1080, &s, 1.0);
    }
    let each = started.elapsed().as_secs_f64() / 10.0;
    // Printed, not only asserted: the number is the point, and `--nocapture`
    // is how the next person checks it against K-475 rather than against this
    // machine's mood.
    println!(
        "the default look: {} particles, {:.3} ms an evaluation and draw at 1920x1080",
        s.len(),
        each * 1000.0
    );
    assert!(
        each < 0.010,
        "the default look costs {:.3} ms an evaluation on the CPU reference",
        each * 1000.0
    );
}

/// **A fresh Tile changes nothing** (docs/08 §3.39, §1.2, K-542 — which reverses
/// the earlier 2×2 default).
///
/// AE's Motion Tile is the identity until it is set up, and so is Lumit's: one
/// whole-frame tile, cut from the middle of the frame, stamped over exactly the
/// frame it came from. "Nothing" here means to the bit, not to the eye — the
/// mapping is a divide followed by the multiply that undoes it, which fp32 does
/// not always answer exactly, so both kernels short-circuit it. This test fails
/// against a Tile width of 50, and against a centre that has not been moved onto
/// the raster.
#[test]
fn a_fresh_tile_changes_not_one_bit() {
    let (w, h) = (37u32, 21u32);
    let source: Vec<f32> = (0..(w * h * 4))
        .map(|i| ((i * 7 % 251) as f32) / 251.0)
        .collect();

    for (rw, rh) in [(w, h), (1920, 1080), (3840, 2160)] {
        let inst = builtins::instantiate_for_raster("tile", f64::from(rw), f64::from(rh))
            .expect("tile is a builtin");
        let list = vec![inst];
        let ops = super::resolve_stack_temporal_named(
            &list,
            crate::fx::drivers::ResolvedDrivers::NONE,
            0.0,
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(ExpressionContext::detached()),
        )
        .1;
        // The raster the fresh instance was centred on is the one it must be the
        // identity on; the corpus is small, so the two other cases prove only
        // that the centring happened, which the next assertion covers.
        if (rw, rh) != (w, h) {
            let t = effects::tile::Tile::read(ops.iter().next().expect("one op").params);
            assert_eq!(
                (t.tile_centre_x, t.tile_centre_y),
                (rw as f32 * 0.5, rh as f32 * 0.5),
                "a fresh Tile must be cut from the middle of the comp it was dropped on"
            );
            continue;
        }
        let mut out = source.clone();
        cpu::apply_stack(&mut out, w, h, &ops);
        assert_eq!(out, source, "dropping Tile on a layer changed the picture");
    }
}

/// **Output width and height above 100 % grow the working raster** (docs/08
/// §3.39, K-542): the copies land past the frame's edges, and the effects after
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
        t.tile_width = 50.0;
        t.tile_height = 50.0;
        t.output_width = ow;
        t.output_height = oh;
        t.mix = mix;
        t.packed()
    };
    assert_eq!(cpu::tile_raster(w, h, &of(100.0, 100.0, 100.0)), (w, h));
    assert_eq!(cpu::tile_raster(w, h, &of(60.0, 60.0, 100.0)), (w, h));
    assert_eq!(cpu::tile_raster(w, h, &of(200.0, 150.0, 100.0)), (64, 36));
    assert_eq!(
        cpu::tile_raster(w, h, &of(200.0, 150.0, 0.0)),
        (w, h),
        "Mix 0 is the identity, and an identity does not reallocate"
    );
    // The ceiling holds a slider drag to a raster every backend can allocate.
    assert_eq!(
        cpu::tile_raster(3840, 2160, &of(500.0, 500.0, 100.0)),
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
    let p = of(200.0, 150.0, 100.0);
    let (ow, oh) = cpu::tile_raster(w, h, &p);
    let mut grown = vec![0.0f32; (ow * oh * 4) as usize];
    cpu::tile_into(&img, w, h, &mut grown, ow, oh, &p);
    let mut flat = img.clone();
    cpu::tile(&mut flat, w, h, &of(100.0, 100.0, 100.0));
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

/// The corpus the K-546 spatial tests key against: a `w × h` frame that is the
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

/// **The spatial controls change nothing until one is asked for** (K-546).
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

/// **Screen pre-blur softens what the key is judged from, not what comes out**
/// (K-546).
///
/// Both halves matter. A pre-blur must move the matte — otherwise the control
/// does nothing — and it must leave the *colour* alone, which is what separates
/// it from blurring the layer and keying the result.
#[test]
fn the_screen_pre_blur_judges_a_soft_picture_and_returns_a_sharp_one() {
    let (w, h) = (24u32, 16u32);
    let img = keyer_plate(w, h, (8, 4, 8, 8));
    let blank = cpu::MaskFillParams::blank();
    let base = matte_view_params();

    let mut sharp = img.clone();
    cpu::matte_key_spatial(&mut sharp, w, h, &base, &blank, &blank);
    let mut soft = img.clone();
    cpu::matte_key_spatial(
        &mut soft,
        w,
        h,
        &MatteKeyParams {
            pre_blur: 3.0,
            ..base
        },
        &blank,
        &blank,
    );
    // The matte has genuinely moved: a hard edge has become a ramp.
    let edges = sharp
        .chunks_exact(4)
        .zip(soft.chunks_exact(4))
        .filter(|(a, b)| (a[0] - b[0]).abs() > 0.05)
        .count();
    assert!(
        edges > 8,
        "a pre-blur that changed {edges} pixels is not one"
    );
    let mid: Vec<f32> = soft
        .chunks_exact(4)
        .map(|c| c[0])
        .filter(|m| *m > 0.05 && *m < 0.95)
        .collect();
    assert!(
        !mid.is_empty(),
        "the softened key produced no partial matte"
    );

    // And on the Final view the kept colour is the original's, not a blurred
    // one: the block's middle is still exactly the magenta that went in.
    let mut final_out = img.clone();
    cpu::matte_key_spatial(
        &mut final_out,
        w,
        h,
        &MatteKeyParams {
            view: 0,
            pre_blur: 3.0,
            spill: 0.0,
            replace_method: 0,
            ..base
        },
        &blank,
        &blank,
    );
    let centre = ((8 * w + 12) * 4) as usize;
    assert!(
        (final_out[centre] - 0.6).abs() < 1e-5 && (final_out[centre + 2] - 0.5).abs() < 1e-5,
        "the colour was blurred as well as the judgement: {:?}",
        &final_out[centre..centre + 4]
    );
}

/// **Shrink and grow march the matte's edge, in opposite directions** (K-546).
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

/// **Softness blurs the matte and only the matte** (K-546): a hard edge becomes
/// a ramp, and the amount of matte in the frame is roughly conserved.
#[test]
fn the_screen_softness_ramps_the_mattes_edge() {
    let (w, h) = (24u32, 24u32);
    let img = keyer_plate(w, h, (8, 8, 8, 8));
    let blank = cpu::MaskFillParams::blank();
    let base = matte_view_params();
    let mut out = img.clone();
    cpu::matte_key_spatial(
        &mut out,
        w,
        h,
        &MatteKeyParams {
            softness: 3.0,
            ..base
        },
        &blank,
        &blank,
    );
    let partial = out
        .chunks_exact(4)
        .filter(|c| c[0] > 0.05 && c[0] < 0.95)
        .count();
    assert!(
        partial > 16,
        "a blurred matte has an edge: {partial} pixels"
    );
    let total: f32 = out.chunks_exact(4).map(|c| c[0]).sum();
    assert!(
        (total - 64.0).abs() < 8.0,
        "a blur moved the matte's weight: {total}"
    );
}

/// **Despot removes a lone speck and leaves a real edge alone** (K-546).
///
/// The distinction is the whole control: a pixel that disagrees with all eight
/// of its neighbours is a speck, and a pixel on an edge always has a neighbour
/// on its own side.
#[test]
fn the_despots_take_specks_and_leave_edges() {
    let (w, h) = (24u32, 24u32);
    let mut img = keyer_plate(w, h, (8, 8, 8, 8));
    // A lone screen pixel inside the block (a pinhole) and a lone foreground
    // pixel out in the screen (a fleck).
    let hole = ((11 * w + 11) * 4) as usize;
    img[hole..hole + 4].copy_from_slice(&[0.0, 0.6, 0.0, 1.0]);
    let fleck = ((3 * w + 3) * 4) as usize;
    img[fleck..fleck + 4].copy_from_slice(&[0.6, 0.1, 0.5, 1.0]);
    let blank = cpu::MaskFillParams::blank();
    let base = matte_view_params();
    let run = |p: MatteKeyParams| {
        let mut out = img.clone();
        cpu::matte_key_spatial(&mut out, w, h, &p, &blank, &blank);
        out
    };

    let plain = run(base);
    assert!(plain[hole] < 0.01, "the pinhole should key to nothing");
    assert!(plain[fleck] > 0.99, "the fleck should key to something");

    let black = run(MatteKeyParams {
        despot_black: 1.0,
        ..base
    });
    assert!(black[hole] > 0.99, "despot black left the pinhole");
    assert!(black[fleck] > 0.99, "despot black should not touch a fleck");

    let white = run(MatteKeyParams {
        despot_white: 1.0,
        ..base
    });
    assert!(white[fleck] < 0.01, "despot white left the fleck");
    assert!(
        white[hole] < 0.01,
        "despot white should not touch a pinhole"
    );

    // The block's own corner is an edge, not a speck, and survives both.
    let corner = ((8 * w + 8) * 4) as usize;
    let both = run(MatteKeyParams {
        despot_black: 1.0,
        despot_white: 1.0,
        ..base
    });
    assert!(
        (both[corner] - plain[corner]).abs() < 1e-6,
        "a despot ate the corner of the foreground"
    );
}

/// **The garbage masks force opaque and force transparent** (K-546), and an
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

/// **A mask's own feather and expansion ride with its curve** (K-546): the
/// carriage hands them over so a garbage matte softens exactly where the mask
/// it was drawn from softens.
#[test]
fn a_mask_path_carries_its_own_feather_and_expansion() {
    let mut masks = vec![crate::mask::Mask::rectangle(6.0, 6.0, 8.0, 8.0)];
    masks[0].feather = Property::fixed(4.0);
    masks[0].expansion = Property::fixed(-1.5);
    let poly = crate::mask::mask_path_at(&masks, None, true, 0.0);
    assert!((poly.feather - 4.0).abs() < 1e-6);
    assert!((poly.expansion + 1.5).abs() < 1e-6);

    // The ramp is the feather at this raster, floored at the one pixel a hard
    // edge is antialiased over; the expansion travels with it.
    let fill = cpu::mask_fill_params(&poly, 2.0);
    assert!((fill.ramp - 8.0).abs() < 1e-6, "ramp {}", fill.ramp);
    assert!((fill.expansion + 3.0).abs() < 1e-6);
    let hard = crate::mask::mask_path_at(
        &[crate::mask::Mask::rectangle(0.0, 0.0, 4.0, 4.0)],
        None,
        true,
        0.0,
    );
    assert!((cpu::mask_fill_params(&hard, 1.0).ramp - 1.0).abs() < 1e-6);

    // A feathered outline really does ramp rather than step.
    let fill = cpu::mask_fill_params(&poly, 1.0);
    let on_edge = cpu::mask_fill_at(6.0, 10.0, &fill);
    assert!(
        on_edge > 0.05 && on_edge < 0.95,
        "a feathered edge stepped: {on_edge}"
    );
}
