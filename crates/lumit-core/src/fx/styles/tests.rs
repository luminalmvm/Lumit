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

/// Every style's match name, in §2's stored order.
fn names() -> Vec<&'static str> {
    all().map(|d| d.schema().match_name).collect()
}

/// §2's order is the whole of the pinned list, and the two outer styles lead it
/// — the property `outer_prefix` leans on to split without allocating.
#[test]
fn the_pinned_order_is_the_one_the_note_writes_down() {
    assert_eq!(
        names(),
        vec![
            "style_drop_shadow",
            "style_outer_glow",
            "style_gradient_overlay",
            "style_colour_overlay",
            "style_satin",
            "style_inner_glow",
            "style_inner_shadow",
            "style_stroke",
            "style_bevel_emboss",
        ]
    );
    for (i, name) in names().iter().enumerate() {
        assert_eq!(style_index(name), Some(i), "{name} knows where it sits");
        assert_eq!(
            style_is_outer(name),
            i < 2,
            "{name}: only Drop shadow and Outer glow paint outside the alpha"
        );
    }
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

/// K-258's degrade rule reaches styles too: a tenth style from a newer Lumit is
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
            "{name} declares a Matte; K-395's injected row is suppressed on \
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

/// Every style ends with the host-uniform Mix, so every style gets the injected
/// Blend choice beside it (K-425) — which is where a style's blend mode lives
/// (§1), and the reason no style needed a field of its own for one.
#[test]
fn every_style_carries_a_mix_and_the_blend_row_beside_it() {
    for def in all() {
        let s = def.schema();
        let at = s
            .params
            .iter()
            .position(|p| p.id == crate::fx::MIX_PARAM)
            .unwrap_or_else(|| panic!("{} has no Mix row", s.match_name));
        assert_eq!(
            s.params.get(at + 1).map(|p| p.id),
            Some(crate::fx::BLEND_PARAM),
            "{}: the Blend choice sits beside Mix",
            s.match_name
        );
    }
}

/// The names are prefixed and unique, which is what keeps a style out of the
/// catalogue's way: `fx::def` asks the catalogue first, and no effect can ever
/// answer to a `style_` name.
#[test]
fn style_names_are_prefixed_unique_and_unknown_to_the_catalogue() {
    let mut seen: Vec<&str> = Vec::new();
    for name in names() {
        assert!(name.starts_with("style_"), "{name} is not prefixed");
        assert!(!seen.contains(&name), "two styles answer to {name}");
        seen.push(name);
        assert!(
            super::super::BUILTIN_DEFS.get(name).is_none(),
            "{name} is in the effect catalogue, so the Add-effect search offers it"
        );
        assert!(
            crate::fx::def(name).is_some(),
            "{name} is not reachable through the one lookup"
        );
    }
}

/// A fresh instance of every style carries a value for every row it declares —
/// the same promise `instantiate` makes for an effect, checked here because
/// styles are born through the same call but from the other list.
#[test]
fn a_fresh_style_carries_every_declared_value() {
    for def in all() {
        let name = def.schema().match_name;
        let inst = style(name);
        for p in def.schema().params {
            if matches!(p.kind, ParamKind::Action) {
                continue;
            }
            assert!(
                inst.params.iter().any(|have| have.id == p.id),
                "{name} was born without {}",
                p.id
            );
        }
    }
}

/// Spread's slope is exactly 1 at nought, which is what makes a shadow with no
/// spread take no branch and stay the bytes it always was — and the Drop shadow
/// *effect* packs that neutral pair, so its kernel is untouched by K-706.
#[test]
fn spread_at_nought_is_the_neutral_the_effect_packs() {
    assert_eq!(cpu::spread_scale(0.0), 1.0);
    assert_eq!(cpu::spread_scale(-5.0), 1.0, "a negative spread is nought");
    assert!(
        cpu::spread_scale(100.0) > 1000.0,
        "full spread is a hard cut"
    );
    assert!(
        cpu::spread_scale(100.0).is_finite(),
        "and finite: no division by zero (docs/14 §4)"
    );

    let effect = crate::fx::effects::drop_shadow::DropShadow::read(crate::fx::Params::EMPTY);
    let packed = effect.packed();
    assert_eq!(packed.spread_scale, 1.0);
    assert!(!packed.knockout);
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

/// Layer knocks out shadow: on a **semi-transparent** layer the shape takes the
/// shadow away where it covers, and on an opaque one the two settings are the
/// same picture, because the composite already hides the shadow there.
#[test]
fn the_layer_knocks_the_shadow_out_only_where_it_is_transparent() {
    let (w, h) = (32u32, 32u32);
    let run = |alpha: f32, knockout: bool| {
        let mut px = square(w, h, alpha);
        cpu::drop_shadow(
            &mut px,
            w,
            h,
            &cpu::DropShadowParams {
                colour: [0.0, 0.0, 0.0],
                opacity: 1.0,
                // No offset, so the shadow sits exactly under the shape and the
                // question is only whether the shape removes it.
                offset: [0.0, 0.0],
                softness_px: 0.0,
                shadow_only: false,
                mix: 1.0,
                spread_scale: 1.0,
                knockout,
            },
        );
        // The middle of the square.
        px[(((h / 2) * w + w / 2) * 4) as usize + 3]
    };
    assert_eq!(
        run(1.0, true),
        run(1.0, false),
        "on an opaque layer the shadow is hidden behind the shape either way"
    );
    let half_off = run(0.5, false);
    let half_on = run(0.5, true);
    assert!(
        half_on < half_off - 0.05,
        "a half-transparent layer must show less shadow with the knockout on: \
         {half_on} against {half_off}"
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
/// before the field existed reads back empty — the two halves of K-258 for a
/// new field.
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
