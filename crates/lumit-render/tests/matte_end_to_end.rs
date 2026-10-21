//! **A matte bound in a document drives a real effect, in a real render**
//! (the end-to-end half of the campaign).
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
//! comp walk preview and export share). So it crosses every seam at once: the
//! document, the draw builder's `mattes` list, the Realiser rendering that
//! layer *alone* at this raster, and the kernel reading its luma.
//!
//! The picture is arranged so the answer is unmissable. The bottom layer is a
//! flat mid grey carrying an Exposure of +1 stop, which unmatted lifts the whole
//! frame from 137 to 188. Its Matte is a **precomp** holding a white solid that
//! covers the left half of the frame, so one render contains both cases: lit
//! matte on the left, black matte on the right. The assertion is the override's
//! own sentence — **where the matte is lit the exposure applies in full, and
//! where it is black the pixels are the untouched source**, not close to it, the
//! same bytes.
//!
//! ## Two traps this file exists to have already fallen into
//!
//! 1. **A matte source used to be stretched to the effect's raster.** A 32-wide
//!    white solid pointed at *directly* filled the frame and arrived as
//!    a uniformly white matte — the test would pass its "inside" check and
//!    silently be testing nothing. Wrapping the band in a full-size comp gives
//!    it real geometry first, which is how this is authored anyway: a white
//!    circle in a precomp is the natural way to author a flare source.
//! 2. **The band is left-aligned, and its edge is soft.** A solid at the default
//!    transform starts at the origin rather than centred, and its edge
//!    anti-aliases across a few pixels. So the assertions read the two *flat*
//!    regions either side of the transition and leave the boundary alone —
//!    testing a matte, not the resampler's edge treatment.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, EffectValue, Layer, LayerKind, LinearColour, MatteChannel, MatteRef,
    ProjectItem, SolidDef, Switches, TransformGroup,
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
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

fn comp_of(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
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
    let Ok(mut r) = HeadlessRenderer::shared() else {
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

/// **A hidden layer still mattes, and still stays out of the picture** (K-???).
///
/// The visibility switch says "do not composite this layer into the frame". It
/// has never meant "stop being a matte": a matte source is hidden precisely so
/// its own picture does not paint over the thing it is gating, which is how
/// every project authors one. Hiding it and losing the matte is the bug.
///
/// The scene is the plainest statement of both halves at once: a full-frame
/// grey plate matted by a hidden 32-wide white solid on the left of a 64-wide
/// comp. The left half must be the grey (the matte let it through) and never
/// the white (the matte source itself did not draw); the right half must be the
/// comp background (the matte held it back).
///
/// Twice more where the matte used to land in the wrong place: with both
/// layers parented to a layer moved 16 to the right, as a duplicated layer
/// keeps its parent, and in a Viewer region over the right half, which the
/// matte was squeezed into whole.
#[test]
fn a_hidden_layer_still_mattes_and_stays_out_of_the_picture() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let grey_def = Uuid::now_v7();
    let white_def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items
        .push(solid(grey_def, "grey", [0.25, 0.25, 0.25, 1.0], COMP, COMP));
    doc.items
        .push(solid(white_def, "band", [1.0, 1.0, 1.0, 1.0], BAND, COMP));

    let mut parent = layer("parent", LayerKind::Solid { def: grey_def });
    parent.transform.position_x = lumit_core::anim::Property::fixed(16.0);
    parent.switches.visible = false;
    let mut matte_layer = layer("matte source", LayerKind::Solid { def: white_def });
    matte_layer.switches.visible = false;
    let mut base = layer("base", LayerKind::Solid { def: grey_def });
    base.matte = Some(MatteRef {
        layer: matte_layer.id,
        channel: MatteChannel::Alpha,
        inverted: false,
        source: lumit_core::model::LayerInputSource::default(),
    });

    let grey = lumit_core::pixels::srgb_encode(0.25);
    // How far the parent moves both layers, and the region as comp fractions.
    for (shift, region) in [(0, None), (16, None), (0, Some([0.5, 0.0, 1.0, 1.0]))] {
        let (mut matte_layer, mut base) = (matte_layer.clone(), base.clone());
        if shift > 0 {
            matte_layer.parent = Some(parent.id);
            base.parent = Some(parent.id);
        }
        let mut doc = doc.clone();
        let comp = comp_of("Comp", vec![parent.clone(), matte_layer, base]);
        let comp_id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        r.set_region(region);
        let (rgba, w, _h) = r
            .render_rgba(&Arc::new(doc), comp_id, 0, 1.0)
            .expect("render");
        // A region comes back stretched to the comp's size.
        let [u0, _, u1, _] = region.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        for x in 0..w {
            let at = (u0 + (x as f32 + 0.5) / w as f32 * (u1 - u0)) * COMP as f32;
            let at = at as i64 - shift;
            let got = px(&rgba, w, x);
            if at >= 0 && LIT.contains(&(at as u32)) {
                assert_eq!(
                    got, grey,
                    "column {x} ({shift}, {region:?}) is under the hidden matte: the plate must show through it"
                );
            } else if at < 0 || DARK.contains(&(at as u32)) {
                assert_eq!(
                    got, 0,
                    "column {x} ({shift}, {region:?}) is past the hidden matte: the plate must be held back"
                );
            }
        }
    }
    r.set_region(None);
}

/// **The Matte dropdown on an adjustment layer gates its whole stack.**
///
/// Not the per-effect Matte row above, but the layer's own Matte — the row in
/// the timeline every layer carries. The draw builder used to leave it behind
/// on the adjustment arm (the arm pushes its draw and moves on, and the matte
/// was built after the arm), so the dropdown chose a source that gated nothing
/// whatever: the adjustment applied to the entire frame.
///
/// A grey plate under an adjustment layer carrying a +1 stop Exposure, matted
/// by a hidden white band down the left half. The exposure must reach the plate
/// under the band and leave the rest of it exactly as it found it.
#[test]
fn the_matte_dropdown_on_an_adjustment_layer_gates_its_stack() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let grey_def = Uuid::now_v7();
    let white_def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items
        .push(solid(grey_def, "grey", [0.25, 0.25, 0.25, 1.0], COMP, COMP));
    doc.items
        .push(solid(white_def, "band", [1.0, 1.0, 1.0, 1.0], BAND, COMP));

    // A layer matte is composited into comp space by its own transform rather
    // than resampled to an effect's raster, so the band needs no precomp
    // wrapper here (unlike trap 1 in this file's header).
    let mut matte_layer = layer("matte source", LayerKind::Solid { def: white_def });
    matte_layer.switches.visible = false;

    let mut adjust = layer("adjustment", LayerKind::Adjustment);
    let mut exposure = lumit_core::fx::instantiate("exposure").expect("exposure is a built-in");
    for p in &mut exposure.params {
        if p.id == "stops" {
            p.value = EffectValue::Float(lumit_core::anim::Property::fixed(1.0));
        }
    }
    adjust.effects = vec![exposure];
    adjust.matte = Some(MatteRef {
        layer: matte_layer.id,
        channel: MatteChannel::Alpha,
        inverted: false,
        source: lumit_core::model::LayerInputSource::default(),
    });

    let base = layer("base", LayerKind::Solid { def: grey_def });
    let comp = comp_of("Comp", vec![matte_layer, adjust, base]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let (rgba, w, _h) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");

    let lifted = lumit_core::pixels::srgb_encode(0.5);
    let source = lumit_core::pixels::srgb_encode(0.25);
    assert_ne!(lifted, source, "the Exposure must actually do something");
    for x in LIT {
        assert_eq!(
            px(&rgba, w, x),
            lifted,
            "column {x} is under the layer matte: the adjustment applies in full"
        );
    }
    for x in DARK {
        assert_eq!(
            px(&rgba, w, x),
            source,
            "column {x} is past the layer matte: the adjustment is held off"
        );
    }
}

/// The bound document again, with the Matte row's Invert and Channel and the
/// Mix row's Blend set as asked — `None` for a parameter leaves it at its
/// default, and `strip` removes the Channel and Blend rows altogether, which is
/// what every project saved before they existed looks like.
fn project_with(
    invert: bool,
    channel: Option<u32>,
    blend: Option<u32>,
    strip: bool,
) -> (Arc<Document>, Uuid) {
    let (doc, comp_id) = project(true);
    let mut doc = Arc::try_unwrap(doc).expect("unshared");
    for item in &mut doc.items {
        let ProjectItem::Composition(c) = item else {
            continue;
        };
        for layer in &mut c.layers {
            for fx in &mut layer.effects {
                for p in &mut fx.params {
                    if p.id == lumit_core::fx::MATTE_INVERT_PARAM {
                        p.value = EffectValue::Bool(invert);
                    }
                    if let (true, Some(ch)) = (p.id == lumit_core::fx::MATTE_CHANNEL_PARAM, channel)
                    {
                        p.value = EffectValue::Choice(ch);
                    }
                    if let (true, Some(b)) = (p.id == lumit_core::fx::BLEND_PARAM, blend) {
                        p.value = EffectValue::Choice(b);
                    }
                }
                if strip {
                    fx.params.retain(|p| {
                        p.id != lumit_core::fx::MATTE_CHANNEL_PARAM
                            && p.id != lumit_core::fx::BLEND_PARAM
                    });
                }
            }
        }
    }
    (Arc::new(doc), comp_id)
}

/// **The Blend row combines the effect with its input, end to end.** An
/// Exposure of +1 stop on mid grey, blended Multiply: the lifted grey
/// multiplied by the source grey, 0.5 x 0.25 = 0.125 in linear, darker than
/// the source — where Normal lifts it. Exposure claims its matte: under the
/// dark half its Stops are scaled to 0, so the kernel hands the blend the
/// untouched grey and Multiply squares it, 0.25 x 0.25 = 0.0625 — the same
/// answer an Exposure at 0 stops under Multiply gives with no matte at all,
/// which is what "the matte scales the amount" means beside a blend.
#[test]
fn a_multiply_blend_darkens_where_normal_lifts() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (doc, comp) = project_with(false, None, Some(2), false);
    let (rgba, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("render");
    let multiplied = lumit_core::pixels::srgb_encode(0.125);
    let squared = lumit_core::pixels::srgb_encode(0.0625);
    for x in LIT {
        let got = px(&rgba, w, x);
        assert!(
            (i16::from(got) - i16::from(multiplied)).abs() <= 1,
            "column {x}: Multiply of the lifted grey with the source, got {got} want {multiplied}"
        );
    }
    for x in DARK {
        let got = px(&rgba, w, x);
        assert!(
            (i16::from(got) - i16::from(squared)).abs() <= 1,
            "column {x}: Multiply of the unlifted grey with itself, got {got} want {squared}"
        );
    }
}

/// **A map is read where its layer sits.** Emit from image pointed at a small
/// white square off to one side of the comp puts its points on that square,
/// not over the whole frame as a stretched read did. The layer the effect is
/// on is moved as well, so the points only land right if the read follows
/// both layers into the comp. Particulate's Density layer is read the same
/// way, so its births land on the square too.
#[test]
fn emit_from_image_reads_its_source_where_the_source_sits() {
    use lumit_core::anim::Property;
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (black_def, white_def) = (Uuid::now_v7(), Uuid::now_v7());
    let mut doc = Document::new();
    doc.items
        .push(solid(black_def, "black", [0.0, 0.0, 0.0, 1.0], COMP, COMP));
    doc.items
        .push(solid(white_def, "logo", [1.0, 1.0, 1.0, 1.0], 16, 16));

    // The square covers x 40..56 and y 8..24 of the comp, and is only read.
    let mut logo = layer("logo", LayerKind::Solid { def: white_def });
    logo.transform.position_x = Property::fixed(40.0);
    logo.transform.position_y = Property::fixed(8.0);
    logo.switches.visible = false;

    let mut base = layer("base", LayerKind::Solid { def: black_def });
    base.transform.position_x = Property::fixed(8.0);
    base.transform.position_y = Property::fixed(4.0);
    let mut emit = lumit_core::fx::instantiate("emit_from_image").expect("a built-in");
    for p in &mut emit.params {
        match p.id.as_str() {
            "source" => p.value = EffectValue::Layer(Some(logo.id)),
            // Enough candidates that the square gets well over a hundred.
            "density" => p.value = EffectValue::Float(Property::fixed(5000.0)),
            "size" => p.value = EffectValue::Float(Property::fixed(2.0)),
            _ => {}
        }
    }
    // Still particles born over the whole of the base layer, a second in.
    let mut dense = lumit_core::fx::instantiate("particulate").expect("a built-in");
    for p in &mut dense.params {
        let f = |v: f64| EffectValue::Float(Property::fixed(v));
        match p.id.as_str() {
            "density_layer" => p.value = EffectValue::Layer(Some(logo.id)),
            "shape" => p.value = EffectValue::Choice(3),
            "position_x" | "position_y" => p.value = f(32.0),
            "width" | "height" => p.value = f(64.0),
            "emit_rate" => p.value = f(1000.0),
            "initial_speed" | "turbulence_amount" => p.value = f(0.0),
            "size" => p.value = f(2.0),
            _ => {}
        }
    }

    for (effect, frame) in [(emit, 0), (dense, 60)] {
        let mut base = base.clone();
        let name = effect.effect.match_name.clone();
        base.effects = vec![effect];
        let mut doc = doc.clone();
        let comp = comp_of("Comp", vec![logo.clone(), base]);
        let comp_id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        let (rgba, w, _) = r
            .render_rgba(&Arc::new(doc), comp_id, frame, 1.0)
            .expect("the render");

        // A point is a disc two pixels wide, so it may reach a pixel past the
        // square's edge.
        let (mut inside, mut outside) = (0, 0);
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            if px[0] < 64 {
                continue;
            }
            if (39..57).contains(&x) && (7..25).contains(&y) {
                inside += 1;
            } else {
                outside += 1;
            }
        }
        assert!(
            inside > 20,
            "{name}: only {inside} lit pixels on the square"
        );
        assert_eq!(
            outside, 0,
            "{name}: {outside} lit pixels away from the square"
        );
    }
}

/// **A matte is read where its layer sits.** The grey layer is 32 wide and
/// starts at x 24, so the full-frame band's edge at x 32 crosses it a quarter
/// of the way in. Stretched over the layer, the band would light its left half
/// instead, out to x 40.
#[test]
fn a_matte_lines_up_with_a_layer_that_is_not_full_frame() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (doc, comp_id) = project(true);
    let mut doc = (*doc).clone();
    for item in &mut doc.items {
        match item {
            ProjectItem::Solid(s) if s.name == "grey" => (s.width, s.height) = (BAND, 16),
            ProjectItem::Composition(c) if c.name == "Comp" => {
                let t = &mut c.layers[1].transform;
                t.position_x = lumit_core::anim::Property::fixed(24.0);
                t.position_y = lumit_core::anim::Property::fixed(24.0);
            }
            _ => {}
        }
    }
    let (rgba, w, _) = r
        .render_rgba(&Arc::new(doc), comp_id, 0, 1.0)
        .expect("the render");
    let source = lumit_core::pixels::srgb_encode(0.25);
    assert_ne!(px(&rgba, w, 27), source, "x 27 is under the lit band");
    assert_eq!(px(&rgba, w, 36), source, "x 36 is past the band's edge");
}
