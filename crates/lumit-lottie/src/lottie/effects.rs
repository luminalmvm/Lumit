//! The five effects a Lottie player can draw: drop shadow, Gaussian blur,
//! fill, tint and tritone, with their numbers in the order and units the
//! players read (lottie-web's `SVGDropShadowEffect`, `SVGGaussianBlurEffect`,
//! `SVGFillFilter`, `SVGTintEffect` and `SVGTritoneFilter`; lottie-android
//! finds a drop shadow's and a blur's numbers by their names), with Stroke,
//! Levels, Transform and Set matte for lottie-web. Every other effect is left
//! out and named in the check.
//!
//! Runs on the export's thread.

use lumit_core::anim::{Animation, Property};
use lumit_core::model::{art_shift, EffectInstance, EffectNamespace, EffectValue, Layer};
use serde_json::{json, Value};

use uuid::Uuid;

use super::value::{Class, Clock};
use crate::check::{arg, Feature, Place, Report};
use crate::fit::tolerance;
use crate::walk::{set_matte, set_matte_english, SetMatte};

/// What the effect is called where it is listed: its own name, else its
/// label.
#[must_use]
pub fn label(e: &EffectInstance) -> String {
    e.custom_name.clone().unwrap_or_else(|| {
        lumit_core::fx::builtins::schema(&e.effect.match_name)
            .map_or_else(|| e.effect.match_name.clone(), |s| s.label.to_owned())
    })
}

/// The layer's effects a player draws, as Lottie's `ef`.
/// `shift` is where the layer's own pixels sit in the coordinates its
/// content is written in, which a Transform effect's points move by.
/// `source` gives the number of a drawn layer of the comp, which a Set matte
/// names as its matte, and `matted` says the layer is drawn through a track
/// matte, which a player's Set matte would take the place of.
pub fn effects(
    c: &Clock,
    l: &Layer,
    shift: (f64, f64),
    source: &dyn Fn(Uuid) -> Option<u32>,
    matted: bool,
    report: &mut Report,
    place: &Place,
) -> Vec<Value> {
    if !l.switches.fx {
        return Vec::new();
    }
    let mut out = Vec::new();
    for e in l.effects.iter().filter(|e| e.enabled) {
        let name = label(e);
        let here = place.part(&name);
        let builtin = matches!(e.effect.namespace, EffectNamespace::Builtin);
        let (mapped, reason) = if !builtin {
            (None, "effect")
        } else if e.effect.match_name == "set_matte" {
            match set_matte(c.doc, c.comp, l, e) {
                SetMatte::Unset => continue,
                SetMatte::Alpha {
                    source: s,
                    inverted: false,
                    from,
                    to,
                } if !matted && placed_alike(l, s, from, to) => match source(s.id) {
                    Some(ind) => (Some(matte(ind)), "effect"),
                    // lottie-web shows the matte layer itself whatever its
                    // eye says, so only a drawn one is named.
                    None => (None, "set_matte_hidden"),
                },
                SetMatte::Alpha { .. } => (None, "effect"),
                SetMatte::LeftOut(reason) => (None, reason),
            }
        } else {
            (mapped(c, l, e, shift, &name, report, &here), "effect")
        };
        match mapped {
            Some((ty, feature, values)) => {
                report.players(
                    &here,
                    feature,
                    "effect",
                    name.clone(),
                    vec![arg("label", name.clone())],
                );
                let ef: Vec<Value> = values
                    .into_iter()
                    .enumerate()
                    .map(|(i, (vt, nm, v))| json!({"ty": vt, "nm": nm, "ix": i + 1, "v": v}))
                    .collect();
                let mut m = serde_json::Map::new();
                m.insert("ty".into(), Value::from(ty));
                c.fmt.name(&mut m, &name);
                m.insert("np".into(), Value::from(ef.len() + 1));
                m.insert("ix".into(), Value::from(out.len() + 1));
                m.insert("en".into(), Value::from(1));
                m.insert("ef".into(), Value::Array(ef));
                out.push(Value::Object(m));
            }
            None => report.left_out(
                &here,
                reason,
                set_matte_english(reason, &name),
                vec![arg("label", name.clone())],
            ),
        }
    }
    out
}

/// True when lottie-web's Set matte, which reads the source as placed in the
/// comp with its opacity and masks after every other effect, reads what
/// Lumit reads: the source's own pixels stretched onto the layer. So the two
/// boxes and placements are one, the source is fully opaque, and the matte is
/// the layer's only effect.
fn placed_alike(l: &Layer, s: &Layer, from: [f64; 4], to: [f64; 4]) -> bool {
    let full = matches!(s.transform.opacity.animation, Animation::Static(v) if v >= 100.0);
    from == to
        && full
        && l.transform == s.transform
        && l.parent == s.parent
        && l.start_offset == s.start_offset
        && l.switches.three_d == s.switches.three_d
        && l.effects.iter().filter(|e| e.enabled).count() == 1
}

/// Set matte as lottie-web's (`SVGMatte3Effect`), which reads only the
/// layer it names, by number, and masks with its alpha.
fn matte(ind: u32) -> (u8, Feature, Vec<Param>) {
    let still = |v: f64| json!({"a": 0, "k": v});
    (
        28,
        Feature::EffectSetMatte,
        vec![
            (10, "Take Matte From Layer", json!({"a": 0, "k": ind})),
            (7, "Use For Matte", still(4.0)),
            (4, "Invert Matte", still(0.0)),
            (7, "If Layer Sizes Differ", still(1.0)),
            (4, "Composite Matte with Original", still(1.0)),
            (4, "Premultiply Matte Layer", still(1.0)),
        ],
    )
}

/// One effect value: its kind (0 slider, 2 colour, 4 checkbox, 7 menu, 10
/// layer), the name players find it by, and the property.
type Param = (u8, &'static str, Value);

/// An effect a player knows, as its Lottie code, the player feature it
/// needs and its values; `None` for every other effect.
#[allow(clippy::too_many_arguments)]
fn mapped(
    c: &Clock,
    l: &Layer,
    e: &EffectInstance,
    shift: (f64, f64),
    name: &str,
    report: &mut Report,
    here: &Place,
) -> Option<(u8, Feature, Vec<Param>)> {
    let float = |id: &str| match param(e, id) {
        Some(EffectValue::Float(p)) => Some(p),
        _ => None,
    };
    let colour = |id: &str| match param(e, id) {
        Some(EffectValue::Colour(ch)) => Some(c.colour_padded([&ch[0], &ch[1], &ch[2]], &[1.0])),
        _ => None,
    };
    let still = |v: f64| json!({"a": 0, "k": v});
    // The Mix every effect ends with, as a fraction when it is still.
    let mix = float("mix").map_or(Some(1.0), |p| match p.animation {
        Animation::Static(v) => Some(v / 100.0),
        _ => None,
    });
    let slider =
        |p: &Property, k: f64| c.prop_mapped(p, Class::Px, tolerance::PIXELS, move |v| v * k);
    let partly = |report: &mut Report| {
        if mix.is_none_or(|m| m < 1.0) {
            report.changed(
                here,
                "effect_mix",
                format!("{name}'s Mix is left out"),
                vec![arg("label", name)],
            );
        }
    };
    match e.effect.match_name.as_str() {
        "drop_shadow" => {
            if matches!(param(e, "shadow_only"), Some(EffectValue::Bool(true))) {
                report.left_out(
                    here,
                    "effect_option",
                    format!("{name}'s Shadow only is left out"),
                    vec![arg("label", name)],
                );
            }
            // A shadow under its layer mixed in part is the same shadow,
            // lighter: the Mix folds into the opacity, 0 to 255 as players
            // read it. A Mix that moves can't fold.
            if mix.is_none() {
                partly(report);
            }
            let fade = mix.unwrap_or(1.0) * 2.55;
            Some((
                25,
                Feature::EffectDropShadow,
                vec![
                    (2, "Shadow Color", colour("shadow_colour")?),
                    (0, "Opacity", slider(float("opacity")?, fade)),
                    (
                        1,
                        "Direction",
                        c.prop(&[float("direction")?], Class::Deg, tolerance::DEGREES),
                    ),
                    (0, "Distance", slider(float("distance")?, 1.0)),
                    // Lumit's softness is twice the gaussian's sigma; a
                    // player's is four times it.
                    (0, "Softness", slider(float("softness")?, 2.0)),
                ],
            ))
        }
        "blur" => {
            partly(report);
            let repeat = matches!(
                param(e, "repeat_edge_pixels"),
                Some(EffectValue::Bool(true))
            );
            Some((
                29,
                Feature::EffectBlur,
                vec![
                    // Lumit's sigma is half the radius; a player's is 0.3 of
                    // the blurriness.
                    (0, "Blurriness", slider(float("radius")?, 0.5 / 0.3)),
                    (7, "Blur Dimensions", still(1.0)),
                    (
                        4,
                        "Repeat Edge Pixels",
                        still(if repeat { 1.0 } else { 0.0 }),
                    ),
                ],
            ))
        }
        "fill" => Some((
            21,
            Feature::EffectFill,
            vec![
                (10, "Fill Mask", still(0.0)),
                (7, "All Masks", still(0.0)),
                (2, "Color", colour("colour")?),
                (7, "Invert", still(0.0)),
                (0, "Horizontal Feather", still(0.0)),
                (0, "Vertical Feather", still(0.0)),
                (0, "Opacity", slider(float("mix")?, 0.01)),
            ],
        )),
        "tint" => Some((
            20,
            Feature::EffectTint,
            vec![
                (2, "Map Black To", colour("black")?),
                (2, "Map White To", colour("white")?),
                (0, "Amount to Tint", slider(float("mix")?, 1.0)),
            ],
        )),
        "tritone" => {
            partly(report);
            Some((
                23,
                Feature::EffectTritone,
                vec![
                    (2, "Highlights", colour("highlights")?),
                    (2, "Midtones", colour("midtones")?),
                    (2, "Shadows", colour("shadows")?),
                    (
                        0,
                        "Blend With Original",
                        c.prop_mapped(float("mix")?, Class::Px, tolerance::PERCENT, |v| 100.0 - v),
                    ),
                ],
            ))
        }
        "stroke" => {
            // lottie-web strokes one of the layer's masks, counted from 1.
            let mask = match param(e, "path") {
                Some(EffectValue::MaskPath(None)) => (!l.masks.is_empty()).then_some(1),
                Some(EffectValue::MaskPath(Some(id))) => {
                    l.masks.iter().position(|m| m.id == *id).map(|i| i + 1)
                }
                _ => None,
            }?;
            let style = match param(e, "paint_style") {
                Some(EffectValue::Choice(n)) => *n,
                _ => 0,
            };
            // On the original the Mix only lightens the stroke; on its own
            // or revealing, it would bring the layer back, which a player
            // can't.
            if style != 0 || mix.is_none() {
                partly(report);
            }
            let hard = float("hardness")
                .is_some_and(|p| matches!(p.animation, Animation::Static(v) if v >= 100.0));
            if !hard {
                report.changed(
                    here,
                    "effect_hard_edge",
                    format!("{name} is drawn with a hard edge"),
                    vec![arg("label", name)],
                );
            }
            let fade = (if style == 0 { mix.unwrap_or(1.0) } else { 1.0 }) / 100.0;
            Some((
                22,
                Feature::EffectStroke,
                vec![
                    (11, "Path", json!({"a": 0, "k": mask})),
                    (4, "All Masks", still(0.0)),
                    (4, "Stroke Sequentially", still(0.0)),
                    (2, "Color", colour("colour")?),
                    // A player's brush size is the radius; Lumit's the width.
                    (0, "Brush Size", slider(float("brush_size")?, 0.5)),
                    (0, "Brush Hardness", slider(float("hardness")?, 1.0)),
                    (0, "Opacity", slider(float("opacity")?, fade)),
                    (0, "Start", slider(float("start")?, 1.0)),
                    (0, "End", slider(float("end")?, 1.0)),
                    (0, "Spacing", slider(float("spacing")?, 1.0)),
                    (7, "Paint Style", json!({"a": 0, "k": style + 1})),
                ],
            ))
        }
        "levels" => levels(c, e).map(|values| {
            partly(report);
            (24, Feature::EffectLevels, values)
        }),
        "transform" => {
            // A Mix below 100 shows the moved and the unmoved layer at once,
            // which a transform can't.
            if mix != Some(1.0) {
                return None;
            }
            // The points are in the layer's pixels, whose corner moves with
            // a Shape layer's art.
            let art_moves = c
                .times()
                .into_iter()
                .take_while(|_| !c.host.cancelled())
                .any(|lt| art_shift(l, lt) != (0.0, 0.0));
            let point = |x: &str, y: &str| -> Option<Value> {
                let (px, py) = (float(x)?, float(y)?);
                let track = if art_moves {
                    c.sampled(
                        |lt| {
                            let (dx, dy) = art_shift(l, lt);
                            vec![
                                c.value(px, lt) + shift.0 + dx,
                                c.value(py, lt) + shift.1 + dy,
                            ]
                        },
                        &[tolerance::PIXELS; 2],
                    )
                } else {
                    c.track(&[px, py], &[tolerance::PIXELS; 2])
                        .map_affine(|d, v| v + if d == 0 { shift.0 } else { shift.1 })
                };
                Some(c.write(&track, Class::Px, true, &[]))
            };
            let angle = |id: &str| Some(c.prop(&[float(id)?], Class::Deg, tolerance::DEGREES));
            Some((
                35,
                Feature::EffectTransform,
                vec![
                    (3, "Anchor Point", point("anchor_x", "anchor_y")?),
                    (3, "Position", point("position_x", "position_y")?),
                    (4, "Uniform Scale", still(0.0)),
                    (0, "Scale Height", slider(float("scale_y")?, 1.0)),
                    (0, "Scale Width", slider(float("scale_x")?, 1.0)),
                    (0, "Skew", slider(float("skew")?, 1.0)),
                    (1, "Skew Axis", angle("skew_axis")?),
                    (1, "Rotation", angle("rotation")?),
                    (0, "Opacity", slider(float("opacity")?, 1.0)),
                ],
            ))
        }
        _ => None,
    }
}

/// Levels as lottie-web's Pro Levels, whose rows sit at 3, 10, 17, 24 and
/// 31 (master, red, green, blue, alpha), five numbers each.
///
/// Lumit grades linear light and a player grades the display encoding. With
/// both black points at 0 a curve of whites and gamma reads the same once its
/// whites are encoded (the encoding is close to a power, which commutes with
/// the curve), so that is written. A black point has no such reading and the
/// effect is left out.
fn levels(c: &Clock, e: &EffectInstance) -> Option<Vec<Param>> {
    let rows = ["master", "red", "green", "blue"];
    let float = |id: String| match param(e, &id) {
        Some(EffectValue::Float(p)) => Some(p),
        _ => None,
    };
    let zero = |p: &Property| matches!(p.animation, Animation::Static(v) if v == 0.0);
    let mut out: Vec<Param> = (0..36).map(|_| (0, "", json!({"a": 0, "k": 0}))).collect();
    for (n, row) in rows.iter().enumerate() {
        let get = |what: &str| float(format!("{row}_{what}"));
        if !(zero(get("in_black")?) && zero(get("out_black")?)) {
            return None;
        }
        let encoded = |p: &Property| {
            let track = c.sampled(|lt| vec![c.host.display([c.value(p, lt); 3])[0]], &[1e-3]);
            c.write(&track, Class::Unit, false, &[])
        };
        let at = 3 + n * 7;
        let values = [
            ("Input Black", json!({"a": 0, "k": 0})),
            ("Input White", encoded(get("in_white")?)),
            ("Gamma", c.prop(&[get("gamma")?], Class::Unit, 1e-3)),
            ("Output Black", json!({"a": 0, "k": 0})),
            ("Output White", encoded(get("out_white")?)),
        ];
        for (i, (nm, v)) in values.into_iter().enumerate() {
            if let Some(slot) = out.get_mut(at + i) {
                *slot = (0, nm, v);
            }
        }
    }
    // The alpha row, untouched.
    for (i, v) in [0.0, 1.0, 1.0, 0.0, 1.0].into_iter().enumerate() {
        if let Some(slot) = out.get_mut(31 + i) {
            *slot = (0, "", json!({"a": 0, "k": v}));
        }
    }
    Some(out)
}

/// The value of the parameter `id`.
fn param<'a>(e: &'a EffectInstance, id: &str) -> Option<&'a EffectValue> {
    e.params.iter().find(|p| p.id == id).map(|p| &p.value)
}
