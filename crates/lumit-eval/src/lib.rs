//! The seed of **Togi**'s evaluator (K-067): pure content-hash frame keys
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
pub mod graph;
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
    // Draw order is content: iterate the stack as rendered. Layers outside
    // their span or hidden contribute nothing — presence is gated, never
    // hashed, so trimming a bar without crossing `t` changes no key.
    for layer in &comp.layers {
        let in_span = t >= layer.in_point.0.to_f64() && t < layer.out_point.0.to_f64();
        if !layer.switches.visible || !in_span {
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

    // The effect stack (docs/08): each live effect's identity, version and
    // evaluated parameters are content — the version bump is what retires
    // cached frames when an effect's maths change (K-016). Hashed only when
    // a live stack exists, so every pre-effects key stays valid. A bypassed
    // effect (or an fx-switched-off layer) contributes nothing, exactly as
    // it renders nothing.
    if layer.switches.fx && layer.effects.iter().any(|e| e.enabled) {
        h.update(b"effects/");
        for e in layer.effects.iter().filter(|e| e.enabled) {
            h.update(&[match e.effect.namespace {
                lumit_core::model::EffectNamespace::Builtin => 0,
                lumit_core::model::EffectNamespace::Ofx => 1,
                lumit_core::model::EffectNamespace::Lfx => 2,
                lumit_core::model::EffectNamespace::Placeholder => 3,
            }]);
            h.update(e.effect.match_name.as_bytes());
            h.update(&e.effect.version.to_le_bytes());
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
            // so it hashes nothing and those keys stay shared. (At an exactly
            // integral position blend/flow also degenerate to the source
            // frame; hashing them anyway over-segments harmlessly — a spare
            // render, never a wrong pixel.)
            if let Some(r) = retime {
                if !matches!(r.interpolation, lumit_core::retime::Interpolation::Nearest) {
                    h.update(&[interp_tag(&r.interpolation)]);
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
                            h.update(&[interp_tag(&clip.interpolation)]);
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
            extra: serde_json::Map::new(),
        }))));
        assert_ne!(blend, half);
        assert_ne!(half, full);
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
