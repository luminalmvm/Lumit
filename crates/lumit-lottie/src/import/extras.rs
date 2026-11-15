//! A Lottie layer's effects and its text.
//!
//! # In plain terms
//!
//! A Lottie carries a layer's effects as After Effects listed them, values in
//! After Effects' own order. Five of them have Lumit equals with the same
//! controls: Tint, Fill, Tritone, Drop shadow and Gaussian blur. Any other is
//! left out and named in the report.
//!
//! A text layer keeps its words and how they are set, as keyed documents. The
//! first document is the one that arrives; words that change over time, and
//! the per-letter animators, are named in the report.
//!
//! Runs on the import's thread.

use lumit_core::anim::Property;
use lumit_core::model::{EffectInstance, EffectValue, LinearColour, TextDocument};
use lumit_import::report::{ItemPath, Outcome, Reason};
use serde_json::Value;

use super::layers::linear;
use super::read::{self, num, nums, text};
use super::{keys, Cx};
use crate::colour::srgb_decode;
use crate::LottieError;

/// The layer's effects that have Lumit equals, top of the stack first.
pub(super) fn effects(
    cx: &mut Cx<'_>,
    path: &ItemPath,
    v: &Value,
) -> Result<Vec<EffectInstance>, LottieError> {
    let mut out = Vec::new();
    for (n, e) in read::list(v, "ef").iter().enumerate() {
        let name = text(e, "nm")
            .filter(|n| !n.trim().is_empty())
            .map_or_else(|| format!("Effect {}", n + 1), str::to_owned);
        let here = path.property(&name);
        let values = read::list(e, "ef");
        let value = |i: usize| values.get(i).and_then(|p| p.get("v"));
        let lumit = match num(e.get("ty")).map(|t| t as i64) {
            Some(20) => "tint",
            Some(21) => "fill",
            Some(23) => "tritone",
            Some(25) => "drop_shadow",
            Some(29) => "blur",
            _ => {
                cx.left_out(here, &name);
                continue;
            }
        };
        let Some(mut fx) = lumit_core::fx::builtins::instantiate(lumit) else {
            cx.left_out(here, &name);
            continue;
        };
        fx.enabled = num(e.get("en")).is_none_or(|en| en != 0.0);
        fx.custom_name = Some(name.clone());
        match lumit {
            "tint" => {
                colour(cx, &here, &mut fx, "black", value(0))?;
                colour(cx, &here, &mut fx, "white", value(1))?;
                float(cx, &here, &mut fx, "mix", value(2), 1.0, 0.0)?;
            }
            "fill" => {
                colour(cx, &here, &mut fx, "colour", value(2))?;
                float(cx, &here, &mut fx, "mix", value(6), 100.0, 0.0)?;
            }
            "tritone" => {
                colour(cx, &here, &mut fx, "highlights", value(0))?;
                colour(cx, &here, &mut fx, "midtones", value(1))?;
                colour(cx, &here, &mut fx, "shadows", value(2))?;
                if value(3).is_some() {
                    float(cx, &here, &mut fx, "mix", value(3), -1.0, 100.0)?;
                }
            }
            "drop_shadow" => {
                colour(cx, &here, &mut fx, "shadow_colour", value(0))?;
                float(cx, &here, &mut fx, "opacity", value(1), 100.0 / 255.0, 0.0)?;
                float(cx, &here, &mut fx, "direction", value(2), 1.0, 0.0)?;
                float(cx, &here, &mut fx, "distance", value(3), 1.0, 0.0)?;
                float(cx, &here, &mut fx, "softness", value(4), 1.0, 0.0)?;
            }
            _ => {
                float(cx, &here, &mut fx, "radius", value(0), 1.0, 0.0)?;
                if num(value(1).and_then(|d| d.get("k"))).is_some_and(|d| d != 1.0) {
                    cx.approximated(here.property("Blur dimensions"), "Blur dimensions");
                }
                if let Some(edge) = num(value(2).and_then(|d| d.get("k"))) {
                    set(
                        &mut fx,
                        "repeat_edge_pixels",
                        EffectValue::Bool(edge != 0.0),
                    );
                }
            }
        }
        out.push(fx);
    }
    Ok(out)
}

fn set(fx: &mut EffectInstance, id: &str, value: EffectValue) {
    if let Some(p) = fx.params.iter_mut().find(|p| p.id == id) {
        p.value = value;
    }
}

/// A number control, as `a·v + b`.
fn float(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    fx: &mut EffectInstance,
    id: &str,
    v: Option<&Value>,
    a: f64,
    b: f64,
) -> Result<(), LottieError> {
    let Some(read) = cx.read(here, id, v)? else {
        return Ok(());
    };
    let mut p = keys::property(&read.track, 0, &cx.clock, 0.0);
    super::shapes::affine(&mut p, a, b);
    set(fx, id, EffectValue::Float(p));
    Ok(())
}

/// A colour control, from display sRGB into linear light.
fn colour(
    cx: &mut Cx<'_>,
    here: &ItemPath,
    fx: &mut EffectInstance,
    id: &str,
    v: Option<&Value>,
) -> Result<(), LottieError> {
    let Some(read) = cx.read_mapped(here, id, v, &srgb_decode)? else {
        return Ok(());
    };
    let clock = cx.clock;
    let channel = |d: usize| keys::property(&read.track, d, &clock, 1.0);
    set(
        fx,
        id,
        EffectValue::Colour([channel(0), channel(1), channel(2), Property::fixed(1.0)]),
    );
    Ok(())
}

/// A text layer's first document as Lumit's, and where the Lottie's zero for
/// the layer falls inside the picture Lumit lays the words into.
pub(super) fn text_layer(
    cx: &mut Cx<'_>,
    path: &ItemPath,
    t: &Value,
) -> Result<(TextDocument, (f64, f64)), LottieError> {
    use lumit_core::text::{ParagraphStyle, TextAlign, TextStyle};

    // Documents are keyed, or (before 4.4.14) one document as it is.
    let d = t.get("d").unwrap_or(&Value::Null);
    let docs = read::list(d, "k");
    let doc = docs
        .first()
        .and_then(|k| k.get("s"))
        .or_else(|| d.get("k").filter(|k| k.is_object()))
        .unwrap_or(d);
    if docs.len() > 1
        || !read::list(t, "a").is_empty()
        || t.get("p").is_some_and(|p| p.get("m").is_some())
        || d.get("x").is_some()
    {
        cx.row(
            path.clone(),
            Outcome::Adjusted,
            Reason::TextStylingNotMapped,
        );
    }
    let number = |key: &str| num(doc.get(key));
    let colour = |key: &str| {
        let c = nums(doc.get(key));
        (c.len() >= 3).then(|| {
            linear([
                c.first().copied().unwrap_or(0.0),
                c.get(1).copied().unwrap_or(0.0),
                c.get(2).copied().unwrap_or(0.0),
            ])
        })
    };
    let font = text(doc, "f").unwrap_or_default();
    let (family, face) = cx.fonts.get(font).cloned().unwrap_or_default();
    let words = text(doc, "t")
        .unwrap_or_default()
        .replace(['\r', '\u{3}'], "\n");
    let size = number("s").unwrap_or(36.0);
    let stroke = colour("sc");
    let box_size = nums(doc.get("sz"));
    let document = TextDocument {
        text: words,
        expression: None,
        size,
        fill: colour("fc").unwrap_or(LinearColour::BLACK),
        path: None,
        path_offset: Property::zero(),
        animators: Vec::new(),
        captions: None,
        style: Box::new(TextStyle {
            family,
            face,
            leading: number("lh"),
            tracking: number("tr").unwrap_or(0.0),
            baseline_shift: number("ls").unwrap_or(0.0),
            fill_on: doc.get("fc").is_some(),
            stroke_on: stroke.is_some() && number("sw").unwrap_or(0.0) > 0.0,
            stroke: stroke.unwrap_or(LinearColour::BLACK),
            stroke_width: number("sw").unwrap_or(0.0),
            stroke_over: doc.get("of").and_then(Value::as_bool).unwrap_or(true),
            ..TextStyle::default()
        }),
        paragraph: ParagraphStyle {
            align: match number("j").map(|j| j as i64) {
                Some(1 | 4) => TextAlign::Right,
                Some(2 | 5) => TextAlign::Centre,
                _ => TextAlign::Left,
            },
            justify: number("j").is_some_and(|j| j >= 3.0),
            box_width: box_size.first().copied().unwrap_or(0.0).max(0.0),
            ..ParagraphStyle::default()
        },
        extra: serde_json::Map::new(),
    };

    // Point text hangs from its first baseline at the side it is aligned to;
    // box text from its box's corner.
    let block = lumit_text::layout(&lumit_text::TextBlock::of(&document, &document.text), false);
    let (left, right) = (f64::from(block.left), f64::from(block.right));
    let baseline = block.lines.first().map_or(0.0, |l| f64::from(l.baseline));
    let origin = if document.paragraph.box_width > 0.0 {
        let corner = nums(doc.get("ps"));
        (
            left - corner.first().copied().unwrap_or(0.0),
            baseline - f64::from(block.ascent) - corner.get(1).copied().unwrap_or(0.0),
        )
    } else {
        let x = match document.paragraph.align {
            TextAlign::Left => left,
            TextAlign::Centre => (left + right) / 2.0,
            TextAlign::Right => right,
        };
        (x, baseline)
    };
    Ok((document, origin))
}
