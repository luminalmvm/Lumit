//! **A matte bound in a document drives a real effect, in a real render**
//! (K-395, the end-to-end half of the campaign).
//!
//! # In plain terms
//!
//! The unit tests either side of this one each prove half of the story. The
//! build tests prove a document's Matte row becomes *a slot on the carriage*
//! pointed at the right layer; the `fxops` tests prove `run_ops` hands the kth
//! slot to the kth effect, and that an unbound slot changes no pixel. Neither
//! renders a document.
//!
//! This one does: it builds a project the way a user would — two layers, one of
//! them named as another's Matte — and pushes it through the same public entry
//! the Viewer and the exporter use (`HeadlessRenderer::render_rgba`, the one
//! comp walk of K-031). So it crosses every seam at once: the document, the
//! draw builder's `mattes` list, the Realiser rendering that layer *alone* at
//! this raster, and the kernel reading its luma.
//!
//! The picture is arranged so the answer is unmissable. The bottom layer is a
//! flat mid grey carrying an Exposure of +1 stop, which unmatted lifts the whole
//! frame from 137 to 188. Its Matte is a **precomp** holding a white solid that
//! covers the left half of the frame, so one render contains both cases: lit
//! matte on the left, black matte on the right. The assertion is the K-395
//! sentence itself — **where the matte is lit the exposure applies in full, and
//! where it is black the pixels are the untouched source**, not close to it, the
//! same bytes.
//!
//! ## Two traps this file exists to have already fallen into
//!
//! 1. **A matte source is resampled to the effect's raster.** A 32-wide white
//!    solid pointed at *directly* is stretched to fill the frame and arrives as
//!    a uniformly white matte — the test would pass its "inside" check and
//!    silently be testing nothing. Wrapping the band in a full-size comp gives
//!    it real geometry first, which is the authoring route K-266 describes ("a
//!    white circle in a precomp is the natural way to author a flare source").
//! 2. **The band is left-aligned, and its edge is soft.** A solid at the default
//!    transform starts at the origin rather than centred, and its edge
//!    anti-aliases across a few pixels. So the assertions read the two *flat*
//!    regions either side of the transition and leave the boundary alone —
//!    testing a matte, not the resampler's edge treatment.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, EffectValue, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;
/// The white band's width inside the matte precomp: half the frame, laid down
/// from the left edge.
const BAND: u32 = 32;
/// The flat region inside the lit half, stopping well short of the soft edge.
const LIT: std::ops::Range<u32> = 0..24;
/// The flat region inside the black half, starting well past it.
const DARK: std::ops::Range<u32> = 44..COMP;

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
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

fn comp_of(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
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

/// Builds the grey base layer with an Exposure on it, and returns the document
/// plus the comp to render. `bind` chooses whether the Exposure's Matte row
/// points at the band precomp or stays unset — the only difference between the
/// two renders compared below, so anything they disagree about is the matte and
/// nothing else.
fn project(bind: bool) -> (Arc<Document>, Uuid) {
    let grey_def = Uuid::now_v7();
    let white_def = Uuid::now_v7();
    let mut doc = Document::new();
    // Mid grey, so +1 stop lands well short of clipping and the two states are
    // distinguishable bytes rather than two 255s.
    doc.items
        .push(solid(grey_def, "grey", [0.25, 0.25, 0.25, 1.0], COMP, COMP));
    doc.items
        .push(solid(white_def, "band", [1.0, 1.0, 1.0, 1.0], BAND, COMP));

    // The matte source: a full-size comp holding the half-width white solid, so
    // the band keeps its geometry through the resample (trap 1 above).
    let band_comp = comp_of(
        "band comp",
        vec![layer("band", LayerKind::Solid { def: white_def })],
    );
    let band_comp_id = band_comp.id;
    doc.items.push(ProjectItem::Composition(band_comp));

    let mut matte_layer = layer("matte source", LayerKind::Precomp { comp: band_comp_id });
    // A matte source is read, not composited: the build skips invisible layers
    // when drawing but still resolves them as layer inputs, which is how a depth
    // pass or a flare matte is authored. Left visible, the band would paint over
    // the very pixels the assertions read.
    matte_layer.switches.visible = false;

    let mut base = layer("base", LayerKind::Solid { def: grey_def });
    let mut exposure = lumit_core::fx::instantiate("exposure").expect("exposure is a built-in");
    for p in &mut exposure.params {
        if p.id == "stops" {
            p.value = EffectValue::Float(lumit_core::anim::Property::fixed(1.0));
        }
        if bind && p.id == lumit_core::fx::MATTE_PARAM {
            p.value = EffectValue::Layer(Some(matte_layer.id));
        }
    }
    base.effects = vec![exposure];

    // Topmost first: the hidden matte source above the layer that reads it.
    let comp = comp_of("Comp", vec![matte_layer, base]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id)
}

/// The red channel of the pixel at `x` on the middle row.
fn px(rgba: &[u8], w: u32, x: u32) -> u8 {
    let y = COMP / 2;
    rgba[((y * w + x) * 4) as usize]
}

#[test]
fn a_matte_bound_in_a_document_drives_the_effect_it_is_bound_to() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };

    let (bound_doc, bound_comp) = project(true);
    let (free_doc, free_comp) = project(false);
    let (bound, w, h) = r
        .render_rgba(&bound_doc, bound_comp, 0, 1.0)
        .expect("the matted render");
    let (control, cw, ch) = r
        .render_rgba(&free_doc, free_comp, 0, 1.0)
        .expect("the unmatted control");
    assert_eq!((w, h), (COMP, COMP), "rendered at comp size");
    assert_eq!((cw, ch), (w, h), "both renders are the same raster");

    // The control is one flat lifted grey, which is the premise the rest rests
    // on: if the unmatted render already had a gradient across it, "darker on
    // the right" would prove nothing about the matte.
    let lifted = px(&control, w, 0);
    for x in 0..w {
        assert_eq!(
            px(&control, w, x),
            lifted,
            "the unmatted control must be flat, and column {x} is not"
        );
    }
    // The untouched source, for the dark half to be measured against.
    let source = lumit_core::pixels::srgb_encode(0.25);
    assert_ne!(
        lifted, source,
        "the Exposure must actually do something, or the test is vacuous"
    );

    // 1. Where the matte is lit, the effect applied *in full* — the same byte
    //    the render with no matte at all produces.
    for x in LIT {
        assert_eq!(
            px(&bound, w, x),
            lifted,
            "column {x} is under the lit matte: the Exposure must apply in full"
        );
    }

    // 2. Where it is black, nothing happened — and the strong form of nothing:
    //    the untouched source grey, not merely less effect.
    for x in DARK {
        assert_eq!(
            px(&bound, w, x),
            source,
            "column {x} is under the black matte: the source must be untouched"
        );
    }
}

/// **A fully black matte suppresses the effect entirely** — the other end of the
/// same dissolve, and the cheapest possible statement of it: a full-frame matte,
/// so there is no geometry to get wrong and a failure here is the dissolve
/// itself rather than a resample.
#[test]
fn a_black_matte_suppresses_the_effect_entirely() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };
    let grey_def = Uuid::now_v7();
    let black_def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items
        .push(solid(grey_def, "grey", [0.25, 0.25, 0.25, 1.0], COMP, COMP));
    doc.items
        .push(solid(black_def, "black", [0.0, 0.0, 0.0, 1.0], COMP, COMP));

    let mut matte_layer = layer("matte source", LayerKind::Solid { def: black_def });
    matte_layer.switches.visible = false;
    let mut base = layer("base", LayerKind::Solid { def: grey_def });
    let mut exposure = lumit_core::fx::instantiate("exposure").expect("exposure is a built-in");
    for p in &mut exposure.params {
        if p.id == "stops" {
            p.value = EffectValue::Float(lumit_core::anim::Property::fixed(1.0));
        }
        if p.id == lumit_core::fx::MATTE_PARAM {
            p.value = EffectValue::Layer(Some(matte_layer.id));
        }
    }
    base.effects = vec![exposure];
    let comp = comp_of("Comp", vec![matte_layer, base]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));

    let doc = Arc::new(doc);
    let (rgba, w, _h) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");

    // The source grey, sRGB-encoded: the Exposure ran and was dissolved all the
    // way back out again.
    let want = lumit_core::pixels::srgb_encode(0.25);
    for x in 0..w {
        assert_eq!(
            px(&rgba, w, x),
            want,
            "column {x}: a black matte must leave the source untouched"
        );
    }
}
