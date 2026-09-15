//! **A Precomp layer's Retime map reaches the comp inside it** (docs/impl/
//! node-graph-comp.md §5.6, docs/04-RETIMING.md §11).
//!
//! # In plain terms
//!
//! The Retime property, the ops that write it, the bridge and the Timeline's
//! row all worked on a Precomp layer already. What did not work was the
//! picture: every site that evaluates a nested comp handed layer time straight
//! through, so a precomp slowed to half speed played at full speed and a frozen
//! one kept moving. The map is now read where the comp is evaluated, and this
//! file is that claim made through the public entry the Viewer and the exporter
//! both use.
//!
//! The scene is arranged so the answer is a byte comparison rather than a
//! judgement. The nested comp holds one small white square sliding left to
//! right; where it is says which frame of that comp is on screen. So:
//!
//! - at **half speed** the parent's frame 2N is the un-retimed frame N,
//! - a **freeze** is that one frame for ever,
//! - a map that **overruns** the nested comp holds its last frame,
//! - and with **no map at all** nothing is drawn past the nested duration,
//!   which is the behaviour that was there before and must stay.
//!
//! The second test takes the same precomp down the other two roads a comp is
//! read by - a matte source and a Light wrap background - because those go
//! through `nested_comp_draw` rather than the Precomp arm, and a map read in
//! one place and not the other is exactly the bug this file exists to catch.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::model::{
    Composition, Document, EffectValue, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;
/// The nested comp's own duration, in seconds, and the rate every comp here
/// runs at: one second of sixty frames, so a frame is a sixtieth.
const INNER_S: i64 = 1;
const FPS: u32 = 60;

fn layer(name: &str, kind: LayerKind, out_s: i64) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(out_s, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
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
    }
}

fn comp_of(name: &str, layers: Vec<Layer>, duration_s: i64) -> Composition {
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
        frame_rate: FrameRate::new(FPS, 1).unwrap(),
        duration: Duration(Rational::new(duration_s, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

/// A Retime property from `(layer time, source time)` pairs, straight between
/// them - the shape a constant speed or a ramp has once it is keyframes.
fn retime(points: &[(i64, f64)]) -> Property {
    Property {
        animation: Animation::Keyframed(
            points
                .iter()
                .map(|&(t, s)| Keyframe {
                    time: Rational::new(t, 1).unwrap(),
                    value: s,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                })
                .collect(),
        ),
        extra: serde_json::Map::new(),
    }
}

/// The project: a nested comp holding one white square that slides from the
/// left edge to the right over the comp's one second, and a parent that places
/// it. `map` is the Precomp layer's Retime.
///
/// Answers the document and the parent comp's id.
fn project(map: Option<Property>) -> (Arc<Document>, Uuid) {
    let square = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: square,
        name: "square".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: 8,
        height: COMP,
        extra: serde_json::Map::new(),
    }));
    let mut slider = layer("square", LayerKind::Solid { def: square }, INNER_S);
    slider.transform.position_x = Property {
        animation: Animation::Keyframed(vec![
            Keyframe {
                time: Rational::ZERO,
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            Keyframe {
                time: Rational::new(INNER_S, 1).unwrap(),
                value: f64::from(COMP),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
        ]),
        extra: serde_json::Map::new(),
    };
    let inner = comp_of("inner", vec![slider], INNER_S);
    let inner_id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));

    // The parent runs long enough to ask for moments the nested comp does not
    // have, which is what the overrun assertions need.
    let mut placed = layer("placed", LayerKind::Precomp { comp: inner_id }, 4);
    placed.retime = map;
    let parent = comp_of("parent", vec![placed], 4);
    let parent_id = parent.id;
    doc.items.push(ProjectItem::Composition(parent));
    (Arc::new(doc), parent_id)
}

/// Whether any pixel of the frame is lit - a nested comp past its own end draws
/// nothing at all, and this is how that is read.
fn lit(rgba: &[u8]) -> bool {
    rgba.chunks_exact(4).any(|px| px[0] > 8)
}

#[test]
fn a_retimed_precomp_shows_the_frame_its_map_points_at() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // One shot of the parent comp: the export path at full size, or the
    // preview path at whatever quality is asked for.
    let mut render = |map: Option<Property>, frame: u64, q: Option<lumit_render::Quality>| {
        let (doc, comp) = project(map);
        match q {
            Some(q) => {
                r.render_preview(&doc, comp, frame, q, 1.0)
                    .expect("the preview render")
                    .0
            }
            None => r.render_rgba(&doc, comp, frame, 1.0).expect("the render").0,
        }
    };

    // Half speed: two seconds of the layer are one of the comp inside it, so
    // frame 2N of the retimed parent is frame N of the plain one.
    let half = retime(&[(0, 0.0), (2, 1.0)]);
    for n in [6u64, 15, 29] {
        assert_eq!(
            render(Some(half.clone()), n * 2, None),
            render(None, n, None),
            "at half speed frame {} must be the un-retimed frame {n}",
            n * 2
        );
    }

    // And at half the render scale, where the nested comp's intermediate is
    // half size: the map is read on the clock, not on the raster.
    let half_res = lumit_render::Quality {
        auto_res: true,
        display_scale: 0.5,
        ..lumit_render::Quality::default()
    };
    assert_eq!(
        render(Some(half.clone()), 30, Some(half_res)),
        render(None, 15, Some(half_res)),
        "and the same at half preview resolution"
    );

    // A freeze: one source time for every layer time, so one picture for ever.
    let frozen = retime(&[(0, 0.25), (4, 0.25)]);
    let held = render(Some(frozen.clone()), 90, None);
    assert_eq!(
        held,
        render(None, 15, None),
        "a freeze holds the frame it is parked on"
    );
    assert_eq!(
        held,
        render(Some(frozen), 200, None),
        "and holds it however far the parent runs"
    );

    // Overrun: a map that runs past the nested comp holds its last frame,
    // rather than landing on the empty moment after it (docs/04 §11.3).
    let fast = retime(&[(0, 0.0), (1, 4.0)]);
    let past = render(Some(fast), 180, None);
    assert!(
        lit(&past),
        "an overrun holds a picture, it does not go dark"
    );
    assert_eq!(
        past,
        render(
            None,
            u64::from(FPS) * u64::try_from(INNER_S).unwrap() - 1,
            None
        ),
        "and the picture it holds is the nested comp's last frame"
    );

    // And with no map at all, nothing is drawn past the nested duration -
    // the behaviour that was there before this, pinned so it stays.
    assert!(
        !lit(&render(
            None,
            u64::from(FPS) * u64::try_from(INNER_S).unwrap() + 30,
            None
        )),
        "an un-retimed Precomp still draws nothing past its comp's end"
    );
}

/// The same map, down the two roads that read a comp through
/// `nested_comp_draw` rather than through the Precomp arm: an effect's Matte
/// row and a Light wrap's Background row. Both arrive on the one carriage
/// `layer-input.md` describes, so a map honoured in one and not the other would
/// be a matte from one moment over a picture from another.
#[test]
fn a_retimed_precomp_read_as_a_matte_or_a_background_follows_its_map() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };

    // The scene: a grey base carrying one effect, and above it a hidden
    // Precomp layer of the sliding square, which the effect reads.
    let build = |effect: &str, row: &str, map: Option<Property>| {
        let (doc, parent_id) = project(map);
        let mut doc = doc.as_ref().clone();
        let grey = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: grey,
            name: "grey".into(),
            colour: LinearColour([0.25, 0.25, 0.25, 1.0]),
            width: COMP,
            height: COMP,
            extra: serde_json::Map::new(),
        }));
        let parent = doc.comp_mut(parent_id).expect("the parent");
        // The source is read, never composited: left visible it would paint
        // over the very pixels the comparison reads.
        parent.layers[0].switches.visible = false;
        let source = parent.layers[0].id;
        let mut inst = lumit_core::fx::instantiate(effect).expect("a builtin");
        for p in &mut inst.params {
            if p.id == row {
                p.value = EffectValue::Layer(Some(source));
            }
            // Light wrap does nothing at all until its Width is opened.
            if p.id == "width" {
                p.value = EffectValue::Float(Property::fixed(24.0));
            }
            // And an Exposure gated by a matte has to lift something.
            if p.id == "stops" {
                p.value = EffectValue::Float(Property::fixed(2.0));
            }
        }
        let mut base = layer("base", LayerKind::Solid { def: grey }, 4);
        // A mask, so the foreground's own picture has an alpha edge inside it:
        // Light wrap reaches in from an edge, and a plate that fills its own
        // texture gives it none to reach in from.
        base.masks = vec![lumit_core::mask::Mask::ellipse(
            f64::from(COMP) / 2.0,
            f64::from(COMP) / 2.0,
            20.0,
            20.0,
        )];
        base.effects = vec![inst];
        parent.layers.push(base);
        (Arc::new(doc), parent_id)
    };

    let half = retime(&[(0, 0.0), (2, 1.0)]);
    for (effect, row) in [
        ("exposure", lumit_core::fx::MATTE_PARAM),
        ("light_wrap", "background"),
    ] {
        let mut render = |map: Option<Property>, frame: u64| {
            let (doc, comp) = build(effect, row, map);
            r.render_rgba(&doc, comp, frame, 1.0).expect("the render").0
        };
        assert_eq!(
            render(Some(half.clone()), 30),
            render(None, 15),
            "{effect}'s {row} must read the moment the map points at"
        );
        assert_ne!(
            render(Some(half.clone()), 30),
            render(None, 30),
            "{effect}'s {row} would otherwise be reading the layer's own clock"
        );
    }
}
