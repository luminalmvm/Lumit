//! Label points writes a short piece of text beside every point of a stream.
//!
//! The look comes from a Text layer: its font, size, colour and style. The
//! wording is a template, typed into the Text row, or the Text layer's own
//! text while that row is empty. In it `{index}` is the point's place in the
//! stream, `{number}` the number it carries, `{x}` and `{y}` where it is, and
//! `{id}`, `{size}` and `{age}` read the same way. SICELIOT adds random text:
//! `{pick:a|b|c}` is one of the words, `{chars:N}` N letters and digits and
//! `{hex:N}` N hexadecimal digits, each point keeping its own (Seed re-rolls
//! them). Any other text is written as it is, at every point. Hide the Text
//! layer if it is only there to be read.
//!
//! Labels follow where the points are seen and do not turn with them. This
//! file only says which label goes where. The letters are drawn by the text
//! engine, which sits above this crate, so the renderer does that.
//!
//! Nothing wired, or no Text layer named, draws nothing.

use crate::fx::effects::vary_points::{APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN};
use crate::fx::points::{Column, PointsStream};
use crate::fx::{
    params::ParamId, params::Params, params::ShortText, params::Value, registry::EffectDef,
    registry::EffectMetadata, registry::ResolveCx, registry::Signature, schema::EffectSchema,
    schema::EnabledWhen, schema::Port, schema::PortType,
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

    /// Which words `{pick:…}` chooses and which characters `{chars:N}` and
    /// `{hex:N}` write (SICELIOT). Each point keeps its own, frame after frame.
    #[seed]
    pub seed: u32,

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

/// The characters `{chars:N}` writes.
const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// The characters `{hex:N}` writes.
const HEX: &[u8] = b"0123456789abcdef";
/// The longest `{chars:N}` or `{hex:N}`.
const RANDOM_MAX: usize = 64;
/// The dice the random text rolls, kept off the numbers the points effects use.
const TEXT_ATTR: u32 = 90;

/// **Random text** (SICELIOT): `text` with each `{pick:a|b|c}` replaced by
/// one of its words, each `{chars:N}` by N letters and digits and each
/// `{hex:N}` by N hexadecimal digits, all chosen by the point's `id` and
/// `seed`, so a point keeps its text frame after frame and whatever order the
/// points come in. Tokens are numbered from the left, so two of them on one
/// label roll differently. Anything else is left as it is.
fn random_text(text: &str, seed: u32, id: u64) -> String {
    if !text.contains("{pick:") && !text.contains("{chars:") && !text.contains("{hex:") {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut token = 0u32;
    // One roll in 0..1 for character `k` of token `token`.
    let roll = |token: u32, k: u32| {
        let attr = TEXT_ATTR
            .wrapping_add(token.wrapping_mul(1_000))
            .wrapping_add(k);
        crate::fx::points::draw(seed, id, attr)
    };
    let pick = |n: usize, u: f32| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let k = (u * n as f32) as usize;
        k.min(n.saturating_sub(1))
    };
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let inner = &after[1..close];
        let made = if let Some(words) = inner.strip_prefix("pick:") {
            let words: Vec<&str> = words.split('|').collect();
            Some(words[pick(words.len(), roll(token, 0))].to_owned())
        } else if let Some((set, n)) = inner
            .strip_prefix("chars:")
            .map(|n| (CHARS, n))
            .or_else(|| inner.strip_prefix("hex:").map(|n| (HEX, n)))
        {
            n.trim().parse::<usize>().ok().map(|n| {
                (0..n.clamp(1, RANDOM_MAX))
                    .map(|k| {
                        #[allow(clippy::cast_possible_truncation)]
                        let u = roll(token, k as u32 + 1);
                        char::from(set[pick(set.len(), u)])
                    })
                    .collect()
            })
        } else {
            None
        };
        match made {
            Some(made) => {
                out.push_str(&made);
                token += 1;
            }
            None => out.push_str(&after[..=close]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// `template` cut at each `{attr:NAME}` and `{@NAME}` in it: the text
/// before each one with what its name reads on `stream`, then the text after
/// the last. A name that reads nothing is left in the text as it was typed,
/// so a slip shows in the picture.
fn named<'a>(template: &'a str, stream: &PointsStream) -> (Vec<(&'a str, Column)>, &'a str) {
    let mut cuts = Vec::new();
    // Where the text not yet handed out starts, and where the search is.
    let (mut from, mut at) = (0, 0);
    while let Some(close) = template.get(at..).and_then(|s| s.find('}')) {
        let close = at + close;
        // The token starts at the last brace opened before this one closes.
        let open = template.get(at..close).and_then(|s| s.rfind('{'));
        let open = open.map(|open| at + open);
        at = close + 1;
        let inner = open.and_then(|open| template.get(open + 1..close));
        let name = inner.and_then(|inner| {
            inner
                .strip_prefix("attr:")
                .or(inner.starts_with('@').then_some(inner))
        });
        let column = name.map_or(Column::Nothing, |name| stream.column(name));
        if let (Some(open), false) = (open, column == Column::Nothing) {
            cuts.push((template.get(from..open).unwrap_or(""), column));
            from = at;
        }
    }
    (cuts, template.get(from..).unwrap_or(""))
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
        let group = stream.group(self.apply_group.as_str());
        let (cuts, tail) = named(template, stream);
        let chosen: Vec<usize> = (0..stream.len())
            .filter(|i| {
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
                let plain = |text: &str| {
                    if !text.contains('{') {
                        return text.to_owned();
                    }
                    let id = stream.id.get(i).copied().unwrap_or(0);
                    let filled = text
                        .replace("{index}", &i.to_string())
                        .replace("{id}", &id.to_string())
                        .replace("{number}", &number(stream.index_of(i), decimals))
                        .replace("{x}", &number(seen[0], decimals))
                        .replace("{y}", &number(seen[1], decimals))
                        .replace("{size}", &number(get(&stream.size, i), decimals))
                        .replace("{age}", &number(get(&stream.age, i), decimals));
                    random_text(&filled, self.seed, id)
                };
                let mut text = String::new();
                for (before, column) in &cuts {
                    // A place, an id and a count are whole numbers.
                    let whole = matches!(column, Column::Index | Column::Id | Column::Count);
                    let v = stream.read(*column, i).unwrap_or(0.0);
                    text += &plain(before);
                    text += &number(v, if whole { 0 } else { decimals });
                }
                text += &plain(tail);
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::random_text;

    /// SICELIOT's random text: each token is replaced, a point keeps its
    /// text, points differ, the Seed re-rolls, unknown braces stay. Guards
    /// labels that change from frame to frame or read the same everywhere.
    #[test]
    fn random_text_fills_tokens_per_point() {
        let t = "{pick:alpha|beta|gamma}-{chars:6}-{hex:4}";
        let a = random_text(t, 1, 7);
        let (word, rest) = a.split_once('-').unwrap();
        let (chars, hex) = rest.split_once('-').unwrap();
        assert!(["alpha", "beta", "gamma"].contains(&word), "{a}");
        assert_eq!(chars.len(), 6);
        assert!(chars.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(hex.len(), 4);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, random_text(t, 1, 7), "the same point keeps its text");
        let others: Vec<String> = (0..20).map(|id| random_text(t, 1, id)).collect();
        assert!(others.windows(2).any(|w| w[0] != w[1]), "points differ");
        assert_ne!(
            random_text("{chars:12}", 1, 7),
            random_text("{chars:12}", 2, 7)
        );
        assert_eq!(random_text("{index} {nope}", 1, 7), "{index} {nope}");
        assert_ne!(
            random_text("{chars:8}{chars:8}", 1, 7)[..8],
            random_text("{chars:8}{chars:8}", 1, 7)[8..],
            "two tokens roll differently"
        );
    }
}
