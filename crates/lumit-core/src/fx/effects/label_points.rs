//! Label points writes a short piece of text beside every point of a stream.
//!
//! The look comes from a Text layer: its font, size, colour and style. The
//! wording is a template, typed into the Text row, or the Text layer's own
//! text while that row is empty. In it `{index}` is the point's place in the
//! stream, `{number}` the number it carries, `{x}` and `{y}` where it is, and
//! `{id}`, `{size}` and `{age}` read the same way. Any other text is written
//! as it is, at every point. Hide the Text layer if it is only there to be
//! read.
//!
//! Labels follow where the points are seen and do not turn with them. This
//! file only says which label goes where. The letters are drawn by the text
//! engine, which sits above this crate, so the renderer does that.
//!
//! Nothing wired, or no Text layer named, draws nothing.

use crate::fx::effects::vary_points::{APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN};
use crate::fx::points::PointsStream;
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledWhen, ParamId, Params, Port, PortType,
    ResolveCx, ShortText, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input.
pub const POINTS_PORT: &str = "points";

const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// The most labels a frame may ever be asked for. Each different wording is
/// laid out and drawn by the text engine, so this is what bounds the cost.
pub const LABELS_HARD: i64 = 2_000;

pub const LABEL_ENABLED_WHEN: &[EnabledWhen] = &[APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN];

/// Label points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "label_points",
    label = "Label points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be anywhere, and its label reaches away from it.
    roi = FullFrame,
    premultiplied = true,
    seeded = false,
    enabled_when = LABEL_ENABLED_WHEN,
)]
pub struct LabelPoints {
    /// Which points get a label.
    #[choice(
        label = "Apply to",
        options = ["All points", "Picked", "Not picked"],
        default = 0
    )]
    pub apply_to: u32,

    /// The group Picked and Not picked go by: a name a Pick points or a Vary
    /// points above wrote, or one of the `@` names. Empty is the points a
    /// Pick points picked.
    #[text(label = "Group", default = "")]
    pub apply_group: ShortText,

    /// What a point's Group has to read above to be in it. Only read with a
    /// Group named.
    #[slider(label = "Threshold", min = 0.0, max = 1.0, default = 0.5, unit = Raw)]
    pub apply_threshold: f32,

    /// The Text layer whose text is the template and whose look the labels
    /// take. Unset draws nothing, unless this effect is on a Text layer.
    #[layer(label = "Text layer")]
    pub text_layer: bool,

    /// The template. Empty means the Text layer's own text is the template.
    #[text(label = "Text", default = "")]
    pub text: ShortText,

    /// How many decimal places the numbers in a label are written to.
    #[counter(
        label = "Decimals",
        min = 0,
        max = 4,
        default = 0,
        hard_min = 0,
        hard_max = 4,
        unit = Raw
    )]
    pub decimals: i32,

    /// How far across from its point a label sits, px@comp.
    #[slider(label = "Offset x", min = -200.0, max = 200.0, default = 8.0, unit = Px)]
    pub offset_x: f32,

    /// How far down from its point a label sits, px@comp.
    #[slider(label = "Offset y", min = -200.0, max = 200.0, default = 0.0, unit = Px)]
    pub offset_y: f32,

    /// Which part of the label sits at that offset. Left runs the text away
    /// to the right of it, and Right to the left.
    #[choice(label = "Anchor", options = ["Left", "Centre", "Right"], default = 0)]
    pub anchor: u32,

    /// The most labels drawn. Past it the newest points keep theirs.
    #[counter(
        label = "Max labels",
        min = 1,
        max = LABELS_HARD,
        default = 300,
        hard_min = 1,
        hard_max = LABELS_HARD,
        unit = Raw
    )]
    pub max_labels: i32,

    /// The Mix every effect ends with, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// `v` written to `decimals` places. A value that rounds to nothing is
/// written without a minus sign.
fn number(v: f32, decimals: usize) -> String {
    let unit = 10f32.powi(decimals as i32);
    format!("{:.decimals$}", (v * unit).round() / unit + 0.0)
}

/// `text` with each `{attr:NAME}` and `{@NAME}` in it written as what that
/// name reads at point `i`. A name that reads nothing is left as it was
/// typed, so a slip shows in the picture.
fn named(text: &str, stream: &PointsStream, i: usize, decimals: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((head, after)) = rest.split_once('}') {
        rest = after;
        // The token starts at the last brace opened before this one closes.
        let Some((before, inner)) = head.rsplit_once('{') else {
            out.push_str(head);
            out.push('}');
            continue;
        };
        out.push_str(before);
        let name = inner
            .strip_prefix("attr:")
            .or(inner.starts_with('@').then_some(inner));
        match name.and_then(|name| Some((name, stream.value(name, i)?))) {
            Some((name, v)) => {
                // A place, an id and a count are whole numbers.
                let whole = ["@index", "@id", "@n"]
                    .iter()
                    .any(|w| name.eq_ignore_ascii_case(w));
                out.push_str(&number(v, if whole { 0 } else { decimals }));
            }
            None => {
                out.push('{');
                out.push_str(inner);
                out.push('}');
            }
        }
    }
    out.push_str(rest);
    out
}

impl LabelPoints {
    /// The raster factor, since a stream read off a wire is in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// How far along a label its anchor is: 0 its left end, 1 its right.
    #[must_use]
    pub fn anchor_along(self) -> f32 {
        match self.anchor {
            1 => 0.5,
            2 => 1.0,
            _ => 0.0,
        }
    }

    /// The template: the Text row, or `layer_text` while that row is empty.
    #[must_use]
    pub fn template<'a>(&'a self, layer_text: &'a str) -> &'a str {
        if self.text.is_empty() {
            layer_text
        } else {
            self.text.as_str()
        }
    }

    /// Each label's wording and where its anchor goes, in the stream's own
    /// order. `stream` is in px@comp, which is what the numbers in a label
    /// read, and the places come back in a raster `px_scale` times that.
    #[must_use]
    pub fn labels(
        self,
        stream: &PointsStream,
        template: &str,
        px_scale: f32,
    ) -> Vec<(String, [f32; 2])> {
        // A point behind the camera is not seen, so it has no label.
        let chosen: Vec<usize> = (0..stream.len())
            .filter(|i| {
                let group = self.apply_group.as_str();
                stream.applies(self.apply_to, group, self.apply_threshold, *i)
                    && stream.depth_scale(*i) > 0.0
            })
            .collect();
        let most = self.max_labels.clamp(0, LABELS_HARD as i32) as usize;
        let decimals = self.decimals.clamp(0, 4) as usize;
        let get = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
        chosen
            .iter()
            .skip(chosen.len().saturating_sub(most))
            .map(|&i| {
                let seen = stream.projected(i);
                let text = if template.contains('{') {
                    template
                        .replace("{index}", &i.to_string())
                        .replace("{id}", &stream.id.get(i).copied().unwrap_or(0).to_string())
                        .replace("{number}", &number(stream.index_of(i), decimals))
                        .replace("{x}", &number(seen[0], decimals))
                        .replace("{y}", &number(seen[1], decimals))
                        .replace("{size}", &number(get(&stream.size, i), decimals))
                        .replace("{age}", &number(get(&stream.age, i), decimals))
                } else {
                    template.to_owned()
                };
                let text = named(&text, stream, i, decimals);
                let at = [
                    seen[0] * px_scale + self.offset_x,
                    seen[1] * px_scale + self.offset_y,
                ];
                (text, at)
            })
            .collect()
    }
}

/// Label points' behaviour. No CPU reference through the trait, as with the
/// rest of the family: the stream and the Text layer aren't in the bag.
pub struct LabelPointsDef;

impl EffectDef for LabelPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LabelPoints as EffectMetadata>::SCHEMA
    }

    /// A picture in, a picture out, and a stream in beside it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(LabelPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}
