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
    schema::EnabledCond, schema::EnabledWhen, schema::Port, schema::PortType,
};
use lumit_fx_macros::Effect;

/// The wire-only data input.
pub const POINTS_PORT: &str = "points";

const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// The most labels a frame may ever be asked for. Each different wording is
/// laid out and drawn by the text engine, so this is what bounds the cost.
pub const LABELS_HARD: i64 = 2_000;

pub const LABEL_ENABLED_WHEN: &[EnabledWhen] = &[
    APPLY_GROUP_WHEN,
    APPLY_THRESHOLD_WHEN,
    EnabledWhen {
        param: "colour_a",
        on: "flash_colour",
        cond: EnabledCond::ChoiceIs(2),
    },
    EnabledWhen {
        param: "colour_b",
        on: "flash_colour",
        cond: EnabledCond::ChoiceIs(2),
    },
];

/// What a label's colour does while it is in the wave.
pub const FLASH_COLOUR: &[&str] = &["Off", "Random hue", "Two colours"];

/// The rows a driver can give each label its own value for, and the named
/// columns the render writes those values into (SICELIOT,
/// [`crate::fx::effects::clone_to_points::drive_rows`]).
pub const LABEL_DRIVEN_ROWS: &[(&str, &str)] = &[
    ("change", "siceliot:change"),
    ("label_scale", "siceliot:label_scale"),
];

/// One label: its wording, where its anchor goes, how big it is drawn (1 is
/// the Text layer's size) and the colour it is drawn in, `None` for the Text
/// layer's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    pub at: [f32; 2],
    pub scale: f32,
    pub tint: Option<[f32; 4]>,
}

/// The dice a flashing colour rolls.
const TINT_ATTR: u32 = 95;

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
    // Seeded (SICELIOT): a label in the wave scrambles and flashes with the
    // layer's clock under constant rows, so its frames cannot be shared
    // across time.
    seeded = true,
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

    /// How much a wave is at a point, per cent (SICELIOT). Wire a driver into
    /// it, an Oscillator by Position say, and each point reads its own value.
    #[slider(label = "Change", min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub change: f32,

    /// Above this, a point is in the wave: its random text scrambles and its
    /// colour flashes. Below it, the label rests on its own text and colour.
    #[slider(
        label = "Change above",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub change_above: f32,

    /// New random text a second while a point is in the wave. At 0 it takes
    /// one other text for as long as the wave is there.
    #[slider(
        label = "Scramble rate",
        min = 0.0,
        max = 60.0,
        default = 12.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub scramble_rate: f32,

    /// The colour a label takes while it is in the wave, changing at Scramble
    /// rate: none, any hue, or Colour A and Colour B in turn at random.
    #[choice(
        label = "Flash colour",
        options = FLASH_COLOUR,
        default = 0
    )]
    pub flash_colour: u32,

    /// One of the two colours Two colours flashes between.
    #[colour(label = "Colour A", default = [1.0, 0.2, 0.1, 1.0], max = 4.0)]
    pub colour_a: [f32; 4],

    /// The other.
    #[colour(label = "Colour B", default = [0.1, 0.6, 1.0, 1.0], max = 4.0)]
    pub colour_b: [f32; 4],

    /// How big a label is drawn, per cent of the Text layer's size. Wire a
    /// driver into it and each label takes its own size.
    #[slider(
        label = "Label scale",
        min = 0.0,
        max = 400.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub label_scale: f32,

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

    /// The layer's time, seconds, which the scrambling text and the flashing
    /// colour step by (SICELIOT).
    pub const DERIVED_TIME: ParamId = ParamId::new("derived.time");

    /// This instance's layer time, read back out of a resolved bag.
    #[must_use]
    pub fn time_of(p: Params<'_>) -> f64 {
        f64::from(p.float(Self::DERIVED_TIME, 0.0))
    }

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
        lt: f64,
    ) -> Vec<Label> {
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
        // Change and Label scale, per point where a driver wrote them.
        let column_of = |name: &str| match stream.column(name) {
            Column::Named(_) => Some(stream.column(name)),
            _ => None,
        };
        let change_column = column_of(LABEL_DRIVEN_ROWS[0].1);
        let scale_column = column_of(LABEL_DRIVEN_ROWS[1].1);
        chosen
            .iter()
            .skip(chosen.len().saturating_sub(most))
            .map(|&i| {
                let seen = stream.projected(i);
                let id = stream.id.get(i).copied().unwrap_or(0);
                let change = change_column
                    .and_then(|c| stream.read(c, i))
                    .unwrap_or(self.change);
                let scale = scale_column
                    .and_then(|c| stream.read(c, i))
                    .unwrap_or(self.label_scale);
                let (seed, tint) = self.in_wave(change, id, lt);
                let plain = |text: &str| {
                    if !text.contains('{') {
                        return text.to_owned();
                    }
                    let filled = text
                        .replace("{index}", &i.to_string())
                        .replace("{id}", &id.to_string())
                        .replace("{number}", &number(stream.index_of(i), decimals))
                        .replace("{x}", &number(seen[0], decimals))
                        .replace("{y}", &number(seen[1], decimals))
                        .replace("{size}", &number(get(&stream.size, i), decimals))
                        .replace("{age}", &number(get(&stream.age, i), decimals));
                    random_text(&filled, seed, id)
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
                Label {
                    text,
                    at,
                    scale: (scale / 100.0).max(0.0),
                    tint,
                }
            })
            .collect()
    }

    /// Whether a point with `change` (per cent) is in the wave at layer time
    /// `lt`, answered as the seed its random text rolls with and the colour
    /// it flashes. At rest a point keeps its own text (the instance's Seed)
    /// and the Text layer's colour (`None`).
    #[must_use]
    pub fn in_wave(self, change: f32, id: u64, lt: f64) -> (u32, Option<[f32; 4]>) {
        if change <= self.change_above {
            return (self.seed, None);
        }
        // One step per Scramble rate tick, counted from 1 so a wave always
        // shows something other than the resting text.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = if self.scramble_rate > 0.0 {
            (lt * f64::from(self.scramble_rate)).floor().max(0.0) as u32 + 1
        } else {
            1
        };
        let seed = self.seed ^ step.wrapping_mul(0x9e37_79b9);
        let roll = crate::fx::points::draw(seed, id, TINT_ATTR);
        let tint = match self.flash_colour {
            1 => Some(hue(roll)),
            2 => Some(if roll < 0.5 {
                self.colour_a
            } else {
                self.colour_b
            }),
            _ => None,
        };
        (seed, tint)
    }
}

/// A bright colour of hue `h` (0..1), scene-linear.
fn hue(h: f32) -> [f32; 4] {
    let k = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        // HSV with full saturation and value, as sRGB, then made linear.
        let v = 1.0 - (k.min(4.0 - k).clamp(0.0, 1.0));
        v.powf(2.2)
    };
    [k(5.0), k(3.0), k(1.0), 1.0]
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
        #[allow(clippy::cast_possible_truncation)]
        push(LabelPoints::DERIVED_TIME, Value::Float(cx.lt as f32));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{random_text, LabelPoints, LABEL_DRIVEN_ROWS};
    use crate::fx::params::Params;
    use crate::fx::points::PointsStream;
    use crate::fx::registry::EffectMetadata;

    /// SICELIOT's wave on Label points: a point below Change above keeps its
    /// resting text and the Text layer's colour; a point above it scrambles
    /// with time at Scramble rate and flashes one of the two colours; Label
    /// scale and Change are read per point from the columns the render
    /// writes. Guards the decode effect drawing the wrong labels.
    #[test]
    fn labels_scramble_and_flash_only_in_the_wave() {
        let mut l = LabelPoints::read(Params::EMPTY);
        l.text = crate::fx::params::ShortText::new("{chars:6}");
        l.flash_colour = 2;
        l.scramble_rate = 10.0;
        l.colour_a = [1.0, 0.0, 0.0, 1.0];
        l.colour_b = [0.0, 0.0, 1.0, 1.0];
        let mut s = PointsStream {
            position: vec![[0.0; 3]; 3],
            speed: vec![[0.0; 3]; 3],
            age: vec![0.0; 3],
            life: vec![1.0; 3],
            size: vec![8.0; 3],
            rotation: vec![0.0; 3],
            colour: vec![[1.0; 4]; 3],
            id: vec![0, 1, 2],
            ..PointsStream::default()
        };
        for (k, v) in s
            .named_mut(LABEL_DRIVEN_ROWS[0].1, 1)
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            v[0] = [0.0, 80.0, 80.0][k];
        }
        for (k, v) in s
            .named_mut(LABEL_DRIVEN_ROWS[1].1, 1)
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            v[0] = [100.0, 150.0, 50.0][k];
        }
        let at = |lt: f64| l.labels(&s, l.template(""), 1.0, lt);
        let (a, b) = (at(0.0), at(0.05));
        let rest = random_text("{chars:6}", l.seed, 0);
        assert_eq!(a[0].text, rest, "below the threshold the label rests");
        assert_eq!(a[0].tint, None);
        assert_eq!(a[0].text, b[0].text, "and does not scramble");
        assert!(a[1].tint == Some(l.colour_a) || a[1].tint == Some(l.colour_b));
        assert_ne!(
            a[1].text,
            random_text("{chars:6}", l.seed, 1),
            "in the wave it is not its rest text"
        );
        assert_eq!(a[1].text, b[1].text, "inside one tick it holds");
        assert_ne!(a[1].text, at(0.15)[1].text, "a tick later it has scrambled");
        assert!(
            (a[0].scale - 1.0).abs() < 1e-6
                && (a[1].scale - 1.5).abs() < 1e-6
                && (a[2].scale - 0.5).abs() < 1e-6
        );
    }

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
