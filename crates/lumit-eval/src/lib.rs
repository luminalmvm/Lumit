//! The seed of **Nova**'s evaluator (K-067): pure content-hash frame keys
//! per docs/06-RENDER-PIPELINE.md §5.2.
//!
//! In plain terms: before rendering a frame, Lumit writes down everything
//! that could possibly change its pixels — sizes, evaluated property values,
//! source frames, masks, the camera — and hashes it into one 128-bit number.
//! Two frames with the same number are the same picture, so the second one
//! comes from the cache instead of rendering again. Nothing else invalidates
//! the cache: an edit changes values, values change hashes, and stale entries
//! simply stop being looked up (the After Effects Global Performance Cache
//! lesson, taken whole).
//!
//! The normative rules from the spec, enforced here and locked by tests:
//! - **No instance identity, no timeline position** in any key: layer/comp
//!   ids never feed the hash, in/out points only gate which layers appear,
//!   and `start_offset` matters only through the evaluated local time. Two
//!   identical comps hash identically; a time-shifted static layer keeps its
//!   keys.
//! - **Evaluated values, not keyframe data**: a property animated elsewhere
//!   but constant here hashes the same across the constant span.
//! - **Algorithm version** (`ALGO_VERSION`) is bumped whenever rendering
//!   output changes, invalidating every old entry by construction.

use lumit_core::model::{Composition, Document, LayerKind, MatteChannel};
use uuid::Uuid;

pub mod epoch;
pub mod exec;
pub mod graph;
pub mod pool;
pub mod schedule;

/// Bump when any rendering algorithm's output changes: every cached frame
/// keyed under the old version stops being addressed.
pub const ALGO_VERSION: u32 = 1;

/// A 128-bit content hash addressing one rendered comp frame (docs/06 §5.2:
/// collisions are treated as impossible; no structural comparison at lookup).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameKey(pub u128);

/// The quality axis of the key: the same frame at half preview resolution is
/// a different cache entry (docs/06 §6: each tier's caches are first-class).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    /// Preview resolution divisor (1 = Full, 2 = Half, …).
    pub divisor: u32,
}

impl Default for Quality {
    fn default() -> Self {
        Self { divisor: 1 }
    }
}

/// Supplies the content identity of footage pixels: which source, and which
/// source frame, a Footage layer shows at layer time `lt`. The evaluator
/// depends on media through this trait (docs/05-ARCHITECTURE.md: trait
/// objects defined in lumit-eval itself), so the crate stays engine-pure.
pub trait SourceStamper {
    /// (source identity, source frame index), or None while the media is
    /// still unprobed — an unknown source makes the whole frame unkeyable.
    fn stamp(&self, item: Uuid, lt: f64) -> Option<(String, u64)>;
}

/// The content-hash key for `comp` rendered at time `t` — or None when some
/// footage is not yet identifiable (no probe), in which case the frame is
/// rendered live and simply not cached.
pub fn comp_frame_key(
    doc: &Document,
    comp: &Composition,
    t: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
) -> Option<FrameKey> {
    let mut visited = Vec::new();
    let mut h = blake3::Hasher::new();
    feed_comp(&mut h, doc, comp, t, quality, stamper, &mut visited)?;
    let bytes = h.finalize();
    let mut k = [0u8; 16];
    k.copy_from_slice(&bytes.as_bytes()[..16]);
    Some(FrameKey(u128::from_le_bytes(k)))
}

fn feed_comp(
    h: &mut blake3::Hasher,
    doc: &Document,
    comp: &Composition,
    t: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    h.update(b"comp/");
    h.update(&ALGO_VERSION.to_le_bytes());
    h.update(&comp.width.to_le_bytes());
    h.update(&comp.height.to_le_bytes());
    h.update(&quality.divisor.to_le_bytes());
    for c in comp.background.0 {
        h.update(&c.to_le_bytes());
    }
    match comp.camera_pose(t) {
        Some(pose) => {
            h.update(b"cam");
            for v in [
                pose.zoom,
                pose.position.0,
                pose.position.1,
                pose.position.2,
                pose.rotation_deg.0,
                pose.rotation_deg.1,
                pose.rotation_deg.2,
            ] {
                feed_f64(h, v);
            }
        }
        None => {
            h.update(b"flat");
        }
    }
    // Comp-wide motion blur (docs/06 §4, K-120): the shutter shape is content
    // for every layer that blurs. Hashed only when the master is on AND at least
    // one layer actually has its motion-blur switch set — so toggling the master
    // (or nudging the shutter) in a comp where nothing blurs changes no pixels
    // and retires no cached frames.
    if comp.motion_blur.enabled && comp.layers.iter().any(|l| l.switches.motion_blur) {
        h.update(b"mblur/");
        feed_f64(h, comp.motion_blur.shutter_angle);
        feed_f64(h, comp.motion_blur.shutter_phase);
        h.update(&comp.motion_blur.samples.to_le_bytes());
    }
    // Draw order is content: iterate the stack as rendered. Layers outside
    // their span, hidden, or muted by someone else's solo (K-105) contribute
    // nothing — presence is gated, never hashed, so trimming a bar without
    // crossing `t` changes no key, and soloing the only contributing layer
    // (same picture) keeps its cached frames valid.
    let any_solo = lumit_core::model::any_solo(comp);
    for layer in &comp.layers {
        let in_span = t >= layer.in_point.0.to_f64() && t < layer.out_point.0.to_f64();
        if !layer.switches.visible || !in_span || (any_solo && !layer.switches.solo) {
            continue;
        }
        if matches!(layer.kind, LayerKind::Camera { .. }) {
            continue; // folded in through camera_pose above
        }
        let lt = t - layer.start_offset.0.to_f64();
        feed_layer(h, doc, comp, layer, t, lt, quality, stamper, visited)?;
    }
    Some(())
}

/// Fold an effect stack into the frame key (docs/08): each live effect's
/// identity, version and evaluated parameters, plus the local time for seeded
/// and marker-driven effects (their pixels depend on time even when parameters
/// hold). Shared by a layer's own stack and — for an effects-and-masks matte or
/// depth input (K-142) — a referenced layer's stack, so editing that layer's
/// effects invalidates the consumer's cached frames. `lt` is the local time the
/// effects evaluate at (the referenced layer's own local time when hashing a
/// referenced layer); `t` is comp time, threaded through only for nested
/// layer-reference sources. `marker_layer` supplies the §1.4 marker context.
/// When `allow_after_effects_refs` is true, a Layer parameter whose source mode
/// (`EffectInstance::layer_source`) is `EffectsAndMasks` also folds the
/// referenced layer's own stack (K-142); the nested fold passes false, so a
/// referenced layer's own layer-refs stay source-only — bounding recursion to
/// one level and matching the v1 render, where a referenced layer's own
/// layer-inputs render as passthrough. Emits nothing when the fx switch is off
/// or no effect is enabled, so every pre-effects key stays valid.
#[allow(clippy::too_many_arguments)]
fn feed_effect_stack(
    h: &mut blake3::Hasher,
    fx_on: bool,
    effects: &[lumit_core::model::EffectInstance],
    marker_layer: &lumit_core::model::Layer,
    comp: &Composition,
    doc: &Document,
    t: f64,
    lt: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
    allow_after_effects_refs: bool,
) -> Option<()> {
    if !(fx_on && effects.iter().any(|e| e.enabled)) {
        return Some(());
    }
    h.update(b"effects/");
    // The §1.4 marker context, built lazily (only marker-driven effects read
    // it) by the same shared constructor resolution uses (K-031), so the key
    // hashes exactly the beat times resolution sees.
    let mut mctx: Option<lumit_core::fx::MarkerContext> = None;
    for e in effects.iter().filter(|e| e.enabled) {
        h.update(&[match e.effect.namespace {
            lumit_core::model::EffectNamespace::Builtin => 0,
            lumit_core::model::EffectNamespace::Ofx => 1,
            lumit_core::model::EffectNamespace::Lfx => 2,
            lumit_core::model::EffectNamespace::Placeholder => 3,
        }]);
        h.update(e.effect.match_name.as_bytes());
        h.update(&e.effect.version.to_le_bytes());
        // The per-effect temporal opt-out (K-132, docs/impl/temporal-rerender.md
        // §6): an effect flagged sample_temporally == false renders at the frame
        // time (not the held/sample time) inside a temporal re-render below a
        // Posterize/accumulation adjustment, so the flag is content. Feed only the
        // non-default (false), so ordinary keys — every effect sampling, the
        // overwhelming case — are unchanged, but toggling it off is never a stale
        // cache hit.
        if !e.sample_temporally {
            h.update(b"no-temporal-sample/");
        }
        for p in &e.params {
            h.update(p.id.as_bytes());
            use lumit_core::model::EffectValue;
            match &p.value {
                EffectValue::Float(v) => feed_f64(h, v.value_at(lt)),
                EffectValue::Point(x, y) => {
                    feed_f64(h, x.value_at(lt));
                    feed_f64(h, y.value_at(lt));
                }
                EffectValue::Colour(c) => {
                    for ch in c {
                        feed_f64(h, ch.value_at(lt));
                    }
                }
                EffectValue::Bool(b) => {
                    h.update(&[u8::from(*b)]);
                }
                EffectValue::Choice(v) | EffectValue::Seed(v) => {
                    h.update(&v.to_le_bytes());
                }
                EffectValue::File(f) => {
                    // Which file is live at this time (the hold-keyed index
                    // selects it); an unset param feeds a distinct 0 marker.
                    // The path string is hashed (length-prefixed), not the
                    // file's bytes — the same policy a footage source path
                    // follows. Refreshing a file edited on disk is the LUT
                    // loader's job (specified as path + mtime caching,
                    // docs/impl/lut.md §4; the shipped caches key by path
                    // only, so an on-disk edit shows after a restart).
                    match f.path_at(lt) {
                        Some(p) => {
                            h.update(&[1]);
                            h.update(&(p.len() as u64).to_le_bytes());
                            h.update(p.as_bytes());
                        }
                        None => {
                            h.update(&[0]);
                        }
                    }
                }
                EffectValue::Layer(lref) => {
                    // The referenced layer is rendered alone at comp size as
                    // this effect's auxiliary input (a depth pass for depth
                    // of field, docs/impl/layer-input.md §4): its content is
                    // content the parameter hash cannot otherwise see, so it
                    // joins the key. It is rendered SOURCE-ONLY (its own
                    // effect stack is not applied), so — exactly like a matte
                    // source — the key is the referenced layer's source plus
                    // its evaluated transform. A source-only render never
                    // re-enters an effect stack, so a layer reference cannot
                    // recurse; `visited` still guards any precomp cycle
                    // inside that source. An unset or dangling reference
                    // feeds a distinct 0 marker (the effect is a no-op).
                    match lref
                        .as_ref()
                        .and_then(|id| comp.layers.iter().find(|l| l.id == *id))
                    {
                        Some(src) => {
                            h.update(&[1]);
                            h.update(src.id.as_bytes());
                            let slt = t - src.start_offset.0.to_f64();
                            feed_source(h, doc, &src.kind, slt, quality, stamper, visited)?;
                            let dtr = &src.transform;
                            for v in [
                                dtr.position_x.value_at(slt),
                                dtr.position_y.value_at(slt),
                                dtr.position_z.value_at(slt),
                                dtr.anchor_x.value_at(slt),
                                dtr.anchor_y.value_at(slt),
                                dtr.scale_x.value_at(slt),
                                dtr.scale_y.value_at(slt),
                                dtr.rotation.value_at(slt),
                                dtr.rotation_x.value_at(slt),
                                dtr.rotation_y.value_at(slt),
                                dtr.opacity.value_at(slt),
                            ] {
                                feed_f64(h, v);
                            }
                            h.update(&[u8::from(src.switches.three_d)]);
                            // Layer-input source mode (K-142): None / Masks /
                            // Effects and masks. The discriminant is content
                            // (switching modes changes the sampled input), so it
                            // joins the key.
                            let mode = e.layer_source(&p.id);
                            h.update(&[mode.key_byte()]);
                            // Effects-and-masks input: the referenced layer is
                            // consumed *after* its own stack, so that stack is
                            // content. Fold it once — the nested call disables
                            // further effect-refs, so the source's own
                            // layer-inputs stay source-only, matching the render
                            // (they resolve to passthrough) and bounding recursion.
                            if allow_after_effects_refs && mode.folds_effects() {
                                h.update(b"ref-fx/");
                                feed_effect_stack(
                                    h,
                                    src.switches.fx,
                                    &src.effects,
                                    src,
                                    comp,
                                    doc,
                                    t,
                                    slt,
                                    quality,
                                    stamper,
                                    visited,
                                    false,
                                )?;
                            }
                        }
                        None => {
                            h.update(&[0]);
                        }
                    }
                }
            }
        }
        // A seeded effect (docs/08 §1.3 Randomness) draws from
        // hash(seed, time, …) generators (§2.4): its pixels are a
        // function of the layer's local time even while every
        // parameter holds constant — a Shake wobbles a static solid
        // differently every frame. The local time therefore joins the
        // key for exactly these effects; everything else keeps its
        // time-free keys (a blurred solid still shares one cached
        // frame across its whole span).
        if e.effect.namespace == lumit_core::model::EffectNamespace::Builtin {
            if let Some(s) = lumit_core::fx::schema(&e.effect.match_name) {
                if s.traits.seeded {
                    h.update(b"fx-time");
                    feed_f64(h, lt);
                }
                // A marker-driven effect (docs/08 §1.3 Marker input,
                // §1.4) reads beat markers the parameter hash cannot
                // see, so its key gains the layer's local time plus
                // the §1.4 window it consumes — the same trigger
                // times, through the same shared context constructor,
                // that resolution reads (K-031). The window, not the
                // whole marker list: a marker edit that cannot change
                // this frame's envelope leaves its key alone. A
                // Manual-mode Flash reports no window at all and keeps
                // its time-free keys — no over-invalidation.
                if s.traits.beat_input {
                    let ctx = mctx.get_or_insert_with(|| {
                        lumit_core::fx::MarkerContext::for_layer(comp, marker_layer)
                    });
                    if let Some(w) = lumit_core::fx::marker_window(e, lt, ctx) {
                        h.update(b"fx-markers");
                        feed_f64(h, lt);
                        feed_f64(h, w.fps);
                        for side in [w.before, w.after] {
                            match side {
                                Some(t) => {
                                    h.update(&[1]);
                                    feed_f64(h, t);
                                }
                                None => {
                                    h.update(&[0]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn feed_layer(
    h: &mut blake3::Hasher,
    doc: &Document,
    comp: &Composition,
    layer: &lumit_core::model::Layer,
    t: f64,
    lt: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    h.update(b"layer/");
    feed_source(h, doc, &layer.kind, lt, quality, stamper, visited)?;

    // Evaluated transform at the layer's local time — never keyframe data.
    let tr = &layer.transform;
    for v in [
        tr.position_x.value_at(lt),
        tr.position_y.value_at(lt),
        tr.position_z.value_at(lt),
        tr.anchor_x.value_at(lt),
        tr.anchor_y.value_at(lt),
        tr.scale_x.value_at(lt),
        tr.scale_y.value_at(lt),
        tr.rotation.value_at(lt),
        tr.rotation_x.value_at(lt),
        tr.rotation_y.value_at(lt),
        tr.opacity.value_at(lt),
    ] {
        feed_f64(h, v);
    }
    h.update(&[u8::from(layer.switches.three_d)]);
    h.update(&[blend_tag(layer.blend)]);
    // Collapse changes how a Precomp composites (docs/06 §1.4), so it is
    // content. Hashed only when set, so every pre-collapse key stays valid.
    if layer.switches.collapse && matches!(layer.kind, LayerKind::Precomp { .. }) {
        h.update(b"collapsed");
    }

    // Per-layer motion blur (docs/06 §4, K-120): the layer's switch is content
    // only while the comp master is on (else the layer renders normally). When
    // it blurs, the pixels are the layer's transform sampled across the open
    // shutter and averaged — so the evaluated sub-frame transforms join the
    // key. Two comp times sharing this frame's instantaneous transform but
    // differing in motion (a same-position/different-velocity frame) smear
    // differently and MUST key apart; hashing only the frame-time transform
    // would collide them (the K-093 lesson, applied to the shutter samples).
    // Hashed only when the layer actually blurs, so every non-blurring key
    // stays valid, and a static blurring layer (constant transform) hashes the
    // same constant samples across its span — no over-invalidation.
    if comp.motion_blur.enabled && layer.switches.motion_blur {
        let offsets = comp.motion_blur.sample_offsets();
        if !offsets.is_empty() {
            h.update(b"mblur-layer/");
            let dt = 1.0 / comp.frame_rate.fps().max(1.0);
            for off in offsets {
                let slt = lt + off * dt;
                for v in [
                    tr.position_x.value_at(slt),
                    tr.position_y.value_at(slt),
                    tr.position_z.value_at(slt),
                    tr.anchor_x.value_at(slt),
                    tr.anchor_y.value_at(slt),
                    tr.scale_x.value_at(slt),
                    tr.scale_y.value_at(slt),
                    tr.rotation.value_at(slt),
                    tr.rotation_x.value_at(slt),
                    tr.rotation_y.value_at(slt),
                ] {
                    feed_f64(h, v);
                }
            }
        }
    }

    // The effect stack (docs/08): each live effect's identity, version and
    // evaluated parameters are content — the version bump is what retires
    // cached frames when an effect's maths change (K-016). Hashed only when
    // a live stack exists, so every pre-effects key stays valid. A bypassed
    // effect (or an fx-switched-off layer) contributes nothing, exactly as
    // it renders nothing.
    feed_effect_stack(
        h,
        layer.switches.fx,
        &layer.effects,
        layer,
        comp,
        doc,
        t,
        lt,
        quality,
        stamper,
        visited,
        true,
    )?;

    // A temporal effect (echo, docs/08 §3.13) reads the layer's neighbour
    // source frames, which are content the parameter hash cannot see (K-094):
    // two comp times sharing the current frame — a held/frozen leading frame —
    // can differ in their neighbours, so their echoes differ. Key the stamped
    // neighbour frames, exactly the ones the render decodes (same window, same
    // retime mapping, same comp frame step). Footage layers only, matching the
    // render's neighbour decode; empty otherwise, so a plain layer's key is
    // untouched.
    if lumit_core::fx::stack_is_temporal(&layer.effects, layer.switches.fx) {
        if let LayerKind::Footage { item, retime } = &layer.kind {
            let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
            h.update(b"temporal/");
            for o in lumit_core::fx::stack_temporal_window(&layer.effects, layer.switches.fx)
                .into_iter()
                .filter(|&o| o != 0)
            {
                let nlt = lt + f64::from(o) * comp_dt;
                let nst = retime.as_ref().map(|r| r.evaluate(nlt)).unwrap_or(nlt);
                if let Some((identity, frame)) = stamper.stamp(*item, nst) {
                    h.update(&o.to_le_bytes());
                    h.update(identity.as_bytes());
                    h.update(&frame.to_le_bytes());
                }
            }
        }
    }

    // Masks: static paths are plain data (animated paths will evaluate here).
    if layer.masks.is_empty() {
        h.update(b"nomask");
    } else {
        h.update(b"masks");
        let json = serde_json::to_string(&layer.masks).unwrap_or_default();
        h.update(json.as_bytes());
    }

    // Matte: the matte source's content at this time, plus the mode flags.
    // The source layer's own visibility is irrelevant (mattes render alone).
    match &layer.matte {
        None => {
            h.update(b"nomatte");
        }
        Some(mr) => match comp.layers.iter().find(|l| l.id == mr.layer) {
            None => {
                h.update(b"nomatte"); // dangling ref degrades to no matte
            }
            Some(src) => {
                h.update(b"matte");
                h.update(&[
                    u8::from(matches!(mr.channel, MatteChannel::Luma)),
                    u8::from(mr.inverted),
                    // The three-way source mode (K-142): switching None / Masks /
                    // Effects and masks must retire stale frames, so the mode
                    // discriminant joins the key (replacing K-125's bool byte).
                    mr.source.key_byte(),
                ]);
                let mlt = t - src.start_offset.0.to_f64();
                feed_source(h, doc, &src.kind, mlt, quality, stamper, visited)?;
                let mtr = &src.transform;
                for v in [
                    mtr.position_x.value_at(mlt),
                    mtr.position_y.value_at(mlt),
                    mtr.position_z.value_at(mlt),
                    mtr.anchor_x.value_at(mlt),
                    mtr.anchor_y.value_at(mlt),
                    mtr.scale_x.value_at(mlt),
                    mtr.scale_y.value_at(mlt),
                    mtr.rotation.value_at(mlt),
                    mtr.rotation_x.value_at(mlt),
                    mtr.rotation_y.value_at(mlt),
                    mtr.opacity.value_at(mlt),
                ] {
                    feed_f64(h, v);
                }
                h.update(&[u8::from(src.switches.three_d)]);
                // Effects-and-masks matte (K-142): the matte gates by the
                // source's *processed* pixels, so the source's own effect stack
                // is content — fold it into the key at the source's local time,
                // the same way the source's own draw would. None / Masks leave
                // the key untouched (a bypassed source stack too).
                if mr.source.folds_effects() {
                    // false: the matte render runs the source's stack with no
                    // layer-inputs (v1), so the source's own layer-input refs
                    // are passthrough — don't fold them, matching the render.
                    feed_effect_stack(
                        h,
                        src.switches.fx,
                        &src.effects,
                        src,
                        comp,
                        doc,
                        t,
                        mlt,
                        quality,
                        stamper,
                        visited,
                        false,
                    )?;
                }
            }
        },
    }
    Some(())
}

/// Stable one-byte tag per blend mode (never reuse a value — the key must
/// not change meaning across versions without an ALGO_VERSION bump).
fn blend_tag(b: lumit_core::model::BlendMode) -> u8 {
    use lumit_core::model::BlendMode;
    match b {
        BlendMode::Normal => 0,
        BlendMode::Add => 1,
        BlendMode::Multiply => 2,
        BlendMode::Screen => 3,
        BlendMode::Overlay => 4,
        BlendMode::SoftLight => 5,
        BlendMode::HardLight => 6,
        BlendMode::Lighten => 7,
        BlendMode::Darken => 8,
        BlendMode::Subtract => 9,
        // The rest of the After Effects set (K-162, T24). Never reuse a tag —
        // the value is part of the frame-cache key.
        BlendMode::ColourBurn => 10,
        BlendMode::LinearBurn => 11,
        BlendMode::DarkerColour => 12,
        BlendMode::ColourDodge => 13,
        BlendMode::LighterColour => 14,
        BlendMode::LinearLight => 15,
        BlendMode::VividLight => 16,
        BlendMode::PinLight => 17,
        BlendMode::HardMix => 18,
        BlendMode::Difference => 19,
        BlendMode::Exclusion => 20,
        BlendMode::Divide => 21,
        BlendMode::Hue => 22,
        BlendMode::Saturation => 23,
        BlendMode::Colour => 24,
        BlendMode::Luminosity => 25,
    }
}

/// The layer's source pixels as content (docs/06 §5.2 "node type id ‖
/// algorithm version, evaluated parameters, key(inputs)").
fn feed_source(
    h: &mut blake3::Hasher,
    doc: &Document,
    kind: &LayerKind,
    lt: f64,
    quality: Quality,
    stamper: &dyn SourceStamper,
    visited: &mut Vec<Uuid>,
) -> Option<()> {
    match kind {
        LayerKind::Footage { item, retime } => {
            // The retime maps local time → source time; the cache key must key
            // the RETIMED source frame, so two different ramps never collide.
            let source_time = retime.as_ref().map(|r| r.evaluate(lt)).unwrap_or(lt);
            let (identity, frame) = stamper.stamp(*item, source_time)?;
            h.update(b"footage/");
            h.update(identity.as_bytes());
            h.update(&frame.to_le_bytes());
            // A non-Nearest interpolation policy synthesises different
            // in-between pixels (blend/flow, K-088), so it is content. Nearest
            // shows exactly the stamped frame — pixel-identical to no retime —
            // so it hashes nothing and those keys stay shared.
            //
            // The synthesised image depends on the *sub-frame position*, not
            // just the nearest integer frame: a flow ramp from source frame 39
            // to 40 crosses every fraction in between, and each fraction is a
            // different morph. `stamp` returns only the integer frame, so
            // without also hashing `source_time` every fraction across an
            // integer span collides onto one key and the cache holds a single
            // frame per span (K-093 — the "flow only changes once in the
            // middle" bug). Hashing the exact retimed time keys each fraction
            // distinctly; identical times reuse, so it never over-renders a
            // truly repeated position.
            if let Some(r) = retime {
                if !matches!(r.interpolation, lumit_core::retime::Interpolation::Nearest) {
                    h.update(&[interp_tag(&r.interpolation)]);
                    feed_f64(h, source_time);
                    // A flow conform rate (K-095) synthesises from different
                    // source frames at the same source time, so it is content.
                    if let lumit_core::retime::Interpolation::Flow(p) = &r.interpolation {
                        // The conform rate is keyframeable (K-160): hash the
                        // value it reads at this local time, so each rate along
                        // an animated ramp keys its own synthesised frame.
                        if let Some(fps) = p.input_fps_at(lt) {
                            h.update(b"conform");
                            feed_f64(h, fps);
                        }
                    }
                }
            }
        }
        LayerKind::Solid { def } => match doc.solid(*def) {
            None => {
                h.update(b"nosolid"); // deleted def renders as nothing
            }
            Some(sd) => {
                h.update(b"solid/");
                for c in sd.colour.0 {
                    h.update(&c.to_le_bytes());
                }
                h.update(&sd.width.to_le_bytes());
                h.update(&sd.height.to_le_bytes());
            }
        },
        LayerKind::Text { document } => {
            h.update(b"text/");
            h.update(document.text.as_bytes());
            h.update(&[0]); // length delimiter: text then size never collide
            feed_f64(h, document.size);
            for c in document.fill.0 {
                h.update(&c.to_le_bytes());
            }
        }
        LayerKind::Precomp { comp } => {
            if visited.contains(comp) {
                h.update(b"cycle"); // renders as nothing, matches the pipeline
                return Some(());
            }
            let Some(nested) = doc.comp(*comp) else {
                h.update(b"nocomp");
                return Some(());
            };
            h.update(b"precomp/");
            visited.push(*comp);
            let r = feed_comp(h, doc, nested, lt, quality, stamper, visited);
            visited.pop();
            r?;
        }
        LayerKind::Camera { .. } => {
            h.update(b"camera"); // draws nothing; pose is hashed at comp level
        }
        LayerKind::Sequence { clips } => {
            // Key the active clip's resolved source (docs/04-RETIMING.md §1.3):
            // a gap is transparent, a footage clip keys its retimed source
            // frame, a comp clip recurses.
            match lumit_core::sequence::resolve(clips, lt) {
                None => {
                    h.update(b"gap");
                }
                Some((_id, lumit_core::sequence::ClipSource::Footage(item), st)) => {
                    let (identity, frame) = stamper.stamp(item, st)?;
                    h.update(b"seq-footage/");
                    h.update(identity.as_bytes());
                    h.update(&frame.to_le_bytes());
                    if let Some(clip) = lumit_core::sequence::active_clip(clips, lt) {
                        if !matches!(
                            clip.interpolation,
                            lumit_core::retime::Interpolation::Nearest
                        ) {
                            // The sub-frame position is content under blend/flow
                            // (see the Footage case above, K-093).
                            h.update(&[interp_tag(&clip.interpolation)]);
                            feed_f64(h, st);
                            if let lumit_core::retime::Interpolation::Flow(p) = &clip.interpolation
                            {
                                // Keyframeable conform rate (K-160), read at the
                                // clip's layer-local time like the footage case.
                                if let Some(fps) = p.input_fps_at(lt) {
                                    h.update(b"conform");
                                    feed_f64(h, fps);
                                }
                            }
                        }
                    }
                }
                Some((_id, lumit_core::sequence::ClipSource::Comp(comp), st)) => {
                    if visited.contains(&comp) {
                        h.update(b"cycle");
                        return Some(());
                    }
                    let Some(nested) = doc.comp(comp) else {
                        h.update(b"nocomp");
                        return Some(());
                    };
                    h.update(b"seq-comp/");
                    visited.push(comp);
                    let r = feed_comp(h, doc, nested, st, quality, stamper, visited);
                    visited.pop();
                    r?;
                }
            }
        }
        LayerKind::Adjustment => {
            // No source of its own; its live effect stack, masks, transform
            // and opacity — the staging inputs (docs/06 §1.5) — are all
            // hashed at the layer level like any other layer's.
            h.update(b"adjust");
        }
    }
    Some(())
}

/// Stable one-byte tag per frame-interpolation policy (never reuse a value).
fn interp_tag(i: &lumit_core::retime::Interpolation) -> u8 {
    use lumit_core::retime::Interpolation;
    match i {
        Interpolation::Nearest => 1,
        Interpolation::Blend => 2,
        Interpolation::Flow(p) if p.half_resolution => 3,
        Interpolation::Flow(_) => 4,
    }
}

fn feed_f64(h: &mut blake3::Hasher, v: f64) {
    // Canonicalise the one f64 equality wrinkle: 0.0 and -0.0 render alike.
    let v = if v == 0.0 { 0.0 } else { v };
    h.update(&v.to_bits().to_le_bytes());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::model::{
        Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
        TextDocument, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

    struct StubStamper;
    impl SourceStamper for StubStamper {
        fn stamp(&self, item: Uuid, lt: f64) -> Option<(String, u64)> {
            Some((format!("stub:{item}"), (lt * 60.0).round().max(0.0) as u64))
        }
    }

    struct UnknownStamper;
    impl SourceStamper for UnknownStamper {
        fn stamp(&self, _item: Uuid, _lt: f64) -> Option<(String, u64)> {
            None
        }
    }

    fn secs(s: f64) -> CompTime {
        CompTime(Rational::from_f64_on_grid(s, Rational::FLICK_DEN).unwrap())
    }

    fn text_layer(text: &str, in_s: f64, out_s: f64, offset_s: f64) -> Layer {
        Layer {
            id: Uuid::now_v7(),
            name: "t".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: text.into(),
                    size: 72.0,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            in_point: secs(in_s),
            out_point: secs(out_s),
            start_offset: secs(offset_s),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn comp_with(layers: Vec<Layer>) -> Composition {
        Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn key(doc: &Document, comp: &Composition, t: f64) -> FrameKey {
        comp_frame_key(doc, comp, t, Quality::default(), &StubStamper).unwrap()
    }

    /// Same content, different instance ids and names → the same key. This
    /// is the Global Performance Cache lesson made a test: identity never
    /// feeds the hash, so a duplicated comp shares its original's cache.
    #[test]
    fn identical_content_hashes_identically_across_instances() {
        let doc = Document::new();
        let a = comp_with(vec![text_layer("hello", 0.0, 5.0, 0.0)]);
        let mut b = comp_with(vec![text_layer("hello", 0.0, 5.0, 0.0)]);
        b.name = "entirely different name".into();
        assert_eq!(key(&doc, &a, 1.0), key(&doc, &b, 1.0));
        // And deterministic across calls.
        assert_eq!(key(&doc, &a, 1.0), key(&doc, &a, 1.0));
    }

    /// Solo changes which layers render (K-105), so it must change the key —
    /// the RAM/disk frame cache would otherwise replay the pre-solo picture.
    /// Gated like visibility, never hashed: soloing every contributing layer
    /// draws the same picture, so those keys (and caches) are shared.
    #[test]
    fn solo_gates_layers_out_of_the_key() {
        let doc = Document::new();
        let mut comp = comp_with(vec![
            text_layer("top", 0.0, 5.0, 0.0),
            text_layer("under", 0.0, 5.0, 0.0),
        ]);
        let both = key(&doc, &comp, 1.0);
        comp.layers[0].switches.solo = true;
        assert_ne!(
            both,
            key(&doc, &comp, 1.0),
            "solo mutes the other layer, so the two-layer key must retire"
        );
        comp.layers[1].switches.solo = true;
        assert_eq!(both, key(&doc, &comp, 1.0));
    }

    /// Timeline position is not content: sliding a static layer's span and
    /// offset together keeps every overlapping frame's key.
    #[test]
    fn time_shifted_static_layer_keeps_its_keys() {
        let doc = Document::new();
        let a = comp_with(vec![text_layer("hi", 0.0, 8.0, 0.0)]);
        // Same layer slid 2 s later (span and offset together): at t+2 the
        // local time matches, so the pixels match, so the key must match.
        let b = comp_with(vec![text_layer("hi", 2.0, 10.0, 2.0)]);
        assert_eq!(key(&doc, &a, 1.0), key(&doc, &b, 3.0));
        // Trimming the bar without crossing t changes nothing either.
        let c = comp_with(vec![text_layer("hi", 0.0, 3.0, 0.0)]);
        assert_eq!(key(&doc, &a, 1.0), key(&doc, &c, 1.0));
    }

    /// GEN-3 (K-153): a layer may sit across the comp boundaries — starting
    /// before comp time 0 or ending past the comp duration. Only the portion
    /// overlapping [0, comp_end) is ever sampled, so the out-of-window head or
    /// tail changes no rendered frame. In/out points gate presence; they never
    /// feed the hash, and rendered frames only exist within the comp window.
    #[test]
    fn layers_crossing_comp_bounds_render_only_the_in_window_overlap() {
        let doc = Document::new();
        // Negative start: in = -3, out = 2, offset 0 — the head before comp 0 is
        // never sampled, so across [0, 2) every rendered frame keys identically
        // to the same content placed exactly at 0.
        let crossing = comp_with(vec![text_layer("x", -3.0, 2.0, 0.0)]);
        let in_window = comp_with(vec![text_layer("x", 0.0, 2.0, 0.0)]);
        for t in [0.0, 0.5, 1.9] {
            assert_eq!(key(&doc, &crossing, t), key(&doc, &in_window, t));
        }
        // Past its out point the crossing layer is gone (empty-comp key), just
        // like the in-window one — a negative start does not extend the tail.
        let empty = comp_with(vec![]);
        assert_eq!(key(&doc, &crossing, 2.0), key(&doc, &empty, 2.0));

        // Ending past comp end: out = 20 while the comp is 10 s long. It is
        // present at the last comp frames and keys like a layer trimmed exactly
        // to the comp end, because frames only exist within [0, comp_end).
        let over_end = comp_with(vec![text_layer("y", 0.0, 20.0, 0.0)]);
        let to_end = comp_with(vec![text_layer("y", 0.0, 10.0, 0.0)]);
        for t in [8.0, 9.5] {
            assert_eq!(key(&doc, &over_end, t), key(&doc, &to_end, t));
        }
    }

    /// Evaluated values, not keyframe data: adding a keyframe beyond the
    /// evaluated point (linear span unchanged before it) keeps the key;
    /// changing the value at the point changes it.
    #[test]
    fn keyframe_edits_only_invalidate_where_values_change() {
        let doc = Document::new();
        let keyed = |keys: Vec<(f64, f64)>| {
            let mut l = text_layer("k", 0.0, 10.0, 0.0);
            l.transform.opacity = Property {
                extra: serde_json::Map::new(),
                animation: Animation::Keyframed(
                    keys.iter()
                        .map(|(t, v)| Keyframe {
                            time: Rational::from_f64_on_grid(*t, Rational::FLICK_DEN).unwrap(),
                            value: *v,
                            interp_in: SideInterp::Linear,
                            interp_out: SideInterp::Linear,
                        })
                        .collect(),
                ),
            };
            comp_with(vec![l])
        };
        let two = keyed(vec![(0.0, 0.0), (2.0, 100.0)]);
        let three = keyed(vec![(0.0, 0.0), (2.0, 100.0), (6.0, 40.0)]);
        // At t=1 the evaluated opacity is 50 either way → same key.
        assert_eq!(key(&doc, &two, 1.0), key(&doc, &three, 1.0));
        // At t=4 the extra keyframe changes the value → different key.
        assert_ne!(key(&doc, &two, 4.0), key(&doc, &three, 4.0));
    }

    /// Every content axis moves the key: text, solid colour (through the
    /// def), blend, 3D switch, camera, masks, quality tier.
    #[test]
    fn content_edits_change_the_key() {
        let mut doc = Document::new();
        let def_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def_id,
            name: "s".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        }));
        let mut solid = text_layer("x", 0.0, 5.0, 0.0);
        solid.kind = LayerKind::Solid { def: def_id };
        let comp = comp_with(vec![solid]);
        let base = key(&doc, &comp, 1.0);

        // Solid def colour edit reaches every layer using it.
        let mut doc2 = doc.clone();
        if let Some(ProjectItem::Solid(s)) = doc2.item_mut(def_id) {
            s.colour = LinearColour([0.5, 0.5, 0.5, 1.0]);
        }
        assert_ne!(base, key(&doc2, &comp, 1.0));

        // Blend mode.
        let mut c2 = comp.clone();
        c2.layers[0].blend = lumit_core::model::BlendMode::Screen;
        assert_ne!(base, key(&doc, &c2, 1.0));

        // 3D switch.
        let mut c3 = comp.clone();
        c3.layers[0].switches.three_d = true;
        assert_ne!(base, key(&doc, &c3, 1.0));

        // A camera layer above (the pose hashes even while every layer is
        // still 2D — flat vs cam is a content distinction).
        let mut c4 = comp.clone();
        c4.layers.insert(
            0,
            Layer {
                kind: LayerKind::Camera {
                    zoom: Property::fixed(1000.0),
                },
                ..text_layer("", 0.0, 5.0, 0.0)
            },
        );
        assert_ne!(base, key(&doc, &c4, 1.0));

        // Mask.
        let mut c5 = comp.clone();
        c5.layers[0]
            .masks
            .push(lumit_core::mask::Mask::rectangle(0.0, 0.0, 10.0, 10.0));
        assert_ne!(base, key(&doc, &c5, 1.0));

        // Quality tier.
        let half = comp_frame_key(&doc, &comp, 1.0, Quality { divisor: 2 }, &StubStamper);
        assert_ne!(Some(base), half);
    }

    /// The collapse switch is content (docs/06 §1.4 — it changes how a
    /// Precomp composites), so toggling it changes the key; and because it
    /// hashes only when set, every pre-collapse key stays valid.
    #[test]
    fn collapse_switch_changes_a_precomp_layers_key() {
        let mut doc = Document::new();
        let nested = comp_with(vec![text_layer("inner", 0.0, 10.0, 0.0)]);
        let nested_id = nested.id;
        doc.items.push(ProjectItem::Composition(nested));
        let mut pre = text_layer("", 0.0, 10.0, 0.0);
        pre.kind = LayerKind::Precomp { comp: nested_id };
        let parent = comp_with(vec![pre]);
        let base = key(&doc, &parent, 1.0);

        let mut collapsed = parent.clone();
        collapsed.layers[0].switches.collapse = true;
        assert_ne!(base, key(&doc, &collapsed, 1.0));

        // On a non-Precomp layer the switch is inert and never hashed.
        let plain = comp_with(vec![text_layer("t", 0.0, 10.0, 0.0)]);
        let mut plain_flagged = plain.clone();
        plain_flagged.layers[0].switches.collapse = true;
        assert_eq!(key(&doc, &plain, 1.0), key(&doc, &plain_flagged, 1.0));
    }

    /// The effect stack is content (docs/08): adding a live effect changes
    /// the key, its evaluated params move it per frame, bypass and the fx
    /// switch remove it — and a stack-free layer keys exactly as before.
    #[test]
    fn effect_stacks_feed_the_key_only_while_live() {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        use lumit_core::model::{
            EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue,
        };
        let doc = Document::new();
        let plain = comp_with(vec![text_layer("fx", 0.0, 10.0, 0.0)]);
        let base = key(&doc, &plain, 1.0);

        let glow = |radius: lumit_core::anim::Property| EffectInstance {
            id: Uuid::now_v7(),
            effect: EffectKey {
                namespace: EffectNamespace::Builtin,
                match_name: "glow".into(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: vec![EffectParam {
                id: "radius".into(),
                value: EffectValue::Float(radius),
                extra: serde_json::Map::new(),
            }],
            sample_temporally: true,
            extra: serde_json::Map::new(),
        };
        let mut with_fx = plain.clone();
        with_fx.layers[0]
            .effects
            .push(glow(lumit_core::anim::Property::fixed(24.0)));
        let fx_key = key(&doc, &with_fx, 1.0);
        assert_ne!(base, fx_key);

        // Evaluated params, not keyframe data: an animated radius moves the
        // key across frames, and matches the constant where values agree.
        let mut animated = plain.clone();
        let keys = vec![
            Keyframe {
                time: Rational::new(0, 1).unwrap(),
                value: 24.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            Keyframe {
                time: Rational::new(2, 1).unwrap(),
                value: 24.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            Keyframe {
                time: Rational::new(4, 1).unwrap(),
                value: 80.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
        ];
        animated.layers[0]
            .effects
            .push(glow(lumit_core::anim::Property {
                animation: Animation::Keyframed(keys),
                extra: serde_json::Map::new(),
            }));
        assert_eq!(key(&doc, &animated, 1.0), fx_key); // radius 24 either way
        assert_ne!(key(&doc, &animated, 3.0), fx_key); // mid-ramp differs

        // Bypass and the fx switch both return the pre-effects key exactly.
        let mut bypassed = with_fx.clone();
        bypassed.layers[0].effects[0].enabled = false;
        assert_eq!(base, key(&doc, &bypassed, 1.0));
        let mut fx_off = with_fx.clone();
        fx_off.layers[0].switches.fx = false;
        assert_eq!(base, key(&doc, &fx_off, 1.0));
        // A version bump retires the old key (K-016).
        let mut v2 = with_fx.clone();
        v2.layers[0].effects[0].effect.version = 2;
        assert_ne!(fx_key, key(&doc, &v2, 1.0));

        // The per-effect temporal opt-out (K-132) is content: turning
        // sample_temporally off changes the frame under a temporal re-render, so
        // it changes the key; the default (on) keys exactly as the flagless case.
        let mut opt_out = with_fx.clone();
        opt_out.layers[0].effects[0].sample_temporally = false;
        assert_ne!(fx_key, key(&doc, &opt_out, 1.0));
        let mut still_on = with_fx.clone();
        still_on.layers[0].effects[0].sample_temporally = true;
        assert_eq!(fx_key, key(&doc, &still_on, 1.0));
    }

    /// An after-effects matte (K-decision) gates by the source's *processed*
    /// pixels, so the source's effect stack becomes content for the consumer's
    /// key; a source-only matte (the default) ignores it entirely. The source
    /// is hidden here, so its stack can only reach the key through the matte —
    /// isolating the matte path from the source's own layer contribution.
    #[test]
    fn effects_and_masks_matte_keys_on_the_source_stack() {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        use lumit_core::model::{
            EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, LayerInputSource,
            MatteChannel, MatteRef,
        };
        use lumit_core::time::Rational;
        let doc = Document::new();
        let mut source = text_layer("m", 0.0, 10.0, 0.0);
        source.switches.visible = false; // matte-only: renders alone, not as a layer
        let source_id = source.id;
        let mut consumer = text_layer("c", 0.0, 10.0, 0.0);
        consumer.matte = Some(MatteRef {
            layer: source_id,
            channel: MatteChannel::Alpha,
            inverted: false,
            source: LayerInputSource::None,
        });
        let comp = comp_with(vec![consumer, source]);
        let base = key(&doc, &comp, 1.0);

        let glow = |radius: lumit_core::anim::Property| EffectInstance {
            id: Uuid::now_v7(),
            effect: EffectKey {
                namespace: EffectNamespace::Builtin,
                match_name: "glow".into(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: vec![EffectParam {
                id: "radius".into(),
                value: EffectValue::Float(radius),
                extra: serde_json::Map::new(),
            }],
            sample_temporally: true,
            extra: serde_json::Map::new(),
        };

        // None / Masks matte: the source's stack is not content — adding an
        // effect to the hidden source leaves the consumer's key alone.
        let mut src_fx = comp.clone();
        src_fx.layers[1]
            .effects
            .push(glow(lumit_core::anim::Property::fixed(24.0)));
        assert_eq!(
            base,
            key(&doc, &src_fx, 1.0),
            "a None/Masks matte ignores the source's effect stack"
        );

        // The mode itself is content: each of None / Masks / Effects and masks
        // keys apart — a mode discriminant that changes the sampled pixels.
        let mut masks = comp.clone();
        masks.layers[0].matte.as_mut().unwrap().source = LayerInputSource::Masks;
        assert_ne!(
            base,
            key(&doc, &masks, 1.0),
            "Masks differs from None (masks now gate the matte)"
        );
        let mut flag_only = comp.clone();
        flag_only.layers[0].matte.as_mut().unwrap().source = LayerInputSource::EffectsAndMasks;
        assert_ne!(
            base,
            key(&doc, &flag_only, 1.0),
            "Effects and masks differs from None even with no source stack"
        );
        assert_ne!(key(&doc, &masks, 1.0), key(&doc, &flag_only, 1.0));

        // Effects-and-masks matte with a stack: the source's effects fold into
        // the key, so it differs from both the mode-only keys.
        let mut after = src_fx.clone();
        after.layers[0].matte.as_mut().unwrap().source = LayerInputSource::EffectsAndMasks;
        let after_key = key(&doc, &after, 1.0);
        assert_ne!(base, after_key);
        assert_ne!(key(&doc, &flag_only, 1.0), after_key);

        // Evaluated params, not keyframe data: an animated source radius moves
        // the effects-and-masks key across frames.
        let mut animated = comp.clone();
        animated.layers[0].matte.as_mut().unwrap().source = LayerInputSource::EffectsAndMasks;
        animated.layers[1]
            .effects
            .push(glow(lumit_core::anim::Property {
                animation: Animation::Keyframed(vec![
                    Keyframe {
                        time: Rational::new(0, 1).unwrap(),
                        value: 24.0,
                        interp_in: SideInterp::Linear,
                        interp_out: SideInterp::Linear,
                    },
                    Keyframe {
                        time: Rational::new(4, 1).unwrap(),
                        value: 80.0,
                        interp_in: SideInterp::Linear,
                        interp_out: SideInterp::Linear,
                    },
                ]),
                extra: serde_json::Map::new(),
            }));
        assert_ne!(key(&doc, &animated, 1.0), key(&doc, &animated, 3.0));

        // Bypassing the source stack (or its fx switch) returns to source-only.
        let mut bypassed = after.clone();
        bypassed.layers[1].effects[0].enabled = false;
        assert_eq!(
            key(&doc, &flag_only, 1.0),
            key(&doc, &bypassed, 1.0),
            "a bypassed source stack contributes nothing, like the flag alone"
        );
    }

    /// An Effects-and-masks DoF depth input (K-142) folds the depth layer's own
    /// stack into the consumer's key, so grading the depth pass invalidates its
    /// cached frames; None/Masks ignore it. The depth layer is hidden, so its
    /// stack reaches the key only through the depth reference. A project saved
    /// with K-125's legacy `depth_after_effects` bool keys the same as the
    /// current `depth_source` Choice, so old caches migrate cleanly.
    #[test]
    fn effects_and_masks_depth_input_keys_on_the_source_stack() {
        use lumit_core::model::{EffectParam, EffectValue, LayerInputSource};
        let doc = Document::new();
        let mut depth = text_layer("d", 0.0, 10.0, 0.0);
        depth.switches.visible = false; // depth pass: referenced, not rendered as a layer
        let depth_id = depth.id;
        let mut consumer = text_layer("c", 0.0, 10.0, 0.0);
        let mut dof = lumit_core::fx::instantiate("dof").unwrap();
        for p in &mut dof.params {
            if p.id == "depth" {
                p.value = EffectValue::Layer(Some(depth_id));
            }
        }
        consumer.effects.push(dof);
        let comp = comp_with(vec![consumer, depth]);
        // Set (replacing any existing) the DoF depth_source on the consumer.
        let with_source = |comp: &Composition, mode: LayerInputSource| {
            let mut c = comp.clone();
            let params = &mut c.layers[0].effects[0].params;
            params.retain(|p| p.id != "depth_source");
            params.push(EffectParam {
                id: "depth_source".into(),
                value: EffectValue::Choice(mode.to_choice()),
                extra: serde_json::Map::new(),
            });
            c
        };
        let blur = || lumit_core::fx::instantiate("blur").unwrap();

        // Depth source None (set explicitly — the default is now Effects and
        // masks, K-142 follow-up): the depth is read source-only, so a blur on
        // the hidden depth layer leaves the consumer's key untouched.
        let none_comp = with_source(&comp, LayerInputSource::None);
        let base = key(&doc, &none_comp, 1.0);
        let mut none_blur = none_comp.clone();
        none_blur.layers[1].effects.push(blur());
        assert_eq!(
            base,
            key(&doc, &none_blur, 1.0),
            "a None depth input ignores the depth layer's stack"
        );

        // Effects and masks: the depth layer's stack folds in, so the same blur
        // now changes the consumer's key.
        let after = {
            let mut c = with_source(&comp, LayerInputSource::EffectsAndMasks);
            c.layers[1].effects.push(blur());
            c
        };
        assert_ne!(base, key(&doc, &after, 1.0));

        // The mode itself is content even with no depth stack. None / Masks /
        // Effects and masks each key apart from one another.
        let masks = with_source(&comp, LayerInputSource::Masks);
        let flag_only = with_source(&comp, LayerInputSource::EffectsAndMasks);
        assert_ne!(base, key(&doc, &masks, 1.0));
        assert_ne!(base, key(&doc, &flag_only, 1.0));
        assert_ne!(key(&doc, &masks, 1.0), key(&doc, &flag_only, 1.0));
        assert_ne!(key(&doc, &flag_only, 1.0), key(&doc, &after, 1.0));

        // Legacy K-125 bool on a project with NO depth_source: it reads as
        // Effects and masks through `layer_source`'s fallback, so the depth
        // layer's stack still folds. Proven by removing the depth stack: with
        // the bool on, that removal changes the key.
        let legacy_bool = |comp: &Composition| {
            let mut c = comp.clone();
            c.layers[0].effects[0].params.push(EffectParam {
                id: "depth_after_effects".into(),
                value: EffectValue::Bool(true),
                extra: serde_json::Map::new(),
            });
            c
        };
        let mut legacy_src = comp.clone();
        legacy_src.layers[1].effects.push(blur());
        let legacy = legacy_bool(&legacy_src);
        let legacy_no_stack = legacy_bool(&comp);
        assert_ne!(
            key(&doc, &legacy, 1.0),
            key(&doc, &legacy_no_stack, 1.0),
            "the legacy after-effects bool still folds the depth layer's stack"
        );
    }

    /// A seeded effect's pixels move with time under constant parameters
    /// (docs/08 §1.3 Randomness, §2.4), so the layer's local time joins its
    /// frame key: a shaken static solid keys differently at different
    /// frames (else every frame would collide on the first render), keys
    /// identically for the same frame twice, and a non-temporal effect
    /// (blur) keeps its time-free keys — no cache regression elsewhere.
    #[test]
    fn seeded_effects_key_the_local_time() {
        let mut doc = Document::new();
        let def_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def_id,
            name: "s".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        }));
        let with_fx = |name: &str| {
            let mut l = text_layer("x", 0.0, 10.0, 0.0);
            l.kind = LayerKind::Solid { def: def_id };
            l.effects.push(lumit_core::fx::instantiate(name).unwrap());
            comp_with(vec![l])
        };

        // Shake (seeded): different frames, different keys; the same frame
        // twice, the same key.
        let shaken = with_fx("shake");
        assert_ne!(key(&doc, &shaken, 1.0), key(&doc, &shaken, 2.0));
        assert_eq!(key(&doc, &shaken, 1.0), key(&doc, &shaken, 1.0));

        // Block glitch (also seeded, docs/08 §3.12): the same guarantee — a
        // glitched static solid keys differently across frames (its block
        // hash reads the local time-derived tick, §3.12 status note) and
        // identically for the same frame twice.
        let glitched = with_fx("block_glitch");
        assert_ne!(key(&doc, &glitched, 1.0), key(&doc, &glitched, 2.0));
        assert_eq!(key(&doc, &glitched, 1.0), key(&doc, &glitched, 1.0));

        // Blur (not seeded): a static solid keeps one key across frames.
        let blurred = with_fx("blur");
        assert_eq!(key(&doc, &blurred, 1.0), key(&doc, &blurred, 2.0));

        // And the seed itself is content: two Shakes differing only by
        // seed key apart (the params already hash — pinned here so the
        // Seed value kind never falls out of the loop).
        let mut reseeded = shaken.clone();
        for p in &mut reseeded.layers[0].effects[0].params {
            if p.id == "seed" {
                use lumit_core::model::EffectValue;
                let old = match p.value {
                    EffectValue::Seed(s) => s,
                    _ => 0,
                };
                p.value = EffectValue::Seed(old.wrapping_add(1));
            }
        }
        assert_ne!(key(&doc, &shaken, 1.0), key(&doc, &reseeded, 1.0));
    }

    /// A marker-driven Flash (docs/08 §1.3 Marker input, §1.4) keys the
    /// layer's local time plus the window of triggers its envelope reads —
    /// and nothing more. On-beat and off-beat frames key apart; the same
    /// frame keys identically twice; a marker outside the frame's §1.4
    /// window moves no key, while one inside it does; and a Manual-mode
    /// Flash keeps fully time-free keys even with beat markers present —
    /// no over-invalidation.
    #[test]
    fn marker_driven_flash_keys_the_window_it_reads() {
        use lumit_core::markers::Marker;
        use lumit_core::model::{EffectValue, ProjectItem, SolidDef};
        let mut doc = Document::new();
        let def_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def_id,
            name: "s".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        }));
        let beat = |s: i64| Marker::beat(Uuid::now_v7(), Rational::new(s, 1).unwrap(), 0.9);
        let flashed = |mode: u32| {
            let mut l = text_layer("x", 0.0, 10.0, 0.0);
            l.kind = LayerKind::Solid { def: def_id };
            let mut fx = lumit_core::fx::instantiate("flash").unwrap();
            for p in &mut fx.params {
                if p.id == "mode" {
                    p.value = EffectValue::Choice(mode);
                }
            }
            l.effects.push(fx);
            let mut c = comp_with(vec![l]);
            c.markers = vec![beat(1), beat(2)];
            c
        };

        // Trigger mode: on-beat and off-beat frames key apart, the same
        // frame twice keys identically.
        let comp = flashed(1);
        assert_ne!(key(&doc, &comp, 1.0), key(&doc, &comp, 1.5));
        assert_eq!(key(&doc, &comp, 1.5), key(&doc, &comp, 1.5));

        // A far-away beat outside the frame's window (before 1.0 and 2.0,
        // both nearer) leaves the key alone; a beat inside it retires the
        // frame.
        let mut far = comp.clone();
        far.markers.push(beat(9));
        assert_eq!(key(&doc, &comp, 1.2), key(&doc, &far, 1.2));
        let mut near = comp.clone();
        near.markers.push(Marker::beat(
            Uuid::now_v7(),
            Rational::new(11, 10).unwrap(),
            0.9,
        ));
        assert_ne!(key(&doc, &comp, 1.2), key(&doc, &near, 1.2));

        // Manual mode: time-free keys, beat markers or none.
        let manual = flashed(0);
        assert_eq!(key(&doc, &manual, 1.0), key(&doc, &manual, 2.0));
    }

    /// Strobe's Nth-beat indexing counts from the comp's first beat, so a
    /// beat added far in the past can change which beat fires *now*. The
    /// key hashes the filtered triggers the envelope actually consumes, so
    /// that edit retires the frame — hashing raw neighbours would miss it.
    #[test]
    fn strobe_reindexing_retires_the_frame() {
        use lumit_core::markers::Marker;
        use lumit_core::model::{EffectValue, ProjectItem, SolidDef};
        let mut doc = Document::new();
        let def_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def_id,
            name: "s".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        }));
        let beat = |s: i64| Marker::beat(Uuid::now_v7(), Rational::new(s, 1).unwrap(), 0.9);
        let mut l = text_layer("x", 0.0, 10.0, 0.0);
        l.kind = LayerKind::Solid { def: def_id };
        let mut fx = lumit_core::fx::instantiate("flash").unwrap();
        for p in &mut fx.params {
            match p.id.as_str() {
                "mode" => p.value = EffectValue::Choice(2),
                "every_nth" => p.value = EffectValue::Float(lumit_core::anim::Property::fixed(2.0)),
                _ => {}
            }
        }
        l.effects.push(fx);
        let mut comp = comp_with(vec![l]);
        // Beats at 4 s and 6 s; every 2nd fires index 0 only (4 s), so just
        // after 6 s the flash is long spent.
        comp.markers = vec![beat(4), beat(6)];
        let before = key(&doc, &comp, 6.02);
        // A beat at 0 s re-indexes: now 0 s and 6 s fire, so the same frame
        // sits one frame into a live flash — its key must move.
        let mut reindexed = comp.clone();
        reindexed.markers.insert(0, beat(0));
        assert_ne!(before, key(&doc, &reindexed, 6.02));
    }

    /// Precomps recurse: an edit inside the nested comp changes the parent's
    /// key.
    #[test]
    fn precomp_edits_propagate_to_parents() {
        let mut doc = Document::new();
        let nested = comp_with(vec![text_layer("inner", 0.0, 10.0, 0.0)]);
        let nested_id = nested.id;
        doc.items.push(ProjectItem::Composition(nested));
        let mut pre = text_layer("", 0.0, 10.0, 0.0);
        pre.kind = LayerKind::Precomp { comp: nested_id };
        let parent = comp_with(vec![pre]);
        let base = key(&doc, &parent, 1.0);

        let mut doc2 = doc.clone();
        if let Some(c) = doc2.comp_mut(nested_id) {
            if let LayerKind::Text { document } = &mut c.layers[0].kind {
                document.text = "changed".into();
            }
        }
        assert_ne!(base, key(&doc2, &parent, 1.0));
    }

    /// Unprobed footage → unkeyable (None), never a wrong key.
    #[test]
    fn unknown_footage_makes_the_frame_unkeyable() {
        let doc = Document::new();
        let mut l = text_layer("", 0.0, 5.0, 0.0);
        l.kind = LayerKind::Footage {
            item: Uuid::now_v7(),
            retime: None,
        };
        let comp = comp_with(vec![l]);
        assert!(comp_frame_key(&doc, &comp, 1.0, Quality::default(), &UnknownStamper).is_none());
        assert!(comp_frame_key(&doc, &comp, 1.0, Quality::default(), &StubStamper).is_some());
    }

    /// A retime keys the RETIMED source frame: half-speed at t=2 must key the
    /// same frame as no-retime at t=1 (both source time 1), and differ from
    /// no-retime at t=2.
    #[test]
    fn retime_keys_the_source_frame_not_the_local_frame() {
        use lumit_core::retime::Retime;
        use lumit_core::time::Rational;
        let doc = Document::new();
        let item = Uuid::now_v7();
        let footage = |retime| {
            let mut l = text_layer("", 0.0, 10.0, 0.0);
            l.kind = LayerKind::Footage { item, retime };
            comp_with(vec![l])
        };
        let plain = footage(None);
        let half = footage(Some(Retime::constant_speed(
            Rational::new(10, 1).unwrap(),
            Rational::ZERO,
            Rational::new(1, 2).unwrap(),
        )));
        let k = |c: &Composition, t| {
            comp_frame_key(&doc, c, t, Quality::default(), &StubStamper).unwrap()
        };
        // half-speed at t=2 (source 1.0) == plain at t=1 (source 1.0).
        assert_eq!(k(&half, 2.0), k(&plain, 1.0));
        // and differs from plain at t=2 (source 2.0).
        assert_ne!(k(&half, 2.0), k(&plain, 2.0));
    }

    /// The frame-interpolation policy is content only when it synthesises
    /// (K-088): Nearest keys identically to no retime at the same source
    /// frame, while Blend and Flow (and Flow's quality) each key apart.
    #[test]
    fn interpolation_policy_keys_only_when_it_synthesises() {
        use lumit_core::retime::{FlowParams, Interpolation, Retime};
        use lumit_core::time::Rational;
        let doc = Document::new();
        let item = Uuid::now_v7();
        let footage = |interp: Option<Interpolation>| {
            let mut l = text_layer("", 0.0, 10.0, 0.0);
            let retime = interp.map(|i| {
                let mut r = Retime::constant_speed(
                    Rational::new(10, 1).unwrap(),
                    Rational::ZERO,
                    Rational::ONE,
                );
                r.interpolation = i;
                r
            });
            l.kind = LayerKind::Footage { item, retime };
            comp_with(vec![l])
        };
        let k = |c: &Composition| {
            comp_frame_key(&doc, c, 1.0, Quality::default(), &StubStamper).unwrap()
        };
        let plain = k(&footage(None));
        assert_eq!(plain, k(&footage(Some(Interpolation::Nearest))));
        let blend = k(&footage(Some(Interpolation::Blend)));
        assert_ne!(plain, blend);
        let half = k(&footage(Some(Interpolation::Flow(FlowParams::default()))));
        let full = k(&footage(Some(Interpolation::Flow(FlowParams {
            half_resolution: false,
            input_fps: lumit_core::anim::Property::zero(),
            extra: serde_json::Map::new(),
        }))));
        assert_ne!(blend, half);
        assert_ne!(half, full);
    }

    /// K-093: two comp times whose retimed source lands in the *same* integer
    /// source frame (the stamper returns the same frame index) but at
    /// different sub-frame positions. Blend and Flow synthesise a different
    /// in-between at each position, so their keys MUST differ — otherwise the
    /// cache holds one frame across the whole span and flow "only changes
    /// once in the middle". Nearest shows the one stamped frame either way,
    /// so its keys stay shared (the "Nearest keys like no-retime" law holds).
    #[test]
    fn synthesising_interpolation_keys_each_sub_frame_position() {
        use lumit_core::retime::{FlowParams, Interpolation, Retime};
        use lumit_core::time::Rational;
        let doc = Document::new();
        let item = Uuid::now_v7();
        let footage = |interp: Interpolation| {
            let mut l = text_layer("", 0.0, 10.0, 0.0);
            let mut r = Retime::constant_speed(
                Rational::new(10, 1).unwrap(),
                Rational::ZERO,
                Rational::ONE,
            );
            r.interpolation = interp;
            l.kind = LayerKind::Footage {
                item,
                retime: Some(r),
            };
            comp_with(vec![l])
        };
        let key = |c: &Composition, t: f64| {
            comp_frame_key(&doc, c, t, Quality::default(), &StubStamper).unwrap()
        };
        // Both times land in the same stamped integer frame (the stub rounds
        // source·60; 1.000 and 1.004 both round to 60), so it is the sub-frame
        // fraction, not the frame, that must differentiate the keys below.
        assert_eq!(
            StubStamper.stamp(item, 1.000).unwrap().1,
            StubStamper.stamp(item, 1.004).unwrap().1,
        );
        for policy in [
            Interpolation::Blend,
            Interpolation::Flow(FlowParams::default()),
        ] {
            let c = footage(policy);
            assert_ne!(
                key(&c, 1.000),
                key(&c, 1.004),
                "a synthesising policy must key each sub-frame position distinctly"
            );
        }
        let nearest = footage(Interpolation::Nearest);
        assert_eq!(
            key(&nearest, 1.000),
            key(&nearest, 1.004),
            "Nearest shows the one stamped frame — its keys stay shared"
        );
    }

    /// K-094: an echo (temporal) layer's key hashes the neighbour source
    /// frames it reads, so it differs from the same layer with the echo
    /// bypassed, and it moves with time as the neighbours change — the cache
    /// can't hold one echo frame across a span (the failure mode the flow bug
    /// showed).
    #[test]
    fn echo_keys_its_neighbour_frames() {
        let doc = Document::new();
        let item = Uuid::now_v7();
        let layer = |enabled: bool| {
            let mut l = text_layer("", 0.0, 10.0, 0.0);
            let mut echo = lumit_core::fx::instantiate("echo").unwrap();
            echo.enabled = enabled;
            l.effects.push(echo);
            l.kind = LayerKind::Footage { item, retime: None };
            comp_with(vec![l])
        };
        let k = |c: &Composition, t: f64| {
            comp_frame_key(&doc, c, t, Quality::default(), &StubStamper).unwrap()
        };
        // Echo live hashes the temporal neighbour block; bypassed it does not.
        assert_ne!(k(&layer(true), 1.0), k(&layer(false), 1.0));
        // The neighbours move with time, so the key evolves.
        assert_ne!(k(&layer(true), 1.0), k(&layer(true), 1.5));
    }

    /// K-094 covers Flow motion blur too: its temporal window is {0, 1}, so the
    /// existing neighbour-key block already hashes the +1 source frame it reads
    /// (the frame the flow field is measured against). A motion-blur layer's
    /// key therefore differs from the same layer bypassed, and moves with time
    /// as the next frame changes — the cache can't hold one motion-blurred
    /// frame across a span. A regression pinning that the {0, 1} forward window
    /// is covered with no motion-blur-specific plumbing.
    #[test]
    fn motion_blur_keys_its_next_frame() {
        let doc = Document::new();
        let item = Uuid::now_v7();
        let layer = |enabled: bool| {
            let mut l = text_layer("", 0.0, 10.0, 0.0);
            let mut mb = lumit_core::fx::instantiate("motion_blur").unwrap();
            mb.enabled = enabled;
            l.effects.push(mb);
            l.kind = LayerKind::Footage { item, retime: None };
            comp_with(vec![l])
        };
        let k = |c: &Composition, t: f64| {
            comp_frame_key(&doc, c, t, Quality::default(), &StubStamper).unwrap()
        };
        // Live hashes the next-frame block; bypassed does not.
        assert_ne!(k(&layer(true), 1.0), k(&layer(false), 1.0));
        // The +1 neighbour moves with time, so the key evolves.
        assert_ne!(k(&layer(true), 1.0), k(&layer(true), 1.5));
    }

    /// Per-layer transform motion blur (docs/06 §4, K-120) feeds the key only
    /// while the comp master and the layer switch are both on: with the master
    /// off the switch is inert (every pre-motion-blur key holds); with both on,
    /// each shutter setting moves the key, and — because a blurring layer's
    /// pixels are its transform sampled across the shutter — a
    /// same-position/different-velocity frame keys apart where the frame-time
    /// transform alone would collide (the K-093 lesson at the shutter samples).
    #[test]
    fn per_layer_motion_blur_feeds_the_key() {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        use lumit_core::model::MotionBlur;
        let doc = Document::new();
        // A triangle position ramp: 0 → 400 over [0,2], back to 0 over [2,4].
        // At t=1 and t=3 the instantaneous position is 200 either way, but the
        // motion reverses — the sub-frame samples differ.
        let kf = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let triangle = || {
            let mut l = text_layer("m", 0.0, 10.0, 0.0);
            l.transform.position_x = Property {
                extra: serde_json::Map::new(),
                animation: Animation::Keyframed(vec![kf(0, 0.0), kf(2, 400.0), kf(4, 0.0)]),
            };
            l
        };
        let off = comp_with(vec![triangle()]);
        // Master off: the layer switch is inert, and the frame-time transform is
        // equal at t=1 and t=3, so those frames share a key.
        let mut switch_only = off.clone();
        switch_only.layers[0].switches.motion_blur = true;
        assert_eq!(key(&doc, &off, 1.0), key(&doc, &switch_only, 1.0));
        assert_eq!(key(&doc, &off, 1.0), key(&doc, &off, 3.0));

        // Both on: the layer blurs.
        let on = || {
            let mut c = off.clone();
            c.motion_blur = MotionBlur {
                enabled: true,
                ..MotionBlur::default()
            };
            c.layers[0].switches.motion_blur = true;
            c
        };
        let blurred = on();
        let at1 = key(&doc, &blurred, 1.0);
        // Enabling blur is content: the key moves off the no-blur key.
        assert_ne!(key(&doc, &off, 1.0), at1);
        // The switch itself is content while the master is on: master-on with
        // the layer switch off keys differently from the blurring layer.
        let mut master_only = blurred.clone();
        master_only.layers[0].switches.motion_blur = false;
        assert_ne!(at1, key(&doc, &master_only, 1.0));
        // Same instantaneous position, reversed motion → the shutter samples
        // differ, so the two frames key apart (no velocity collision).
        assert_ne!(at1, key(&doc, &blurred, 3.0));

        // Each shutter setting is content.
        for tweak in [
            |m: &mut MotionBlur| m.shutter_angle = 90.0,
            |m: &mut MotionBlur| m.shutter_phase = 0.0,
            |m: &mut MotionBlur| m.samples = 8,
        ] {
            let mut c = blurred.clone();
            tweak(&mut c.motion_blur);
            assert_ne!(at1, key(&doc, &c, 1.0));
        }
    }

    /// K-095: a flow conform rate synthesises from different source frames at
    /// the same time, so changing it (including to/from Native) changes the
    /// key — the cache can't serve a frame flowed at the wrong rate.
    #[test]
    fn flow_conform_rate_keys_distinctly() {
        use lumit_core::retime::{FlowParams, Interpolation, Retime};
        use lumit_core::time::Rational;
        let doc = Document::new();
        let item = Uuid::now_v7();
        let footage = |fps: Option<f64>| {
            let mut l = text_layer("", 0.0, 10.0, 0.0);
            let mut r = Retime::constant_speed(
                Rational::new(10, 1).unwrap(),
                Rational::ZERO,
                Rational::ONE,
            );
            r.interpolation = Interpolation::Flow(FlowParams {
                half_resolution: true,
                input_fps: fps
                    .map(lumit_core::anim::Property::fixed)
                    .unwrap_or_else(lumit_core::anim::Property::zero),
                extra: serde_json::Map::new(),
            });
            l.kind = LayerKind::Footage {
                item,
                retime: Some(r),
            };
            comp_with(vec![l])
        };
        let k = |c: &Composition| {
            comp_frame_key(&doc, c, 1.0, Quality::default(), &StubStamper).unwrap()
        };
        assert_ne!(k(&footage(None)), k(&footage(Some(24.0))));
        assert_ne!(k(&footage(Some(24.0))), k(&footage(Some(12.0))));
    }

    /// A Sequence layer keys the active clip's source frame; a gap keys
    /// distinctly and moving through clips changes the key.
    #[test]
    fn sequence_keys_the_active_clip() {
        use lumit_core::sequence::{Clip, ClipSource};
        use lumit_core::time::Rational;
        let doc = Document::new();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let r = |n| Rational::new(n, 1).unwrap();
        // Clip A [0,2), gap [2,3), clip B [3,5).
        let clips = vec![
            Clip::new(ClipSource::Footage(a), r(0), r(2), r(0), r(2)),
            Clip::new(ClipSource::Footage(b), r(0), r(2), r(3), r(2)),
        ];
        let mut l = text_layer("", 0.0, 10.0, 0.0);
        l.kind = LayerKind::Sequence { clips };
        let comp = comp_with(vec![l]);
        let k = |t| comp_frame_key(&doc, &comp, t, Quality::default(), &StubStamper);
        // Both clips resolve (Some); the gap is still keyable (transparent).
        assert!(k(1.0).is_some() && k(4.0).is_some() && k(2.5).is_some());
        // Different clips → different keys; the gap differs from both.
        assert_ne!(k(1.0), k(4.0));
        assert_ne!(k(1.0), k(2.5));
    }
}
