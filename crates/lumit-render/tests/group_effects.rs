//! **Effects on a layer group, end to end** (docs/impl/group-effects.md §7).
//!
//! # In plain terms
//!
//! A group header carrying effects renders as an implicit per-frame
//! precompose: the members composited alone, the header's stack run on that
//! one picture, the result composited back as one slab. These tests build
//! documents the way a user would and push them through the same public
//! entries the Viewer and the exporter use, to catch what would break
//! silently:
//!
//! - **The promise that grouped and ungrouped render alike.** A group whose
//!   header carries no effects (or only bypassed ones) must render
//!   byte-identical to no group at all, and keep the very same frame key.
//! - **The scope.** The header's blur reaches the members and nothing else —
//!   the layer above stays crisp, the plate below stays crisp.
//! - **The machine itself.** A grouped run with a live header is *the same
//!   picture* as packing the members into a Precomp layer wearing the same
//!   stack — that equivalence is the whole design (§2), so it is asserted
//!   directly, at full and at half preview resolution.
//! - **Mattes cross the boundary** in both directions while the header is
//!   live, because a matte source renders alone by construction.
//! - **An empty run runs nothing**: eyes off ⇒ no unit, and a generator on
//!   the header of an empty group paints not one pixel.
//! - **The name.** A header edit renames the frame; membership changes that
//!   move the drawn run rename it too; bypassing the stack gives the old
//!   name back.
//!
//! Preview and export agreement (test 10) is structural rather than asserted
//! against the encoder: the export renders through the same
//! `build_comp_draws_at` walk these tests drive (export.rs builds via the
//! headless renderer), so an effected group cannot diverge between the two
//! without the identity tests here failing first. Determinism is asserted
//! directly.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::group::LayerGroup;
use lumit_core::model::{
    BlendMode, Composition, Document, EffectInstance, EffectValue, Layer, LayerKind, LinearColour,
    ProjectItem, SolidDef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;

fn solid(def: Uuid, name: &str, colour: [f32; 4], w: u32, h: u32) -> ProjectItem {
    ProjectItem::Solid(SolidDef {
        id: def,
        name: name.into(),
        colour: LinearColour(colour),
        width: w,
        height: h,
        extra: serde_json::Map::new(),
    })
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
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
    }
}

/// A solid layer whose top-left corner sits at `(x, y)` — the default
/// transform puts a solid at the comp's origin, so a plain position offset is
/// the whole placement.
fn placed(name: &str, def: Uuid, x: f64, y: f64) -> Layer {
    let mut l = layer(name, LayerKind::Solid { def });
    l.transform.position_x = lumit_core::anim::Property::fixed(x);
    l.transform.position_y = lumit_core::anim::Property::fixed(y);
    l
}

fn comp_of(name: &str, layers: Vec<Layer>, groups: Vec<LayerGroup>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups,
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
        width: COMP,
        height: COMP,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn group(members: &[Uuid], effects: Vec<EffectInstance>) -> LayerGroup {
    LayerGroup {
        id: Uuid::now_v7(),
        name: "band".into(),
        label: 0,
        members: members.to_vec(),
        effects,
    }
}

/// A fresh builtin instance with `edits` applied to its float rows.
fn effect(name: &str, edits: &[(&str, f64)]) -> EffectInstance {
    let mut inst =
        lumit_core::fx::instantiate(name).unwrap_or_else(|| panic!("{name} is a builtin"));
    for (id, value) in edits {
        for p in &mut inst.params {
            if p.id == *id {
                p.value = EffectValue::Float(lumit_core::anim::Property::fixed(*value));
            }
        }
    }
    inst
}

/// The scene every scoping test reads: a crisp red square top-right, a grey
/// square centre (the member), a crisp blue square bottom-left — three bands
/// that never touch, so softness can be read per band.
///
/// Topmost first, as `Composition::layers` stores them.
fn three_bands(doc: &mut Document) -> (Layer, Layer, Layer) {
    let (ra, rb, rc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    doc.items
        .push(solid(ra, "red", [1.0, 0.0, 0.0, 1.0], 16, 16));
    doc.items
        .push(solid(rb, "grey", [0.5, 0.5, 0.5, 1.0], 24, 24));
    doc.items
        .push(solid(rc, "blue", [0.0, 0.0, 1.0, 1.0], 16, 16));
    (
        placed("above", ra, 40.0, 4.0),   // covers 40..56 × 4..20
        placed("member", rb, 20.0, 20.0), // covers 20..44 × 20..44
        placed("below", rc, 4.0, 44.0),   // covers 4..20 × 44..60
    )
}

fn rgb(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let d = ((y * w + x) * 4) as usize;
    [rgba[d], rgba[d + 1], rgba[d + 2]]
}

struct Stub;
impl lumit_eval::SourceStamper for Stub {
    fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
        Some((format!("stub:{item}"), (lt * 60.0).round().max(0.0) as u64))
    }
}

fn key_of(doc: &Arc<Document>, comp: Uuid) -> lumit_eval::FrameKey {
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.id == comp => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    lumit_eval::comp_frame_key(doc, &comp, 0.0, lumit_eval::Quality::default(), &Stub)
        .expect("a solid comp is always keyable")
}

/// **Test 1 — identity.** An empty header, and a wholly bypassed one, render
/// byte-identical to the same comp with no group at all, and share its frame
/// key; and the group's own file form carries no `effects` key while the list
/// is empty, so every older project re-saves byte-identical.
#[test]
fn a_header_with_no_live_effects_changes_nothing_at_all() {
    let build = |effects: Option<Vec<EffectInstance>>| {
        let mut doc = Document::new();
        let (a, b, c) = three_bands(&mut doc);
        let groups = effects
            .map(|fx| vec![group(&[b.id], fx)])
            .unwrap_or_default();
        let comp = comp_of("Comp", vec![a, b, c], groups);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    // The serde skip: an effect-less group writes no `effects` key.
    let bare = group(&[Uuid::now_v7()], Vec::new());
    let json = serde_json::to_value(&bare).unwrap();
    assert!(
        json.get("effects").is_none(),
        "an empty header stack must leave no trace in the file"
    );
    let back: LayerGroup = serde_json::from_value(json).unwrap();
    assert_eq!(back, bare, "and an older group loads to the same thing");

    let (ungrouped, comp_a) = build(None);
    let (grouped, comp_b) = build(Some(Vec::new()));
    let mut bypassed_fx = effect("blur", &[("radius", 6.0)]);
    bypassed_fx.enabled = false;
    let (bypassed, comp_c) = build(Some(vec![bypassed_fx]));

    // The keys agree before any pixel is rendered — a group that is not live
    // feeds nothing (§4), so every key ever made holds.
    let k = key_of(&ungrouped, comp_a);
    assert_eq!(k, key_of(&grouped, comp_b), "an empty header keeps the key");
    assert_eq!(
        k,
        key_of(&bypassed, comp_c),
        "a wholly bypassed header keeps the key"
    );

    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (base, ..) = r.render_rgba(&ungrouped, comp_a, 0, 1.0).unwrap();
    let (empty, ..) = r.render_rgba(&grouped, comp_b, 0, 1.0).unwrap();
    let (byp, ..) = r.render_rgba(&bypassed, comp_c, 0, 1.0).unwrap();
    assert_eq!(base, empty, "an empty header renders as no group at all");
    assert_eq!(base, byp, "a bypassed header renders as no group at all");
}

/// **Tests 3 and 9 — the machine itself, pinned.** A grouped run with a live
/// header is byte-for-byte the picture of packing the same members into a
/// nested comp and putting the same stack on the Precomp layer — including a
/// Multiply member over a plate below, which is exactly the isolation §3
/// chooses (the member stops multiplying the backdrop; the slab meets it as
/// one Normal picture). Asserted at full size and at half preview resolution,
/// which is the preview rescale riding the same equivalence.
#[test]
fn a_live_group_is_the_precompose_it_claims_to_be() {
    let mk_items = |doc: &mut Document| -> (Uuid, Uuid) {
        let (grey, white) = (Uuid::now_v7(), Uuid::now_v7());
        doc.items
            .push(solid(grey, "grey", [0.5, 0.5, 0.5, 1.0], 24, 24));
        doc.items
            .push(solid(white, "white", [1.0, 1.0, 1.0, 1.0], COMP, COMP));
        (grey, white)
    };
    let fx = || vec![effect("blur", &[("radius", 5.0)])];

    // The grouped document: a Multiply member over a full-frame white plate.
    let mut doc_g = Document::new();
    let (grey, white) = mk_items(&mut doc_g);
    let mut member = placed("member", grey, 20.0, 20.0);
    member.blend = BlendMode::Multiply;
    let plate = layer("plate", LayerKind::Solid { def: white });
    let g = group(&[member.id], fx());
    let comp_g = comp_of("grouped", vec![member, plate], vec![g]);
    let comp_g_id = comp_g.id;
    doc_g.items.push(ProjectItem::Composition(comp_g));
    let doc_g = Arc::new(doc_g);

    // The oracle: the same member packed into a nested comp, the same stack
    // on the Precomp layer, the same plate below.
    let mut doc_p = Document::new();
    let (grey, white) = mk_items(&mut doc_p);
    let mut member = placed("member", grey, 20.0, 20.0);
    member.blend = BlendMode::Multiply;
    let nested = comp_of("packed", vec![member], Vec::new());
    let nested_id = nested.id;
    doc_p.items.push(ProjectItem::Composition(nested));
    let mut pre = layer("pre", LayerKind::Precomp { comp: nested_id });
    pre.effects = fx();
    let plate = layer("plate", LayerKind::Solid { def: white });
    let comp_p = comp_of("parent", vec![pre, plate], Vec::new());
    let comp_p_id = comp_p.id;
    doc_p.items.push(ProjectItem::Composition(comp_p));
    let doc_p = Arc::new(doc_p);

    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    for scale in [1.0f32, 0.5] {
        let (g, w, _) = r.render_rgba(&doc_g, comp_g_id, 0, scale).unwrap();
        let (p, pw, _) = r.render_rgba(&doc_p, comp_p_id, 0, scale).unwrap();
        assert_eq!(w, pw);
        assert_eq!(
            g, p,
            "a live group at scale {scale} must be the Precomp picture exactly"
        );
    }
    // And the isolation is real: ungrouped, the Multiply member darkens the
    // white plate; grouped-with-live-header it no longer can.
    let mut doc_u = Document::new();
    let (grey, white) = mk_items(&mut doc_u);
    let mut member = placed("member", grey, 20.0, 20.0);
    member.blend = BlendMode::Multiply;
    let plate = layer("plate", LayerKind::Solid { def: white });
    let comp_u = comp_of("ungrouped", vec![member, plate], Vec::new());
    let comp_u_id = comp_u.id;
    doc_u.items.push(ProjectItem::Composition(comp_u));
    let (u, w, _) = r.render_rgba(&Arc::new(doc_u), comp_u_id, 0, 1.0).unwrap();
    let (g, ..) = r.render_rgba(&doc_g, comp_g_id, 0, 1.0).unwrap();
    let centre = rgb(&u, w, 32, 32);
    assert!(
        centre[0] < 200,
        "ungrouped, Multiply must darken the plate: {centre:?}"
    );
    assert_ne!(
        rgb(&g, w, 32, 32),
        centre,
        "grouped with a live header, the member composites in isolation (§3)"
    );
}

/// **Test 7 — the frame key moves when the picture can.** A header effect
/// renames the frame, a parameter edit renames it again, bypassing the last
/// instance restores the no-effects name, and a membership change that moves
/// the drawn run renames it while the layers themselves stand still.
#[test]
fn the_frame_key_follows_the_header_and_the_run() {
    let build = |fx: Vec<EffectInstance>, members_of: fn(&[Uuid]) -> Vec<Uuid>| {
        let mut doc = Document::new();
        let (a, b, c) = three_bands(&mut doc);
        let ids = [a.id, b.id, c.id];
        let g = group(&members_of(&ids), fx);
        let comp = comp_of("Comp", vec![a, b, c], vec![g]);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    let two = |ids: &[Uuid]| vec![ids[0], ids[1]];
    let one = |ids: &[Uuid]| vec![ids[1]];

    let (doc, comp) = build(Vec::new(), one);
    let bare = key_of(&doc, comp);

    let (doc, comp) = build(vec![effect("blur", &[("radius", 6.0)])], one);
    let live = key_of(&doc, comp);
    assert_ne!(bare, live, "a header effect must rename the frame");

    let (doc, comp) = build(vec![effect("blur", &[("radius", 12.0)])], one);
    let edited = key_of(&doc, comp);
    assert_ne!(live, edited, "a parameter edit must rename it again");

    let mut off = effect("blur", &[("radius", 6.0)]);
    off.enabled = false;
    let (doc, comp) = build(vec![off], one);
    assert_eq!(
        bare,
        key_of(&doc, comp),
        "bypassing the last instance restores the no-effects name"
    );

    // Same layers, same order, same stack — only the run the header acts on
    // differs, so the name must differ: the effect reaches other pixels.
    let (doc, comp) = build(vec![effect("blur", &[("radius", 6.0)])], two);
    let wide = key_of(&doc, comp);
    assert_ne!(live, wide, "widening the drawn run must rename the frame");
    let (doc, comp) = build(vec![effect("blur", &[("radius", 6.0)])], one);
    assert_eq!(
        live,
        key_of(&doc, comp),
        "and narrowing it back restores the name — deterministic, twice"
    );
}
