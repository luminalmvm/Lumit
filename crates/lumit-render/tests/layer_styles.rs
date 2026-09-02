//! **Layer styles, end to end** (docs/impl/layer-styles.md §9, K-706).
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
//! - **Nothing at all, for a layer with no styles.** The K-258 regression: the
//!   field's serde default must leave the file and the picture as they were.

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
        master_volume_db: 0.0,
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
/// it, which is the authoring route K-266 already describes.
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

/// **K-258.** A layer that wears no styles saves without the key and renders the
/// frame it always rendered — the whole cost of the new field, measured.
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

/// **The seam.** A Drop shadow style paints outside the layer's own alpha, on a
/// layer carrying no effects at all — which is only possible if the second
/// resolve walk ran and its ops were appended to the (empty) effect stack.
#[test]
fn a_drop_shadow_style_paints_outside_the_layer() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (dressed, comp_a) = project(vec![red_shadow()]);
    let (bare, comp_b) = project(Vec::new());
    let (with, w, _) = r.render_rgba(&dressed, comp_a, 0, 1.0).unwrap();
    let (without, ..) = r.render_rgba(&bare, comp_b, 0, 1.0).unwrap();

    // A point past the square's lower-right corner, inside the shadow's throw.
    let (sx, sy) = (BOX + 4, BOX + 4);
    assert_eq!(
        rgb(&without, w, sx, sy),
        rgb(&without, w, COMP - 1, COMP - 1),
        "the undressed render must be empty there, or the test proves nothing"
    );
    assert!(
        px(&with, w, sx, sy) > 80,
        "the shadow must reach ({sx}, {sy}), which is outside the layer — got {:?}",
        rgb(&with, w, sx, sy)
    );
    // And the square itself is untouched: a shadow goes *under*.
    assert_eq!(
        rgb(&with, w, BOX / 2, BOX / 2),
        rgb(&without, w, BOX / 2, BOX / 2),
        "the layer's own pixels sit over its shadow"
    );
    // Determinism (§9): the same document at the same frame is the same bytes.
    let (again, ..) = r.render_rgba(&dressed, comp_a, 0, 1.0).unwrap();
    assert_eq!(with, again, "a styled frame renders the same twice");
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

/// **The invariants are restored on load** (§1): a project whose style list was
/// written out of order — by hand, or by a tool that did not know §2's order —
/// comes back one-of-each and sorted.
#[test]
fn a_shuffled_style_list_comes_back_in_order_from_a_saved_project() {
    let (doc, comp) = project(vec![
        style("style_stroke", &[]),
        style("style_drop_shadow", &[]),
        style("style_colour_overlay", &[]),
        // A duplicate, which the cap forbids.
        style("style_colour_overlay", &[]),
    ]);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("shuffled.lum");
    lumit_project::save(&doc, &path).expect("save");
    let (back, _) = lumit_project::open(&path).expect("open");
    let comp = back
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.id == comp => Some(c),
            _ => None,
        })
        .expect("the comp survives the round trip");
    assert_eq!(
        comp.layers[0]
            .styles
            .iter()
            .map(|s| s.effect.match_name.as_str())
            .collect::<Vec<_>>(),
        vec!["style_drop_shadow", "style_colour_overlay", "style_stroke"],
        "the load path restores §2's order and the one-of-each cap"
    );
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

/// **§2 entries 4 and 5.** The Colour overlay sits *over* the Gradient overlay,
/// so a flooding colour overlay leaves the interior flat and the ramp underneath
/// it is gone. Run the other way round the ramp would win and the interior would
/// still be graded.
#[test]
fn a_colour_overlay_covers_the_gradient_overlay_beneath_it() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut gradient = style("style_gradient_overlay", &[]);
    tint(&mut gradient, "colour_a", [1.0, 0.0, 0.0, 1.0]);
    tint(&mut gradient, "colour_b", [0.0, 0.0, 1.0, 1.0]);
    let mut overlay = style("style_colour_overlay", &[]);
    tint(&mut overlay, "colour", [0.0, 1.0, 0.0, 1.0]);

    let (ramp_only, comp_a) = project(vec![gradient.clone()]);
    let (both, comp_b) = project(vec![gradient, overlay]);
    let (a, w, _) = r.render_rgba(&ramp_only, comp_a, 0, 1.0).unwrap();
    let (b, ..) = r.render_rgba(&both, comp_b, 0, 1.0).unwrap();

    // Two points down the middle of the square, well inside it.
    let (top, bottom) = ((BOX / 2, 2), (BOX / 2, BOX - 3));
    assert_ne!(
        rgb(&a, w, top.0, top.1),
        rgb(&a, w, bottom.0, bottom.1),
        "the ramp alone must actually grade the interior, or this proves nothing"
    );
    assert_eq!(
        rgb(&b, w, top.0, top.1),
        rgb(&b, w, bottom.0, bottom.1),
        "the colour overlay covers the gradient overlay: the interior is flat"
    );
    let [red, green, blue] = rgb(&b, w, top.0, top.1);
    assert!(
        green > red && green > blue,
        "and it is the overlay's green, not either end of the ramp: {:?}",
        [red, green, blue]
    );
}

/// **§2 entries 1 and 2.** The Drop shadow is furthest back and the Outer glow
/// sits in front of it, so against the layer's own edge — where both are at
/// their strongest — the glow's colour is the one that reads and the shadow is
/// the one that has given ground.
///
/// This is the assertion the seam's reversed outer run exists for: the ops are
/// emitted glow-then-shadow precisely so that the *shadow* ends up underneath.
///
/// **Read it right against the edge.** The two outer styles run on one raster,
/// so the second one blurs an alpha the first has already fattened and therefore
/// reaches further out than it would alone. Far from the shape that extra reach
/// wins on its own and the two colours cross over; hard against the edge, where
/// both coverages are near their peak, being in front is the only thing that
/// decides. Moving this sample outward is how the test stops meaning anything.
#[test]
fn the_drop_shadow_sits_under_the_outer_glow() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // Both at zero throw and the same softness, so they cover the same ground
    // outside the layer and the only question left is which is on top.
    let shadow = red_shadow_at_zero_throw();
    let mut glow = style("style_outer_glow", &[("opacity", 100.0), ("softness", 6.0)]);
    tint(&mut glow, "glow_colour", [0.0, 1.0, 0.0, 1.0]);

    let (both, comp_a) = project(vec![shadow.clone(), glow]);
    let (shadow_only, comp_b) = project(vec![shadow]);
    let (a, w, _) = r.render_rgba(&both, comp_a, 0, 1.0).unwrap();
    let (b, ..) = r.render_rgba(&shadow_only, comp_b, 0, 1.0).unwrap();

    // The first pixel past the square's right edge.
    let (sx, sy) = (BOX, BOX / 2);
    let [red, green, _] = rgb(&a, w, sx, sy);
    let alone = rgb(&b, w, sx, sy)[0];
    assert!(
        green > 20 && red > 20 && alone > 20,
        "both styles must reach ({sx}, {sy}), or these are zeroes agreeing"
    );
    assert!(
        green > red,
        "the glow is in front: green {green} must beat red {red}"
    );
    assert!(
        red < alone,
        "and the shadow, being behind the glow, has given ground: {red} against \
         {alone} with no glow above it"
    );
}

/// The order test's shadow on its own — thrown nowhere, so it sits squarely
/// under the glow rather than beside it.
fn red_shadow_at_zero_throw() -> EffectInstance {
    let mut s = style(
        "style_drop_shadow",
        &[("opacity", 100.0), ("distance", 0.0), ("softness", 6.0)],
    );
    tint(&mut s, "shadow_colour", [1.0, 0.0, 0.0, 1.0]);
    s
}

/// **§2 entry 9.** The Stroke draws over the interiors, so a stroke laid on a
/// layer already flooded by a Colour overlay is still visible.
#[test]
fn the_stroke_draws_over_the_interior_styles() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut overlay = style("style_colour_overlay", &[]);
    tint(&mut overlay, "colour", [0.0, 1.0, 0.0, 1.0]);
    let mut stroke = style("style_stroke", &[("size", 3.0), ("opacity", 100.0)]);
    tint(&mut stroke, "stroke_colour", [1.0, 0.0, 0.0, 1.0]);
    // Inside, so the band lands on ground the overlay has already flooded —
    // which is the point: an Outside stroke would prove nothing about order.
    choose(&mut stroke, "position", 1);

    let (doc, comp) = project(vec![overlay, stroke]);
    let (px, w, _) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
    // The square's RIGHT edge. Its left one is the raster's own border, where
    // the erode's clamp-to-edge sees the shape carry on and leaves no band —
    // the same edge policy the screen matte's shrink has always run on.
    let edge = rgb(&px, w, BOX - 2, BOX / 2);
    let middle = rgb(&px, w, BOX / 2, BOX / 2);
    assert!(
        edge[0] > edge[1],
        "the stroke's red must survive the overlay at the edge: {edge:?}"
    );
    assert!(
        middle[1] > middle[0],
        "and the middle is still the overlay's green: {middle:?}"
    );
}

/// **Padding at a reduced preview resolution** (§9). An outer style declares
/// `roi = FullFrame` and rides the Drop shadow effect's own padding path, so its
/// reach beyond the layer's rect has to survive being rendered at half size —
/// the px@comp resolve is what makes a 10 px throw still 10 comp-pixels when
/// every raster pixel is two of them.
#[test]
fn an_outer_styles_reach_survives_a_reduced_preview_resolution() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (doc, comp) = project(vec![red_shadow()]);
    let (half, w, h) = r.render_rgba(&doc, comp, 0, 0.5).unwrap();
    assert_eq!(
        (w, h),
        (COMP / 2, COMP / 2),
        "half the preview, half the raster"
    );
    // The full-size test reads the shadow at (BOX + 4, BOX + 4); the same point
    // in comp coordinates is half of that here.
    let (sx, sy) = ((BOX + 4) / 2, (BOX + 4) / 2);
    assert!(
        px(&half, w, sx, sy) > 60,
        "the shadow must still reach past the layer at half resolution — got {:?}",
        rgb(&half, w, sx, sy)
    );
    // And it still stops: a corner of the frame is empty either way.
    assert_eq!(
        rgb(&half, w, w - 1, h - 1),
        [0, 0, 0],
        "a FullFrame style is not a flood"
    );
}

/// **Satin and Bevel and emboss are invisible in v1** (§8), through the real
/// seam and on the GPU path — no pass, no fault, no black frame.
#[test]
fn the_two_unrendered_styles_change_no_pixel_of_a_real_frame() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (bare, comp_a) = project(Vec::new());
    let (dressed, comp_b) = project(vec![
        style("style_satin", &[]),
        style("style_bevel_emboss", &[]),
    ]);
    let (a, ..) = r.render_rgba(&bare, comp_a, 0, 1.0).unwrap();
    let (b, ..) = r.render_rgba(&dressed, comp_b, 0, 1.0).unwrap();
    assert_eq!(a, b, "a style with no kernel renders as the identity");
}
