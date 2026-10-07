//! **Layer styles, end to end** (docs/impl/layer-styles.md §9).
//!
//! # In plain terms
//!
//! The unit tests in `lumit-core` prove the model and the kernels: the order,
//! the one-of-each cap, the uniforms on the shared drop-shadow core, the
//! stroke's two morphological copies. They never render anything. This file
//! does — it builds documents the way a user would, dresses a layer with real
//! styles, and pushes them through the same public entries the Viewer and the
//! exporter use.
//!
//! What it is here to catch, all of it silent if it broke:
//!
//! - **The seam.** A style's ops have to be appended to the layer's effect ops
//!   and run on the same raster. If the second resolve walk were dropped, every
//!   assertion about pixels would still *render* — just without the style.
//! - **The order.** Interior styles have to run before the outer ones, or a
//!   Colour overlay floods the shadow the Drop shadow just laid down. That is a
//!   picture, not a crash. §2's own order is pinned the same way: the Colour
//!   overlay covers the Gradient overlay, the Drop shadow sits under the Outer
//!   glow, the Stroke draws over the interiors.
//! - **The name.** A style is picture, so editing one has to change the frame
//!   key and taking the last one off has to give the old key back — otherwise
//!   the cache serves the undressed frame for the dressed layer.
//! - **The two paths agreeing.** The CPU reference is the oracle (docs/08 §1.6),
//!   and a style is only shipped when the kernel matches it.
//! - **Nothing at all, for a layer with no styles.** The field's serde default
//!   must leave the file and the picture as they were.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, Layer, LayerKind, LinearColour,
    ProjectItem, SolidDef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;
/// The dressed solid's own size: a square well inside the frame, so a shadow
/// has somewhere to fall and the assertions have empty ground to read.
const BOX: u32 = 24;

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

/// A fresh style with `edits` applied to its rows.
fn style(name: &str, edits: &[(&str, f64)]) -> EffectInstance {
    let mut inst =
        lumit_core::fx::instantiate(name).unwrap_or_else(|| panic!("{name} is a declared style"));
    for (id, value) in edits {
        for p in &mut inst.params {
            if p.id == *id {
                p.value = EffectValue::Float(lumit_core::anim::Property::fixed(*value));
            }
        }
    }
    inst
}

/// A document whose one visible layer is a **precomp** holding a small grey
/// square, dressed with `styles`.
///
/// The wrapping comp is not decoration. A style runs on the layer's own raster,
/// and a solid's raster is the solid — a 24 × 24 texture with no empty ground
/// anywhere on it, so a shadow thrown ten pixels down-right would land entirely
/// off the edge and the test would be measuring the raster's bounds rather than
/// the style. (Nothing pads a `FullFrame` effect's raster today; the Drop shadow
/// *effect* has always behaved the same way, and the note names the shared
/// padding path as the place that changes when one arrives.) Wrapping the square
/// in a full-size comp gives the layer a 64 × 64 raster with real geometry on
/// it, which is how a layer like this is authored anyway.
fn project(styles: Vec<EffectInstance>) -> (Arc<Document>, Uuid) {
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items
        .push(solid(def, "box", [0.25, 0.25, 0.25, 1.0], BOX, BOX));
    let inner = comp_of("box comp", vec![layer("box", LayerKind::Solid { def })]);
    let inner_id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));

    let mut l = layer("dressed", LayerKind::Precomp { comp: inner_id });
    l.styles = styles;
    let comp = comp_of("Comp", vec![l]);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

/// The red channel at (x, y).
fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
    rgba[((y * w + x) * 4) as usize]
}

/// The whole pixel at (x, y) — needed wherever "did the colour change" is the
/// question, because a **black** shadow over a black background is 0 in every
/// channel whether it is there or not, and a test that read one channel would
/// pass while rendering nothing.
fn rgb(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let d = ((y * w + x) * 4) as usize;
    [rgba[d], rgba[d + 1], rgba[d + 2]]
}

/// Paint a style's colour row — the styles are declared with different ids for
/// it (a shadow's colour is not an overlay's), so the row is named per call.
fn tint(inst: &mut EffectInstance, id: &str, colour: [f64; 4]) {
    for p in &mut inst.params {
        if p.id == id {
            p.value = EffectValue::Colour(colour.map(lumit_core::anim::Property::fixed));
        }
    }
}

/// **Nothing at all.** A layer that wears no styles saves without the key and
/// renders the frame it always rendered — the whole cost of the new field,
/// measured.
#[test]
fn a_layer_with_no_styles_is_the_file_and_the_frame_it_always_was() {
    let (doc, comp) = project(Vec::new());
    let json = serde_json::to_string(&*doc).unwrap();
    assert!(
        !json.contains("styles"),
        "an empty style list must leave no trace in the file"
    );
    // And a document written before the field existed loads to the same thing.
    let older: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(&older, &*doc);

    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (with, w, h) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
    let (again, ..) = r
        .render_rgba(&Arc::new(older), comp, 0, 1.0)
        .expect("the reloaded document");
    assert_eq!((w, h), (COMP, COMP));
    assert_eq!(with, again, "the same bytes, both ways round");
}

/// A hard, opaque, **red** shadow thrown down-and-right well clear of the
/// square. Red rather than the default black because the comp's background is
/// black, and "is the shadow there" is not a question a black shadow over black
/// can answer.
fn red_shadow() -> EffectInstance {
    let mut s = style(
        "style_drop_shadow",
        &[
            ("opacity", 100.0),
            ("distance", 10.0),
            ("softness", 0.0),
            ("direction", 135.0),
        ],
    );
    tint(&mut s, "shadow_colour", [1.0, 0.0, 0.0, 1.0]);
    s
}

/// **The order.** A Colour overlay recolours the layer and leaves the Drop
/// shadow alone. Run in the stored (painting) order the shadow would be laid
/// down first and the overlay would flood it too, which is exactly the picture
/// this asserts against.
#[test]
fn an_interior_style_does_not_paint_the_outer_style_it_sits_above() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // A green overlay over a red shadow: if the overlay flooded the shadow the
    // shadow would come back green, which is a difference no tolerance hides.
    let mut overlay = style("style_colour_overlay", &[]);
    tint(&mut overlay, "colour", [0.0, 1.0, 0.0, 1.0]);
    let (shadow_only, comp_a) = project(vec![red_shadow()]);
    let (both, comp_b) = project(vec![red_shadow(), overlay]);
    let (a, w, _) = r.render_rgba(&shadow_only, comp_a, 0, 1.0).unwrap();
    let (b, ..) = r.render_rgba(&both, comp_b, 0, 1.0).unwrap();

    let (sx, sy) = (BOX + 4, BOX + 4);
    assert!(
        px(&a, w, sx, sy) > 80,
        "the shadow must be visibly red at ({sx}, {sy}), or the comparison below \
         is two empty pixels agreeing — got {:?}",
        rgb(&a, w, sx, sy)
    );
    assert_eq!(
        rgb(&b, w, sx, sy),
        rgb(&a, w, sx, sy),
        "the shadow at ({sx}, {sy}) must be the colour it was with the overlay on: \
         the interior style ran on the layer, not on the shadow"
    );
    // The square, on the other hand, is now green.
    let inside = (BOX / 2, BOX / 2);
    let [_, g_before, _] = rgb(&a, w, inside.0, inside.1);
    let [_, g_after, _] = rgb(&b, w, inside.0, inside.1);
    assert!(
        g_after > g_before + 20,
        "the overlay must actually have greened the layer: {g_before} to {g_after}"
    );
}

/// **The name.** A style is picture: editing one has to rename the frame, and
/// taking the last one off has to give the undressed name back.
#[test]
fn a_style_edit_renames_the_frame_and_removing_it_restores_the_name() {
    struct Stub;
    impl lumit_eval::SourceStamper for Stub {
        fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((format!("stub:{item}"), (lt * 60.0).round().max(0.0) as u64))
        }
    }
    let key = |styles: Vec<EffectInstance>| {
        let (doc, comp) = project(styles);
        let comp = doc
            .items
            .iter()
            .find_map(|i| match i {
                ProjectItem::Composition(c) if c.id == comp => Some(c.clone()),
                _ => None,
            })
            .unwrap();
        lumit_eval::comp_frame_key(&doc, &comp, 0.0, lumit_eval::Quality::default(), &Stub)
            .expect("a solid is always keyable")
    };
    let bare = key(Vec::new());
    let dressed = key(vec![style("style_colour_overlay", &[])]);
    let edited = key(vec![style("style_colour_overlay", &[("mix", 40.0)])]);
    assert_ne!(bare, dressed, "adding a style must rename the frame");
    assert_ne!(dressed, edited, "editing one must rename it again");
    assert_eq!(
        bare,
        key(Vec::new()),
        "and the undressed name is the name it always was — deterministic, twice"
    );
}

/// **The two paths agree**, for every shipped style, through the real seam: the
/// draw builder resolves them, `run_ops` runs the GPU kernels, `cpu::apply_stack`
/// runs the references, and the two pictures are the same within the fp16
/// working format's own tolerance.
#[test]
fn cpu_and_gpu_agree_on_every_shipped_style() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 32u32);
    // A premultiplied grey square on empty ground, so there is an alpha edge for
    // the shadow to be cast from and empty ground for it to fall on.
    let mut source = vec![0.0f32; (w * h * 4) as usize];
    for y in 8..24 {
        for x in 8..24 {
            let d = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                source[d + c] = 0.25;
            }
            source[d + 3] = 1.0;
        }
    }

    // One instance of each shipped style, each with its own controls off their
    // defaults so the kernel is asked a real question rather than a neutral one.
    let inner_glow_centre = {
        let mut s = style("style_inner_glow", &[("softness", 5.0)]);
        choose(&mut s, "source", 1);
        s
    };
    let stroke_centre = {
        let mut s = style("style_stroke", &[("size", 3.5), ("opacity", 80.0)]);
        choose(&mut s, "position", 2);
        s
    };
    for styles in [
        vec![style(
            "style_drop_shadow",
            &[("distance", 5.0), ("softness", 3.0), ("spread", 40.0)],
        )],
        vec![style(
            "style_outer_glow",
            &[("softness", 4.0), ("spread", 30.0)],
        )],
        vec![style(
            "style_gradient_overlay",
            &[("angle", 60.0), ("scale", 140.0)],
        )],
        vec![style("style_colour_overlay", &[("mix", 60.0)])],
        vec![style(
            "style_inner_glow",
            &[("softness", 5.0), ("choke", 20.0)],
        )],
        vec![inner_glow_centre],
        vec![style(
            "style_inner_shadow",
            &[("distance", 4.0), ("softness", 3.0)],
        )],
        vec![style("style_stroke", &[("size", 3.0)])],
        vec![stroke_centre],
    ] {
        let name = styles[0].effect.match_name.clone();
        let ops = lumit_core::fx::resolve_stack(
            &styles,
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 1, "{name} resolved to no op");

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = lumit_render::fxops::run_ops(
            fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &lumit_render::fxops::Side::NONE,
            &[],
            None,
            None,
        );
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
        let mut cpu = source.clone();
        lumit_core::fx::cpu::apply_stack(&mut cpu, w, h, &ops);

        let mut worst = 0.0f32;
        for (g, c) in gpu.iter().zip(cpu.iter()) {
            worst = worst.max((g - c).abs());
        }
        assert!(
            worst < 0.01,
            "{name}: the GPU and the CPU reference disagree by {worst}"
        );
        assert_ne!(
            gpu, source,
            "{name}: the kernel must have done something, or the agreement is vacuous"
        );
    }
}

// ---------------------------------------------------------------------------
// §2's order, pinned in pixels (docs/impl/layer-styles.md §9).
//
// The order is the whole promise of a layer style: everyone's muscle memory
// expects Photoshop's, and getting it wrong is not a crash but a picture that is
// subtly wrong in a way nobody can name. Each test below picks two styles whose
// disagreement is a colour, so a swapped order fails on a channel rather than on
// a tolerance.
// ---------------------------------------------------------------------------

/// Set one of a style's Choice rows — Position, Source, the gradient's Type.
fn choose(inst: &mut EffectInstance, id: &str, value: u32) {
    for p in &mut inst.params {
        if p.id == id {
            p.value = EffectValue::Choice(value);
        }
    }
}
