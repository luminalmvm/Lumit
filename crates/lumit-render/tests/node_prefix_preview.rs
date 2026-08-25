//! **The Node preview renders the chain up to a node, and nothing past it**
//! (K-486, docs/impl/node-graph.md §8 WP5).
//!
//! # In plain terms
//!
//! The Graph panel draws a layer's effects as a row of boxes. The Node preview
//! panel answers "what does the picture look like *here*" — at one of those
//! boxes rather than at the end. Since the image chain **is** the effect stack
//! (§1.1 of the note), the picture at the third box is the picture the layer
//! makes with its stack cut off after the third effect. So the preview is not a
//! new kind of render at all: it is the ordinary comp render of a document whose
//! one layer has a shorter stack.
//!
//! That is the whole seam, and this file is why it can be trusted:
//!
//! - the cut document renders **exactly** what a project authored with those
//!   effects and no others renders — same bytes, not nearly;
//! - the cut renders **differ** from the full frame, so the panel is not quietly
//!   showing the Viewer's picture;
//! - the same cut renders the same bytes twice (K-031's determinism, held at
//!   this seam because a preview that flickers between two answers is a bug the
//!   eye finds before any test does);
//! - the frame key **already** separates them. The prefix point needs no field
//!   of its own in the key: the key hashes each layer's effects, so a shorter
//!   stack is a different name by construction, and a preview can never be
//!   served the full frame out of the cache. The Layer out node cuts nothing, so
//!   it keeps the Viewer's own name and rides its cached frame.
//!
//! The picture is arranged so the three states are unmissable: a mid-grey solid
//! carrying two +1-stop Exposures, so Source, after-the-first and after-the-
//! second are three plainly different greys rather than three shades of nearly.
//!
//! Runs only where there is a GPU, like every other render proof here.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::graph::{self, NodeRef};
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, Layer, LayerKind, LinearColour,
    ProjectItem, SolidDef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;

fn exposure(stops: f64) -> EffectInstance {
    let mut fx = lumit_core::fx::instantiate("exposure").expect("exposure is a built-in");
    for p in &mut fx.params {
        if p.id == "stops" {
            p.value = EffectValue::Float(lumit_core::anim::Property::fixed(stops));
        }
    }
    fx
}

/// A project of one grey solid carrying `effects`, and the comp to render.
/// The layer's id is fixed by the caller so the same layer can be addressed
/// across two documents.
fn project(layer_id: Uuid, effects: Vec<EffectInstance>) -> (Arc<Document>, Uuid) {
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    // Mid grey: two +1 stops still land short of clipping, so all three states
    // are distinguishable bytes rather than three 255s.
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "grey".into(),
        colour: LinearColour([0.1, 0.1, 0.1, 1.0]),
        width: COMP,
        height: COMP,
        extra: serde_json::Map::new(),
    }));
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: layer_id,
        name: "grey".into(),
        kind: LayerKind::Solid { def },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
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
    let comp = Composition {
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: COMP,
        height: COMP,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id)
}

/// The red channel of the middle pixel.
fn grey(rgba: &[u8], w: u32) -> u8 {
    let (x, y) = (w / 2, COMP / 2);
    rgba[((y * w + x) * 4) as usize]
}

#[test]
fn the_picture_at_a_node_is_the_stack_cut_off_at_that_node() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };

    let layer_id = Uuid::now_v7();
    let first = exposure(1.0);
    let second = exposure(1.0);
    let (doc, comp_id) = project(layer_id, vec![first.clone(), second.clone()]);

    // The three prefixes the panel can ask for, as lengths.
    let at_source = graph::prefix_len(&[first.clone(), second.clone()], NodeRef::Source)
        .expect("the source is a picture");
    let at_first = graph::prefix_len(&[first.clone(), second.clone()], NodeRef::Effect(first.id))
        .expect("an effect in this layer");
    let at_out = graph::prefix_len(&[first.clone(), second.clone()], NodeRef::Out)
        .expect("the layer out is a picture");
    assert_eq!((at_source, at_first, at_out), (0, 1, 2));

    // **The Layer out node cuts nothing**, so it is the Viewer's own frame: no
    // clone, no second render, the cached frame as it stands.
    assert!(
        graph::truncated_effects(&doc, comp_id, layer_id, at_out).is_none(),
        "the whole stack is the frame the Viewer already shows"
    );

    let cut_source = graph::truncated_effects(&doc, comp_id, layer_id, at_source)
        .expect("the source cuts both effects");
    let cut_first = graph::truncated_effects(&doc, comp_id, layer_id, at_first)
        .expect("the first node cuts the second effect");

    let (full, w, h) = r
        .render_rgba(&doc, comp_id, 0, 1.0)
        .expect("the full frame");
    let (source, ..) = r
        .render_rgba(&cut_source, comp_id, 0, 1.0)
        .expect("the source preview");
    let (after_first, ..) = r
        .render_rgba(&cut_first, comp_id, 0, 1.0)
        .expect("the first node's preview");
    assert_eq!((w, h), (COMP, COMP), "rendered at comp size");

    // Three plainly different pictures, brightening down the chain — the
    // premise everything below rests on.
    let (a, b, c) = (grey(&source, w), grey(&after_first, w), grey(&full, w));
    assert!(
        a < b && b < c,
        "each effect must show: source {a}, after the first {b}, the whole stack {c}"
    );

    // **The claim of the seam** (the note's WP5 test): the preview of the first
    // node is exactly what a project authored with only that effect renders —
    // the same bytes, so the cut is the honest one and not merely a darker
    // picture.
    let (only_first_doc, only_first_comp) = project(layer_id, vec![first.clone()]);
    let (only_first, ..) = r
        .render_rgba(&only_first_doc, only_first_comp, 0, 1.0)
        .expect("a project of one effect");
    assert_eq!(
        after_first, only_first,
        "the first node's preview differs from the Viewer by exactly the second effect"
    );

    let (bare_doc, bare_comp) = project(layer_id, Vec::new());
    let (bare, ..) = r
        .render_rgba(&bare_doc, bare_comp, 0, 1.0)
        .expect("a project of no effects");
    assert_eq!(
        source, bare,
        "the Source node is the layer before its stack"
    );

    // Determinism (K-031 at this seam): the same cut, twice, byte-identical.
    let (again, ..) = r
        .render_rgba(&cut_first, comp_id, 0, 1.0)
        .expect("the same preview again");
    assert_eq!(after_first, again, "a prefix render is deterministic");
}

/// The prefix point folds into the frame key without the key growing a field:
/// the key hashes each layer's effects, so a shorter stack is a different name.
/// Without this a preview could be served the Viewer's frame out of the cache —
/// the panel would show the wrong picture and nothing would be measurably
/// wrong anywhere else.
#[test]
fn each_prefix_names_its_own_frame() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };

    let layer_id = Uuid::now_v7();
    let first = exposure(1.0);
    let second = exposure(1.0);
    let (doc, comp_id) = project(layer_id, vec![first.clone(), second]);
    let q = Quality::default();

    let full = r
        .frame_key(&doc, comp_id, 0, q)
        .expect("a solid needs no probe, so the frame is nameable");
    let mut keys = vec![full];
    for keep in [0usize, 1] {
        let cut = graph::truncated_effects(&doc, comp_id, layer_id, keep).expect("a real cut");
        let key = r.frame_key(&cut, comp_id, 0, q).expect("nameable too");
        assert!(
            !keys.contains(&key),
            "the prefix keeping {keep} effects must name its own frame"
        );
        keys.push(key);
    }

    // And the same cut names the same frame — a key that moved between two
    // identical documents would defeat the cache instead of protecting it.
    let cut = graph::truncated_effects(&doc, comp_id, layer_id, 1).expect("a real cut");
    assert_eq!(
        r.frame_key(&cut, comp_id, 0, q),
        Some(keys[2]),
        "naming a prefix twice must give one name"
    );
}
