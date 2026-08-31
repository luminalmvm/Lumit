//! **Layer styles** (docs/impl/layer-styles.md §7, K-706): After Effects'
//! `ADBE Layer Styles` group onto [`Layer::styles`](lumit_core::model::Layer).
//!
//! # In plain terms
//!
//! Photoshop's wardrobe — a shadow behind a layer, a glow around it, a colour
//! or a ramp across its face, a stroke round its edge — is a fixed family of
//! nine named slots rather than a stack of effects, and After Effects carries
//! it over unchanged. The walker already captured the whole group: every style,
//! every dial, every keyframe. What happens here is the **map**: each style
//! becomes an ordinary Lumit effect instance in the layer's second, order-locked
//! list, and its dials come across through exactly the machinery an effect's do
//! ([`Fx`](super::fx_colour::Fx)).
//!
//! Four things are worth knowing before reading the table below.
//!
//! **Only the styles After Effects reports as *on* are imported.** The DOM
//! lists all ten slots on any layer that has ever had the group, whether or not
//! the user added anything: the golden bundle's twenty-two styled layers carry
//! ten switched-off styles apiece, and importing those would put eighty
//! disabled instances on layers nobody dressed. There is no signal that tells
//! "added then switched off" from "never added", so the honest rule is the one
//! that matches what the user sees in After Effects.
//!
//! **The angle turns twice.** After Effects measures the *light* counter-
//! clockwise from the positive x axis; Lumit's style direction is measured from
//! straight up, clockwise; and a shadow slides *opposite* its light. So a
//! shadow's direction is `270° − a` (After Effects' 120° default becomes 150°,
//! down and to the right — which is where the default shadow is) and an
//! overlay's or a satin's, which names a direction rather than a light, is
//! `90° − a` with the opposition left out. Neither is reduced modulo 360:
//! angles wrap on the dial anyway, and a modulo applied key by key would tear
//! an animated one where it crossed.
//!
//! **Opacity crosses one for one.** §7 was written against the binary format's
//! 0..255 byte; the scripting DOM the bundle comes from hands per cent already
//! (the golden capture's defaults read 75 and 100, not 191 and 255), so a
//! rebase here would divide every shadow's darkness by two and a half. The
//! capture wins.
//!
//! **A style's blend mode is honoured on the interiors only.** The interior
//! styles paint *on* the layer's own pixels, which is exactly what the injected
//! Blend row does (K-425). The two outer styles composite *underneath*, where a
//! Screen or an Overlay has no meaning on this seam, so those keep Normal and
//! the report says so for anything After Effects had that is not Normal or
//! Multiply.

use lumit_core::model::{BlendMode, EffectInstance, EffectValue, TransformGroup};

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::fx_colour::Fx;
use super::props::{child, group};
use super::Conv;

/// What the report calls the family when a row is about the group rather than
/// about one style.
const FAMILY: &str = "Layer styles";

/// After Effects' `ADBE Layer Styles` group, mapped
/// (docs/impl/layer-styles.md §7).
///
/// Returns the layer's style list, already in §2's pinned painting order — the
/// order is restored here rather than trusted from the DOM, so a capture that
/// ever listed them differently still renders the way Photoshop does.
pub(crate) fn styles(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    props: &[Property],
    transform: &TransformGroup,
) -> Vec<EffectInstance> {
    let members = group(props, "ADBE Layer Styles");
    if members.is_empty() {
        return Vec::new();
    }

    // The comp-wide light, for the styles that follow it. After Effects hands
    // the *resolved* `localLightingAngle` whether or not Use Global Light is
    // on, so this is only needed to know it was read — but reading it here
    // keeps the fallback honest if a future capture stops resolving it.
    let global_angle = child(members, "ADBE Blend Options Group")
        .and_then(|g| super::props::still(g.children(), "ADBE Global Angle2", 0))
        .unwrap_or(120.0);

    let mut out = Vec::new();
    for node in members {
        let Some(name) = node.match_name.as_deref() else {
            continue;
        };
        // Every slot the DOM lists is present on a layer that has ever had the
        // group; only the ones switched on were actually added (see the module
        // note).
        if node.enabled != Some(true) {
            continue;
        }
        if let Some(style) = one(conv, path, node, name, global_angle) {
            out.push(style);
        }
    }

    if out.is_empty() {
        return out;
    }

    // §1's invariants, restored rather than assumed: one of each, in order.
    lumit_core::fx::normalise_styles(&mut out);

    // The Blending Options subgroup is v1's one wholly unmapped part of the
    // group — Fill Opacity, the per-channel switches, the blend ranges. Said
    // once, and only on a layer that actually wears a style, because a row
    // about a group nobody used is a row nobody can act on.
    if child(members, "ADBE Blend Options Group").is_some() {
        conv.report.row(
            path.property("Blending Options"),
            Outcome::Adjusted,
            Reason::EffectParamNotCarried {
                effect: FAMILY.to_string(),
                param: "Blending Options".to_string(),
            },
        );
    }

    // **The pre-transform deviation, said out loud** (§3). Lumit runs a
    // layer's styles on its own raster, before the transform photograph, so
    // they turn and scale with the layer where After Effects keeps them
    // screen-fixed. Invisible on the unrotated, uniformly scaled layers styles
    // overwhelmingly sit on — and this is the row for the ones that are not.
    if turned_or_squashed(transform) {
        conv.report.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::EffectDiffers {
                effect: FAMILY.to_string(),
                detail: "run before this layer's transform, so its rotation or non-uniform scale \
                         turns and stretches them, where After Effects keeps a style's shadow \
                         pointing the same way on screen"
                    .to_string(),
            },
        );
    }

    out
}

/// Whether §3's deviation is **visible** on this layer: it is turned, or its
/// two scale axes differ.
///
/// A keyframed rotation or scale counts however it reads at any one moment —
/// an animated one is exactly the case where the difference shows.
fn turned_or_squashed(t: &TransformGroup) -> bool {
    let still = |p: &lumit_core::anim::Property| match &p.animation {
        lumit_core::anim::Animation::Static(v) => Some(*v),
        _ => None,
    };
    match (still(&t.rotation), still(&t.scale_x), still(&t.scale_y)) {
        (Some(r), Some(x), Some(y)) => r.abs() > 1e-6 || (x - y).abs() > 1e-6,
        // Anything moving is a layer whose rotation or scale is not one number,
        // which is the deviation's own case.
        _ => true,
    }
}

/// One style, or `None` for a slot Lumit does not model.
fn one(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    ae_name: &str,
    global_angle: f64,
) -> Option<EffectInstance> {
    // AE names a style's own switch `<slot>/enabled` and its dials
    // `<slot>/<dial>`; the slot is the whole of the identity.
    let slot = ae_name.split('/').next().unwrap_or(ae_name);
    match slot {
        "dropShadow" => Some(drop_shadow(conv, path, node, global_angle)),
        "innerShadow" => Some(inner_shadow(conv, path, node, global_angle)),
        "outerGlow" => Some(outer_glow(conv, path, node)),
        "innerGlow" => Some(inner_glow(conv, path, node)),
        "solidFill" => Some(colour_overlay(conv, path, node)),
        "gradientFill" => Some(gradient_overlay(conv, path, node)),
        "frameFX" => Some(stroke(conv, path, node)),
        "chromeFX" => Some(satin(conv, path, node, global_angle)),
        "bevelEmboss" => Some(bevel(conv, path, node, global_angle)),
        // **Pattern Overlay is not one of the nine** (§1's family is
        // Photoshop's, minus the pattern that needs an image library Lumit has
        // no shape for). Named rather than dropped.
        "patternFill" => {
            conv.report.row(
                path.property("Pattern Overlay"),
                Outcome::Adjusted,
                Reason::EffectParamNotCarried {
                    effect: FAMILY.to_string(),
                    param: "Pattern Overlay".to_string(),
                },
            );
            None
        }
        _ => None,
    }
    .flatten()
}

// ---------------------------------------------------------------------------
// The nine
// ---------------------------------------------------------------------------

fn drop_shadow(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    global_angle: f64,
) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Drop shadow", "style_drop_shadow")?;
    fx.colour(conv, "dropShadow/color", "shadow_colour");
    fx.float(conv, "dropShadow/opacity", "opacity", 1.0, 0.0);
    light(&mut fx, conv, "dropShadow", "direction", global_angle);
    fx.float(conv, "dropShadow/distance", "distance", 1.0, 0.0);
    fx.float(conv, "dropShadow/blur", "softness", 1.0, 0.0);
    fx.float(conv, "dropShadow/chokeMatte", "spread", 1.0, 0.0);
    fx.toggle("dropShadow/layerConceals", "knockout");
    outer_blend(&mut fx, conv, "dropShadow/mode2");
    noise(&mut fx, conv, "dropShadow/noise");
    fx.done()
}

fn inner_shadow(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    global_angle: f64,
) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Inner shadow", "style_inner_shadow")?;
    fx.colour(conv, "innerShadow/color", "shadow_colour");
    fx.float(conv, "innerShadow/opacity", "opacity", 1.0, 0.0);
    light(&mut fx, conv, "innerShadow", "direction", global_angle);
    fx.float(conv, "innerShadow/distance", "distance", 1.0, 0.0);
    fx.float(conv, "innerShadow/blur", "softness", 1.0, 0.0);
    fx.float(conv, "innerShadow/chokeMatte", "choke", 1.0, 0.0);
    interior_blend(&mut fx, "innerShadow/mode2");
    noise(&mut fx, conv, "innerShadow/noise");
    fx.done()
}

fn outer_glow(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Outer glow", "style_outer_glow")?;
    glow_colour(&mut fx, conv, "outerGlow", "glow_colour");
    fx.float(conv, "outerGlow/opacity", "opacity", 1.0, 0.0);
    fx.float(conv, "outerGlow/blur", "softness", 1.0, 0.0);
    fx.float(conv, "outerGlow/chokeMatte", "spread", 1.0, 0.0);
    outer_blend(&mut fx, conv, "outerGlow/mode2");
    noise(&mut fx, conv, "outerGlow/noise");
    glow_extras(&mut fx, conv, "outerGlow");
    fx.done()
}

fn inner_glow(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Inner glow", "style_inner_glow")?;
    glow_colour(&mut fx, conv, "innerGlow", "glow_colour");
    fx.float(conv, "innerGlow/opacity", "opacity", 1.0, 0.0);
    fx.float(conv, "innerGlow/blur", "softness", 1.0, 0.0);
    fx.float(conv, "innerGlow/chokeMatte", "choke", 1.0, 0.0);
    // After Effects' Source lists Edge first and defaults to it, which is
    // Lumit's own first entry; anything else is Centre.
    fx.choice(conv, "innerGlow/innerGlowSource", "source", |v| match v {
        1 => (0, None),
        _ => (1, None),
    });
    interior_blend(&mut fx, "innerGlow/mode2");
    noise(&mut fx, conv, "innerGlow/noise");
    glow_extras(&mut fx, conv, "innerGlow");
    fx.done()
}

fn colour_overlay(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Colour overlay", "style_colour_overlay")?;
    fx.colour(conv, "solidFill/color", "colour");
    // An overlay's Opacity **is** its Mix row (§1): the seam takes the blended
    // result and then this much of it, which is what Photoshop means.
    fx.float(conv, "solidFill/opacity", "mix", 1.0, 0.0);
    interior_blend(&mut fx, "solidFill/mode2");
    fx.done()
}

fn gradient_overlay(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Gradient overlay", "style_gradient_overlay")?;
    fx.float(conv, "gradientFill/opacity", "mix", 1.0, 0.0);
    // A direction rather than a light, so the opposition is left out.
    fx.float(conv, "gradientFill/angle", "angle", -1.0, 90.0);
    fx.float(conv, "gradientFill/scale", "scale", 1.0, 0.0);
    fx.toggle("gradientFill/reverse", "reverse");
    fx.choice(conv, "gradientFill/type", "gradient_type", |v| match v {
        1 => (0, None),
        2 => (1, None),
        // Angle, Reflected and Diamond are ramp geometries Lumit's two-stop
        // overlay has no shape for; Linear is the nearest, and it says so.
        _ => (
            0,
            Some("a linear ramp — Lumit's overlay has Linear and Radial"),
        ),
    });
    interior_blend(&mut fx, "gradientFill/mode2");
    // The ramp itself is one of the four things the DOM refuses outright (the
    // capture counts it as unreadable), so both stops keep their defaults.
    fx.drop_params(
        conv,
        &[
            "Colors",
            "Gradient Smoothness",
            "Align with Layer",
            "Offset",
        ],
    );
    fx.done()
}

fn stroke(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Stroke", "style_stroke")?;
    fx.colour(conv, "frameFX/color", "stroke_colour");
    fx.float(conv, "frameFX/opacity", "opacity", 1.0, 0.0);
    fx.float(conv, "frameFX/size", "size", 1.0, 0.0);
    // Outside, Inside, Centre — the same three, in the same order.
    fx.choice(conv, "frameFX/style", "position", |v| {
        (u32::try_from(v - 1).unwrap_or(0).min(2), None)
    });
    interior_blend(&mut fx, "frameFX/mode2");
    fx.done()
}

/// Satin — **modelled and imported, not drawn in this version** (§8).
fn satin(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    _global_angle: f64,
) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Satin", "style_satin")?;
    fx.colour(conv, "chromeFX/color", "satin_colour");
    fx.float(conv, "chromeFX/opacity", "opacity", 1.0, 0.0);
    // Satin's angle throws the offset copies rather than placing a light, so
    // it takes the overlay form.
    fx.float(conv, "chromeFX/localLightingAngle", "direction", -1.0, 90.0);
    fx.float(conv, "chromeFX/distance", "distance", 1.0, 0.0);
    fx.float(conv, "chromeFX/blur", "softness", 1.0, 0.0);
    fx.toggle("chromeFX/invert", "invert");
    interior_blend(&mut fx, "chromeFX/mode2");
    unrendered(&mut fx, conv);
    fx.done()
}

/// Bevel and emboss — **modelled and imported, not drawn in this version**
/// (§8), on exactly Satin's terms.
fn bevel(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    global_angle: f64,
) -> Option<EffectInstance> {
    let mut fx = Fx::of(path, node, "Bevel and emboss", "style_bevel_emboss")?;
    fx.choice(conv, "bevelEmboss/bevelStyle", "bevel_style", |v| {
        (u32::try_from(v - 1).unwrap_or(1).min(4), None)
    });
    fx.choice(conv, "bevelEmboss/bevelTechnique", "technique", |v| {
        (u32::try_from(v - 1).unwrap_or(0).min(2), None)
    });
    fx.float(conv, "bevelEmboss/strengthRatio", "depth", 1.0, 0.0);
    fx.choice(conv, "bevelEmboss/bevelDirection", "direction", |v| {
        (u32::try_from(v - 1).unwrap_or(0).min(1), None)
    });
    fx.float(conv, "bevelEmboss/blur", "size", 1.0, 0.0);
    fx.float(conv, "bevelEmboss/softness", "softness", 1.0, 0.0);
    light(&mut fx, conv, "bevelEmboss", "angle", global_angle);
    fx.float(
        conv,
        "bevelEmboss/localLightingAltitude",
        "altitude",
        1.0,
        0.0,
    );
    fx.colour(conv, "bevelEmboss/highlightColor", "highlight_colour");
    fx.float(
        conv,
        "bevelEmboss/highlightOpacity",
        "highlight_opacity",
        1.0,
        0.0,
    );
    fx.colour(conv, "bevelEmboss/shadowColor", "shadow_colour");
    fx.float(
        conv,
        "bevelEmboss/shadowOpacity",
        "shadow_opacity",
        1.0,
        0.0,
    );
    // Two blend modes on one style, and Lumit's Blend row is one: neither is
    // carried, which costs nothing while the style draws nothing.
    fx.drop_params(conv, &["Highlight Mode", "Shadow Mode"]);
    unrendered(&mut fx, conv);
    fx.done()
}

// ---------------------------------------------------------------------------
// The pieces several of them share
// ---------------------------------------------------------------------------

/// A style's **light angle**: `270° − a`, and a report row when After Effects
/// was following the composition's global light.
///
/// The DOM hands the resolved `localLightingAngle` either way, so following the
/// light is not a value that has to be fetched — it is a *link* that Lumit has
/// nowhere to keep (§1: there is no comp-level light in v1), and the row is
/// what stops the link disappearing silently.
fn light(fx: &mut Fx<'_>, conv: &mut Conv<'_>, slot: &str, lumit_id: &str, global_angle: f64) {
    let ae_id = format!("{slot}/localLightingAngle");
    fx.float(conv, &ae_id, lumit_id, -1.0, 270.0);
    if fx
        .still(&format!("{slot}/useGlobalAngle"))
        .is_some_and(|v| v.abs() > f64::EPSILON)
    {
        fx.approx_named(
            conv,
            "Angle",
            &format!(
                "the composition's global light angle of {global_angle:.0}°, baked in — a Lumit \
                 style carries its own direction and there is no comp-wide light to follow"
            ),
        );
    }
}

/// A glow's colour, and the ramp alternative the DOM refuses.
///
/// `AEColorChoice` picks between the single colour and a ramp; the ramp is one
/// of the four things After Effects itself will not hand over (the capture
/// counts it), so a glow set to one imports at its colour and says so.
fn glow_colour(fx: &mut Fx<'_>, conv: &mut Conv<'_>, slot: &str, lumit_id: &str) {
    fx.colour(conv, &format!("{slot}/color"), lumit_id);
    if fx
        .still(&format!("{slot}/AEColorChoice"))
        .is_some_and(|v| (v - 2.0).abs() < f64::EPSILON)
    {
        fx.approx_named(
            conv,
            "Colors",
            "the glow's single colour — After Effects will not hand over a gradient ramp, and \
             Lumit's glow takes one colour",
        );
    }
}

/// The glow dials Lumit has no counterpart for: the edge-finding technique, the
/// range the falloff is measured over, the jitter, and the ramp's smoothness.
fn glow_extras(fx: &mut Fx<'_>, conv: &mut Conv<'_>, slot: &str) {
    let mut named = Vec::new();
    if fx
        .still(&format!("{slot}/glowTechnique"))
        .is_some_and(|v| (v - 1.0).abs() > f64::EPSILON)
    {
        named.push("Technique");
    }
    if fx
        .still(&format!("{slot}/inputRange"))
        .is_some_and(|v| (v - 50.0).abs() > f64::EPSILON)
    {
        named.push("Range");
    }
    if fx
        .still(&format!("{slot}/shadingNoise"))
        .is_some_and(|v| v.abs() > f64::EPSILON)
    {
        named.push("Jitter");
    }
    fx.drop_params(conv, &named);
}

/// Photoshop's per-style dither. Not modelled, and only worth a row when
/// somebody actually moved it off zero.
fn noise(fx: &mut Fx<'_>, conv: &mut Conv<'_>, ae_id: &str) {
    if fx.still(ae_id).is_some_and(|v| v.abs() > f64::EPSILON) {
        fx.drop_param(conv, "Noise");
    }
}

/// An **interior** style's blend mode onto the injected Blend row (K-425).
fn interior_blend(fx: &mut Fx<'_>, ae_id: &str) {
    if let Some(mode) = fx.still(ae_id).and_then(|v| blend_mode(v.round() as i64)) {
        let index = BlendMode::ALL.iter().position(|b| *b == mode).unwrap_or(0);
        fx.set(
            lumit_core::fx::BLEND_PARAM,
            EffectValue::Choice(u32::try_from(index).unwrap_or(0)),
        );
    }
}

/// An **outer** style's blend mode, which this seam cannot honour.
///
/// Drop shadow and Outer glow composite *underneath* the picture rather than
/// over it, so the Blend row — which combines a kernel's output with its input
/// — has nothing to say about them. Normal and Multiply are what a shadow and a
/// glow already do there; anything else is named rather than quietly applied to
/// the wrong side of the composite.
fn outer_blend(fx: &mut Fx<'_>, conv: &mut Conv<'_>, ae_id: &str) {
    let Some(mode) = fx.still(ae_id).and_then(|v| blend_mode(v.round() as i64)) else {
        return;
    };
    if matches!(mode, BlendMode::Normal | BlendMode::Multiply) {
        return;
    }
    fx.approx_named(
        conv,
        "Blend Mode",
        "the ordinary composite — an outer style is drawn underneath the layer, where a blend \
         mode has nothing to combine with",
    );
}

/// A style Lumit keeps whole and does not draw (§8).
fn unrendered(fx: &mut Fx<'_>, conv: &mut Conv<'_>) {
    fx.differs(
        conv,
        "is kept with every value and every keyframe, and is not drawn in this version",
    );
}

/// After Effects' layer-style blend list, which is Photoshop's — **including
/// its four separators**, which occupy an index each.
///
/// Two of the indices are pinned by the golden capture's own defaults: a drop
/// shadow arrives at 5, which Photoshop calls Multiply, and a glow at 11, which
/// is Screen. The rest is Photoshop's published order counted through from
/// there.
// ponytail: the list is read off Photoshop's documented order with two indices
// confirmed against a real capture, rather than off Adobe's enumeration.
// Ceiling: an index this table does not know imports as Normal and says so,
// which is a wrong mode on one style rather than a wrong picture everywhere.
// Upgrade: read the enumeration off a live After Effects. Trigger: a report row
// naming an unmapped index, or a side-by-side comparison where a style's mode
// is wrong.
fn blend_mode(index: i64) -> Option<BlendMode> {
    Some(match index {
        1 => BlendMode::Normal,
        4 => BlendMode::Darken,
        5 => BlendMode::Multiply,
        6 => BlendMode::ColourBurn,
        7 => BlendMode::LinearBurn,
        8 => BlendMode::DarkerColour,
        10 => BlendMode::Lighten,
        11 => BlendMode::Screen,
        12 => BlendMode::ColourDodge,
        13 => BlendMode::Add,
        14 => BlendMode::LighterColour,
        16 => BlendMode::Overlay,
        17 => BlendMode::SoftLight,
        18 => BlendMode::HardLight,
        19 => BlendMode::VividLight,
        20 => BlendMode::LinearLight,
        21 => BlendMode::PinLight,
        22 => BlendMode::HardMix,
        24 => BlendMode::Difference,
        25 => BlendMode::Exclusion,
        26 => BlendMode::Subtract,
        27 => BlendMode::Divide,
        29 => BlendMode::Hue,
        30 => BlendMode::Saturation,
        31 => BlendMode::Colour,
        32 => BlendMode::Luminosity,
        // Dissolve, and the separators, and anything a newer After Effects
        // adds. Normal is the honest stand-in.
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
