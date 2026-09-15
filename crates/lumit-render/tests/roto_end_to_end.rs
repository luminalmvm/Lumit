//! **A propagated matte cuts a layer in a real render** (docs/impl/roto.md
//! §5 and §10 item 7).
//!
//! # In plain terms
//!
//! The unit tests either side of this one prove half the story each. The
//! `lumit-render::roto` tests prove the propagation job produces mattes, caches
//! them, and copies rather than re-solves what a correction did not touch; the
//! `lumit-core::roto` tests prove which frames an edit invalidates. Neither
//! renders anything.
//!
//! This one does. It puts a Roto brush on a layer, files a matte for that
//! layer's source frame under the effect's own id, and pushes the document
//! through the same public entry the Viewer and the exporter use
//! (`HeadlessRenderer::render_rgba`, the one comp walk there is). So it crosses
//! every seam at once: the store, the draw builder's roto carriage, the upload,
//! and the pass in `run_ops`.
//!
//! The picture is arranged so the answer is unmissable. A flat white layer fills
//! the frame; the matte is solid on the left half and empty on the right. Where
//! the matte is solid the layer is opaque and the frame is white; where it is
//! empty the layer is cut away entirely and the frame is the comp's own black
//! background. And a **second render with the matte filed one frame away**
//! proves the passthrough: outside the propagated span the layer is whole, not
//! wearing a neighbour's matte.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
    TransformGroup,
};
use lumit_core::roto::{RotoBlock, RotoStroke, RotoStrokeKind};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;
/// Well inside the matte's solid half.
const KEPT: u32 = 12;
/// Well inside its empty half.
const CUT: u32 = 52;

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

fn comp_of(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
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

/// A white layer wearing a stroked Roto brush, and the effect's own id so the
/// matte can be filed under it.
fn project() -> (Arc<Document>, Uuid, Uuid) {
    let white = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: white,
        name: "white".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: COMP,
        height: COMP,
        extra: serde_json::Map::new(),
    }));

    let mut brush = lumit_core::fx::instantiate("roto_brush").expect("roto brush is a built-in");
    // Stroked, so the frame key stamps a chain hash and the panel would offer
    // Propagate. The strokes themselves are not read here — the matte is filed
    // directly, exactly as the bridge's own tests file a camera solve.
    brush.roto = Some(RotoBlock {
        base_frame: Some(0),
        strokes: vec![RotoStroke {
            id: Uuid::now_v7(),
            points: vec![(4.0, 32.0), (20.0, 32.0)],
            radius: 3.0,
            kind: RotoStrokeKind::Foreground,
            frame: 0,
        }],
        prompts: Vec::new(),
    });
    let instance = brush.id;

    let mut base = layer("cut me", LayerKind::Solid { def: white });
    base.effects = vec![brush];

    let comp = comp_of("Comp", vec![base]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id, instance)
}

/// A matte that is solid on the left half of the frame and empty on the right.
fn half_matte() -> Vec<u8> {
    (0..COMP)
        .flat_map(|_| (0..COMP).map(|x| if x < COMP / 2 { 255u8 } else { 0 }))
        .collect()
}

fn px(rgba: &[u8], w: u32, x: u32) -> [u8; 4] {
    let y = COMP / 2;
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn a_propagated_matte_cuts_the_layer_it_sits_on_and_passes_through_outside_its_span() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (doc, comp, instance) = project();

    // No propagation yet: the effect is a passthrough and the layer is whole.
    lumit_render::roto::clear();
    let (before, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("the plain render");
    assert_eq!(
        px(&before, w, CUT)[0],
        255,
        "with nothing propagated the Roto brush changes no pixel"
    );

    // The matte, filed for source frame 0 under this effect instance.
    let run =
        lumit_render::roto::run_from_planes(COMP, COMP, 60.0, 1, &[(0, [1u8; 32], half_matte())])
            .expect("a run");
    lumit_render::roto::publish(instance, run);
    assert_eq!(lumit_render::roto::span(instance), Some((0, 0)));

    let (cut, w, h) = r
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("the matted render");
    assert_eq!((w, h), (COMP, COMP), "rendered at comp size");
    let kept = px(&cut, w, KEPT);
    let gone = px(&cut, w, CUT);
    assert_eq!(
        kept[0], 255,
        "where the matte is solid the layer is untouched"
    );
    assert_eq!(
        gone,
        [0, 0, 0, 255],
        "where the matte is empty the layer is cut away and the comp's own \
         background shows through"
    );

    // **Outside the span, passthrough.** A Solid layer's source frame is nought
    // at every comp time — it is the same picture throughout — so the span is
    // moved rather than the playhead: a run holding only frame 5 has nothing to
    // say about the frame being drawn, and the layer must come back whole. Never
    // the nearest matte held over, which would be a wrong answer wearing a right
    // one's face.
    let elsewhere =
        lumit_render::roto::run_from_planes(COMP, COMP, 60.0, 8, &[(5, [2u8; 32], half_matte())])
            .expect("a run");
    lumit_render::roto::publish(instance, elsewhere);
    assert_eq!(lumit_render::roto::span(instance), Some((5, 5)));
    let (outside, w, _) = r
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("a frame outside the span");
    assert_eq!(
        px(&outside, w, CUT)[0],
        255,
        "a frame outside the propagated span renders passthrough"
    );

    lumit_render::roto::clear();
}

/// **A layer read as another effect's matte source runs its own Roto brush**
/// (docs/impl/addons.md §13's last trap).
///
/// A referenced layer's stack used to run through `run_ops` with every side
/// carriage empty, so a Roto brush on the layer somebody pointed a Matte row at
/// rendered as a passthrough and the consumer read the plain footage instead.
/// The fold that gave the planes tier its carriage threads the roto one through
/// the same door, and this is the picture that says so: the source layer is a
/// white solid wearing a Roto brush whose matte is solid on the left and empty
/// on the right, and Set matte reads its alpha, so the consuming layer is kept
/// on the left and cut away on the right. Without the carriage the source reads
/// as plain opaque white and the consumer is untouched.
#[test]
fn a_layer_read_as_a_matte_source_runs_its_own_roto_brush() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    lumit_render::roto::clear();

    let white = Uuid::now_v7();
    let red = Uuid::now_v7();
    let mut doc = Document::new();
    for (id, name, colour) in [
        (white, "white", LinearColour([1.0, 1.0, 1.0, 1.0])),
        (red, "red", LinearColour([1.0, 0.0, 0.0, 1.0])),
    ] {
        doc.items.push(ProjectItem::Solid(SolidDef {
            id,
            name: name.into(),
            colour,
            width: COMP,
            height: COMP,
            extra: serde_json::Map::new(),
        }));
    }

    let mut brush = lumit_core::fx::instantiate("roto_brush").expect("roto brush is a built-in");
    brush.roto = Some(RotoBlock {
        base_frame: Some(0),
        strokes: vec![RotoStroke {
            id: Uuid::now_v7(),
            points: vec![(4.0, 32.0), (20.0, 32.0)],
            radius: 3.0,
            kind: RotoStrokeKind::Foreground,
            frame: 0,
        }],
        prompts: Vec::new(),
    });
    let instance = brush.id;
    let mut source = layer("the matte", LayerKind::Solid { def: white });
    source.effects = vec![brush];
    let source_id = source.id;

    let mut set_matte = lumit_core::fx::instantiate("set_matte").expect("a built-in");
    // The alpha, not the luminance: a cut white solid is white where it is
    // kept and *nothing at all* where it is not, so only the alpha tells the
    // two apart.
    for p in &mut set_matte.params {
        match p.id.as_str() {
            "matte" => {
                p.value = lumit_core::model::EffectValue::Layer(Some(source_id));
            }
            "channel" => p.value = lumit_core::model::EffectValue::Choice(1),
            _ => {}
        }
    }
    assert_eq!(set_matte.layer_ref("matte"), Some(source_id));
    let mut consumer = layer("the consumer", LayerKind::Solid { def: red });
    consumer.effects = vec![set_matte];

    let comp = comp_of("Comp", vec![consumer, source]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);

    let run =
        lumit_render::roto::run_from_planes(COMP, COMP, 60.0, 1, &[(0, [3u8; 32], half_matte())])
            .expect("a run");
    lumit_render::roto::publish(instance, run);

    let (cut, w, _) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("the render");
    assert_eq!(
        px(&cut, w, KEPT),
        [255, 0, 0, 255],
        "where the source's matte is solid the consumer is kept"
    );
    // Cut away, and the source layer behind it is cut away there too, so the
    // comp's own background shows. Red here would mean the consumer was kept
    // on both sides.
    assert_eq!(
        px(&cut, w, CUT),
        [0, 0, 0, 255],
        "the consumer is still red on the right, so the referenced layer's \
         own matte did not reach its stack"
    );

    lumit_render::roto::clear();
}
