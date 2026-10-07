//! The style model's own rules (docs/impl/layer-styles.md §9): the order and
//! the one-of-each cap, what a style declaration may and may not carry, the two
//! new uniforms on the shared drop-shadow core, and the promise that a layer
//! with no styles is the file and the picture it always was.

use super::*;
use crate::fx::{cpu, EffectMetadata, ParamKind};
use crate::model::{EffectInstance, EffectValue};

fn style(name: &str) -> EffectInstance {
    crate::fx::instantiate(name).unwrap_or_else(|| panic!("{name} is a declared style"))
}

/// A hand-shuffled list — the shape a file written by another tool, or edited
/// by hand, arrives in — comes back one-of-each and in order.
#[test]
fn normalising_dedupes_and_restores_the_pinned_order() {
    let mut list = vec![
        style("style_stroke"),
        style("style_colour_overlay"),
        style("style_drop_shadow"),
        // A second Colour overlay: the invariant is at most one per style, and
        // the *first* is the one kept, so an id can be followed across the call.
        style("style_colour_overlay"),
        style("style_outer_glow"),
    ];
    let kept = list[1].id;
    normalise_styles(&mut list);
    assert_eq!(
        list.iter()
            .map(|s| s.effect.match_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "style_drop_shadow",
            "style_outer_glow",
            "style_colour_overlay",
            "style_stroke"
        ]
    );
    assert_eq!(
        list[2].id, kept,
        "the duplicate that goes is the later one, not the one already there"
    );
    assert_eq!(
        outer_prefix(&list),
        2,
        "sorted, the outer styles are exactly the leading run"
    );
}

/// The degrade rule reaches styles too: a tenth style from a newer Lumit is
/// kept, sorted to the end, and renders as identity — never thrown away.
#[test]
fn an_unknown_style_survives_and_sorts_last() {
    let mut future = style("style_stroke");
    future.effect.match_name = "style_from_the_future".into();
    let mut list = vec![future, style("style_drop_shadow")];
    normalise_styles(&mut list);
    assert_eq!(list.len(), 2, "nothing is discarded");
    assert_eq!(list[0].effect.match_name, "style_drop_shadow");
    assert_eq!(list[1].effect.match_name, "style_from_the_future");
    assert!(!style_is_outer("style_from_the_future"));
}

/// **The rule that keeps the render's parallel lists 1:1 without a slot per
/// style** (§3). `build.rs` fills its matte, layer-input, mask-path and points
/// lists by walking `layer.effects` only, and `run_ops` advances each counter on
/// the *op's own schema*. A style that declared any of those rows would advance
/// a counter nothing had filled and hand the next effect somebody else's picture
/// — so no style declares one, and this is where that is enforced.
#[test]
fn no_style_declares_a_row_the_render_would_have_to_fill() {
    for def in all() {
        let s = def.schema();
        let name = s.match_name;
        assert!(
            s.matte.param().is_none(),
            "{name} declares a Matte; the injected row is suppressed on \
             styles (matte = false) because a style dresses the layer's own alpha"
        );
        assert!(
            s.layer_input().is_none(),
            "{name} declares a layer input, which the style seam fills no slot for"
        );
        assert_eq!(
            s.mask_path_count(),
            0,
            "{name} declares a mask-path row, which the style seam fills no slot for"
        );
        assert!(
            !crate::fx::points::wants_carriage(def.signature()),
            "{name} declares a points port, which the style seam fills no slot for"
        );
        assert!(
            !s.params
                .iter()
                .any(|p| matches!(p.kind, ParamKind::File { .. })),
            "{name} declares a file row, which the style seam loads nothing for"
        );
    }
}

/// A premultiplied mid-grey square in the middle of an otherwise empty image.
///
/// Grey rather than white so an overlay's default white is visibly a change:
/// a white square under a white overlay is the one picture where "did anything
/// happen" cannot be answered.
fn square(w: u32, h: u32, alpha: f32) -> Vec<f32> {
    let mut px = vec![0.0f32; (w * h * 4) as usize];
    for y in (h / 4)..(3 * h / 4) {
        for x in (w / 4)..(3 * w / 4) {
            let d = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                px[d + c] = 0.25 * alpha;
            }
            px[d + 3] = alpha;
        }
    }
    px
}

/// Spread at 100 % is a **hard-edged** shadow: the gaussian's ramp is re-cut at
/// its half-way line, so almost every shadow pixel is either absent or at full
/// opacity, where the same shadow at Spread 0 is mostly ramp.
#[test]
fn spread_at_full_hardens_the_shadow() {
    let (w, h) = (32u32, 32u32);
    let opacity = 0.5f32;
    let params = |spread: f32| cpu::DropShadowParams {
        colour: [0.0, 0.0, 0.0],
        opacity,
        // Straight down-right, far enough clear of the square that the band the
        // shadow lands on is shadow and nothing else.
        offset: [6.0, 6.0],
        softness_px: 6.0,
        shadow_only: true,
        mix: 1.0,
        spread_scale: cpu::spread_scale(spread),
        knockout: false,
        invert: false,
        inner: false,
    };
    // Shadow only, so the alpha that comes back IS the shadow's coverage.
    let ramps = |spread: f32| {
        let mut px = square(w, h, 1.0);
        cpu::drop_shadow(&mut px, w, h, &params(spread));
        px.chunks_exact(4)
            .filter(|p| p[3] > 0.05 * opacity && p[3] < 0.95 * opacity)
            .count()
    };
    let soft = ramps(0.0);
    let hard = ramps(100.0);
    assert!(soft > 100, "a Spread 0 shadow is mostly ramp, got {soft}");
    assert!(
        hard * 8 < soft,
        "Spread 100 must be a hard edge: {hard} ramp pixels against {soft}"
    );
}

/// Colour overlay recolours what is there and **adds no pixels outside the
/// layer's alpha** — the property that makes it an interior style rather than
/// something that grows the shape (§9).
#[test]
fn a_colour_overlay_adds_no_pixels_outside_the_alpha() {
    let (w, h) = (16u32, 16u32);
    let before = square(w, h, 1.0);
    let mut after = before.clone();
    let inst = style("style_colour_overlay");
    let ops = crate::fx::resolve_stack(
        std::slice::from_ref(&inst),
        0.0,
        1000.0,
        1.0,
        &crate::fx::MarkerContext::NONE,
        std::sync::Arc::new(crate::expression::ExpressionContext::detached()),
    );
    assert_eq!(ops.len(), 1, "a style resolves through the ordinary walk");
    cpu::apply_stack(&mut after, w, h, &ops);
    for (i, (a, b)) in before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .enumerate()
    {
        assert_eq!(a[3], b[3], "pixel {i}: the alpha is never touched");
        if a[3] == 0.0 {
            assert_eq!(
                [b[0], b[1], b[2]],
                [0.0, 0.0, 0.0],
                "pixel {i} is outside the shape and must stay empty"
            );
        }
    }
    assert!(
        before != after,
        "the default white overlay must actually change the grey square"
    );
}

/// A layer with no styles writes **no `styles` key at all**, and one written
/// before the field existed reads back empty — the two halves of compatibility
/// for a new field.
#[test]
fn an_empty_style_list_leaves_the_file_exactly_as_it_was() {
    let mut layer = crate::model::Layer {
        id: uuid::Uuid::now_v7(),
        name: "plate".into(),
        kind: crate::model::LayerKind::Null,
        in_point: crate::time::CompTime(crate::time::Rational::ZERO),
        out_point: crate::time::CompTime(crate::time::Rational::new(1, 1).unwrap()),
        start_offset: crate::time::CompTime(crate::time::Rational::ZERO),
        transform: crate::model::TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        markers: Vec::new(),
        volume_db: crate::anim::Property::zero(),
        pan: crate::anim::Property::zero(),
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
        graph: Default::default(),
        switches: crate::model::Switches::default(),
        extra: serde_json::Map::new(),
    };
    let bare = serde_json::to_string(&layer).unwrap();
    assert!(
        !bare.contains("styles"),
        "an empty style list must not reach the file: {bare}"
    );
    let back: crate::model::Layer = serde_json::from_str(&bare).unwrap();
    assert_eq!(
        back, layer,
        "and a file with no styles key reads back empty"
    );

    layer.styles = vec![style("style_colour_overlay")];
    let dressed = serde_json::to_string(&layer).unwrap();
    assert!(dressed.contains("style_colour_overlay"));
    let back: crate::model::Layer = serde_json::from_str(&dressed).unwrap();
    assert_eq!(back, layer, "and a styled layer round-trips whole");
    assert!(matches!(
        back.styles[0].param("mix"),
        Some(EffectValue::Float(_))
    ));
}

// ---------------------------------------------------------------------------
// The five kernels of §10's second package (docs/impl/layer-styles.md §9).
//
// These read the typed structs straight, rather than through `resolve_stack`,
// because what is under test is the arithmetic each style packs — the walk that
// carries it is already pinned above, and going through it twice would only make
// a kernel failure read as a resolution failure.
// ---------------------------------------------------------------------------

use crate::fx::styles::defs::{GradientOverlay, InnerShadow, StrokeStyle};
use crate::fx::Params;

/// **An inner style stays inside the shape** (§9). Both halves matter: the
/// alpha is never touched, and no pixel outside the layer's own coverage is
/// written — which is what makes Inner shadow an *interior* style rather than a
/// second shadow with the sign flipped.
#[test]
fn an_inner_shadow_leaves_no_pixel_outside_the_shape() {
    let (w, h) = (32u32, 32u32);
    let before = square(w, h, 1.0);
    let mut after = before.clone();
    let mut inner = InnerShadow::read(Params::EMPTY);
    inner.opacity = 100.0;
    inner.distance = 5.0;
    inner.softness = 4.0;
    cpu::drop_shadow(&mut after, w, h, &inner.packed());

    for (i, (a, b)) in before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .enumerate()
    {
        assert_eq!(a[3], b[3], "pixel {i}: an interior style never moves alpha");
        if a[3] == 0.0 {
            assert_eq!(
                [b[0], b[1], b[2]],
                [0.0, 0.0, 0.0],
                "pixel {i} is outside the shape and must stay empty"
            );
        }
    }
    assert_ne!(before, after, "and it darkened something");
    // The dark band lands on the side the light comes FROM: the default 135°
    // throws the shadow down-and-right, so the inside of the TOP-LEFT edge is
    // where the shape's own darkening shows.
    let at = |x: u32, y: u32| after[((y * w + x) * 4) as usize];
    let (near, far) = (at(w / 4 + 1, h / 4 + 1), at(3 * w / 4 - 2, 3 * h / 4 - 2));
    assert!(
        near < far,
        "the inner shadow must darken the light-ward edge more than the far one: \
         {near} against {far}"
    );
}

/// **Stroke's Position is the whole of which side the thickness lands on** (§9):
/// Outside adds nothing inside the shape, Inside adds nothing outside it, and
/// both actually draw a band. Arithmetic rather than a clip — the fat copy is
/// never smaller than the shape and the thin one never larger.
#[test]
fn stroke_outside_adds_nothing_inside_and_inside_nothing_outside() {
    let (w, h) = (32u32, 32u32);
    let before = square(w, h, 1.0);
    let run = |position: u32| {
        let mut s = StrokeStyle::read(Params::EMPTY);
        s.position = position;
        s.size = 3.0;
        s.opacity = 100.0;
        s.stroke_colour = [1.0, 0.0, 0.0, 1.0];
        let mut px = before.clone();
        cpu::stroke_contour(&mut px, w, h, &s.packed());
        px
    };
    let changed = |after: &[f32], want_inside: bool| {
        let mut hits = 0usize;
        for (i, (a, b)) in before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .enumerate()
        {
            if a == b {
                continue;
            }
            hits += 1;
            assert_eq!(
                a[3] > 0.0,
                want_inside,
                "pixel {i} changed on the wrong side of the edge"
            );
        }
        hits
    };
    assert!(changed(&run(0), false) > 0, "Outside must draw a band");
    assert!(changed(&run(1), true) > 0, "Inside must draw one too");

    // Centre straddles: it has to touch both sides, which is what makes it a
    // third position rather than a rounding of one of the other two.
    let centre = run(2);
    let (mut out_hits, mut in_hits) = (0usize, 0usize);
    for (a, b) in before.chunks_exact(4).zip(centre.chunks_exact(4)) {
        if a != b {
            if a[3] > 0.0 {
                in_hits += 1;
            } else {
                out_hits += 1;
            }
        }
    }
    assert!(
        out_hits > 0 && in_hits > 0,
        "Centre must straddle the edge: {out_hits} outside, {in_hits} inside"
    );
}

/// **Gradient overlay is the ramp clipped to the coverage** (§4): it recolours
/// the shape, leaves the alpha alone, adds nothing outside, and Reverse turns
/// the ramp round without moving it.
#[test]
fn a_gradient_overlay_is_clipped_to_the_alpha_and_reverses_in_place() {
    let (w, h) = (32u32, 32u32);
    let before = square(w, h, 1.0);
    let run = |reverse: bool| {
        let mut g = GradientOverlay::read(Params::EMPTY);
        g.reverse = reverse;
        let p = g.packed(w, h);
        assert!(p.clip_to_alpha, "an overlay is not a generator");
        let mut px = before.clone();
        cpu::gradient(&mut px, w, h, &p);
        px
    };
    let plain = run(false);
    let reversed = run(true);

    for (i, (a, b)) in before
        .chunks_exact(4)
        .zip(plain.chunks_exact(4))
        .enumerate()
    {
        assert_eq!(a[3], b[3], "pixel {i}: the overlay never touches alpha");
        if a[3] == 0.0 {
            assert_eq!(
                [b[0], b[1], b[2]],
                [0.0, 0.0, 0.0],
                "pixel {i} is outside the shape and must stay empty"
            );
        }
    }
    // The default angle runs the ramp top to bottom, from Colour A (white) to
    // Colour B (black); Reverse swaps the ends and leaves the axis where it was.
    let at = |px: &[f32], y: u32| px[((y * w + w / 2) * 4) as usize];
    let (top, bottom) = (h / 4 + 1, 3 * h / 4 - 2);
    assert!(
        at(&plain, top) > at(&plain, bottom),
        "the ramp runs light to dark down the layer"
    );
    assert!(
        at(&reversed, top) < at(&reversed, bottom),
        "and Reverse turns it round"
    );
}
