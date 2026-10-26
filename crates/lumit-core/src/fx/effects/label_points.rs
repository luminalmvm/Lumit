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
    EnabledWhen {
        param: "hue_start",
        on: "flash_colour",
        cond: EnabledCond::ChoiceIs(3),
    },
    EnabledWhen {
        param: "hue_range",
        on: "flash_colour",
        cond: EnabledCond::ChoiceIs(3),
    },
    EnabledWhen {
        param: "saturation",
        on: "flash_colour",
        cond: EnabledCond::ChoiceIs(3),
    },
    EnabledWhen {
        param: "darken",
        on: "flash_colour",
        cond: EnabledCond::ChoiceIs(3),
    },
];

/// What a label's colour does while it is in the wave.
pub const FLASH_COLOUR: &[&str] = &["Off", "Random hue", "Two colours", "By strength"];

/// The rows a driver can give each label its own value for, and the named
/// columns the render writes those values into (SICELIOT,
/// [`crate::fx::effects::clone_to_points::drive_rows`]).
pub const LABEL_DRIVEN_ROWS: &[(&str, &str)] = &[
    ("change", "siceliot:change"),
    ("label_scale", "siceliot:label_scale"),
    ("text_step", "siceliot:text_step"),
    ("label_rotation", "siceliot:label_rotation"),
];

/// One label: its wording, where its anchor goes, how big it is drawn (1 is
/// the Text layer's size), how far it is turned (degrees, clockwise on the
/// frame, about its anchor) and its colour, `None` for the Text layer's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    pub at: [f32; 2],
    pub scale: f32,
    pub rotation: f32,
    pub tint: Option<Tint>,
}

/// How a label's colour is changed (SICELIOT): drawn all in one colour,
/// keeping only its letters' shape, or its own colours multiplied by one.
/// Both scene-linear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    Replace([f32; 4]),
    Multiply([f32; 4]),
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

    /// Each whole number here gives the random text a new roll that stays.
    /// Wire Ring wave's Passes into it and a label keeps the text the last
    /// ring left it with.
    #[slider(
        label = "Text step",
        min = 0.0,
        max = 1000.0,
        default = 0.0,
        unit = Raw
    )]
    pub text_step: f32,

    /// The colour a label takes while it is in the wave, changing at Scramble
    /// rate: none, any hue, or Colour A and Colour B in turn at random. By
    /// strength colours it smoothly by Change instead, as far as Change goes.
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

    /// By strength: the hue a little Change gives.
    #[dial(label = "Hue start", default = 0.0)]
    pub hue_start: f32,

    /// By strength: how far round the hues full Change goes, degrees.
    #[slider(
        label = "Hue range",
        min = 0.0,
        max = 360.0,
        default = 200.0,
        unit = Raw
    )]
    pub hue_range: f32,

    /// By strength: how much colour full Change gives, per cent.
    #[slider(
        label = "Saturation",
        min = 0.0,
        max = 100.0,
        default = 80.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub saturation: f32,

    /// By strength: how much darker full Change makes a label, per cent.
    #[slider(
        label = "Darken",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub darken: f32,

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

    /// How much bigger full Change makes a label, per cent, on top of Label
    /// scale.
    #[slider(
        label = "Grow",
        min = 0.0,
        max = 200.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub grow: f32,

    /// How far each label is turned about its anchor, clockwise. Wire a
    /// driver into it, Ring wave's Rotation say, and each label turns its
    /// own way.
    #[dial(label = "Label rotation", default = 0.0)]
    pub label_rotation: f32,

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
        // The driven rows, per point where a driver wrote them, else the row.
        let column_of = |row: &str| {
            let name = LABEL_DRIVEN_ROWS.iter().find(|(r, _)| *r == row)?.1;
            match stream.column(name) {
                Column::Named(_) => Some(stream.column(name)),
                _ => None,
            }
        };
        let change_column = column_of("change");
        let scale_column = column_of("label_scale");
        let step_column = column_of("text_step");
        let rotation_column = column_of("label_rotation");
        chosen
            .iter()
            .skip(chosen.len().saturating_sub(most))
            .map(|&i| {
                let seen = stream.projected(i);
                let id = stream.id.get(i).copied().unwrap_or(0);
                let read = |column: Option<Column>, row: f32| {
                    column.and_then(|c| stream.read(c, i)).unwrap_or(row)
                };
                let change = read(change_column, self.change);
                let scale = read(scale_column, self.label_scale);
                let step = read(step_column, self.text_step);
                let rotation = read(rotation_column, self.label_rotation);
                let (seed, tint) = self.in_wave(change, step, id, lt);
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
                let strength = (change / 100.0).clamp(0.0, 1.0);
                Label {
                    text,
                    at,
                    scale: (scale / 100.0 * (1.0 + strength * self.grow / 100.0)).max(0.0),
                    rotation: if rotation.is_finite() { rotation } else { 0.0 },
                    tint,
                }
            })
            .collect()
    }

    /// Whether a point with `change` (per cent) is in the wave at layer time
    /// `lt`, answered as the seed its random text rolls with and its colour.
    /// At rest a point keeps its own text (the instance's Seed, moved on by
    /// each whole Text step) and the Text layer's colour (`None`), unless By
    /// strength colours it by Change.
    #[must_use]
    pub fn in_wave(self, change: f32, text_step: f32, id: u64, lt: f64) -> (u32, Option<Tint>) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rest = if text_step.is_finite() && text_step.floor() != 0.0 {
            let n = text_step.floor() as i64 as u32;
            self.seed ^ n.wrapping_mul(0x85eb_ca6b).rotate_left(7)
        } else {
            self.seed
        };
        let by_strength = (self.flash_colour == 3 && change > 0.0).then(|| {
            let s = (change / 100.0).clamp(0.0, 1.0);
            Tint::Multiply(hsv(
                self.hue_start + s * self.hue_range,
                s * self.saturation / 100.0,
                1.0 - s * self.darken / 100.0,
            ))
        });
        if change <= self.change_above {
            return (rest, by_strength);
        }
        // One step per Scramble rate tick, counted from 1 so a wave always
        // shows something other than the resting text.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = if self.scramble_rate > 0.0 {
            (lt * f64::from(self.scramble_rate)).floor().max(0.0) as u32 + 1
        } else {
            1
        };
        let seed = rest ^ step.wrapping_mul(0x9e37_79b9);
        let roll = crate::fx::points::draw(seed, id, TINT_ATTR);
        let tint = match self.flash_colour {
            1 => Some(Tint::Replace(hsv(roll * 360.0, 1.0, 1.0))),
            2 => Some(Tint::Replace(if roll < 0.5 {
                self.colour_a
            } else {
                self.colour_b
            })),
            _ => by_strength,
        };
        (seed, tint)
    }
}

/// The colour of hue `h` (degrees), saturation `s` and value `v` (0..1), as
/// sRGB and then made scene-linear.
fn hsv(h: f32, s: f32, v: f32) -> [f32; 4] {
    let h = h.rem_euclid(360.0) / 60.0;
    let k = |n: f32| {
        let k = (n + h) % 6.0;
        let c = v - v * s * k.min(4.0 - k).clamp(0.0, 1.0);
        c.max(0.0).powf(2.2)
    };
    [k(5.0), k(3.0), k(1.0), 1.0]
}

/// One wording laid out by the text engine (SICELIOT): scene-linear,
/// premultiplied texels, row by row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelDrawing {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
}

/// One label as it lands on the frame: its drawing, where on the drawing and
/// on the frame its anchor is, how much smaller than the drawing it is drawn,
/// its turn, its colour, and the pixels round it (left, top, right, bottom,
/// right and bottom not included).
#[derive(Debug, Clone, PartialEq)]
pub struct Placed<'a> {
    pub drawing: &'a LabelDrawing,
    pub anchor: [f32; 2],
    pub at: [f32; 2],
    pub shrink: f32,
    pub cos: f32,
    pub sin: f32,
    pub tint: Option<Tint>,
    pub bounds: [i64; 4],
}

impl<'a> Placed<'a> {
    /// `label` drawn from `drawing`, made `drawn_at` times the Text layer's
    /// size, with its anchor `along` the drawing (0 its left end, 1 its
    /// right) and level with its middle. `None` for nothing to draw.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn new(
        drawing: &'a LabelDrawing,
        label: &Label,
        drawn_at: f32,
        along: f32,
    ) -> Option<Self> {
        if drawing.width == 0
            || drawing.height == 0
            || drawn_at <= 0.0
            || !label.at.iter().all(|v| v.is_finite())
        {
            return None;
        }
        let (w, h) = (drawing.width as f32, drawing.height as f32);
        let anchor = [along * w, 0.5 * h];
        let shrink = label.scale / drawn_at;
        let (sin, cos) = label.rotation.to_radians().sin_cos();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for corner in [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]] {
            let q = [
                (corner[0] - anchor[0]) * shrink,
                (corner[1] - anchor[1]) * shrink,
            ];
            let p = [
                label.at[0] + q[0] * cos - q[1] * sin,
                label.at[1] + q[0] * sin + q[1] * cos,
            ];
            for a in 0..2 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let whole = |v: f32| v.clamp(-1e9, 1e9) as i64;
        Some(Self {
            drawing,
            anchor,
            at: label.at,
            shrink,
            cos,
            sin,
            tint: label.tint,
            bounds: [
                whole(lo[0].floor()) - 1,
                whole(lo[1].floor()) - 1,
                whole(hi[0].ceil()) + 1,
                whole(hi[1].ceil()) + 1,
            ],
        })
    }
}

/// The labels laid one over the other, in order, into a picture `size`
/// pixels big whose top left corner is `origin` on the frame: scene-linear,
/// premultiplied RGBA. Each pixel's middle is turned and grown back onto its
/// label's drawing and read between four texels, so labels sit between
/// pixels at any size and turn. Bands of rows are shared out over the cores.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub fn composite(placed: &[Placed<'_>], origin: [i64; 2], size: [i64; 2]) -> Vec<f32> {
    use rayon::prelude::*;
    const BAND: i64 = 16;
    let [x0, y0] = origin;
    let [bw, bh] = [size[0].max(0), size[1].max(0)];
    let mut px = vec![0.0f32; (bw * bh * 4) as usize];
    if bw == 0 {
        return px;
    }
    px.par_chunks_mut((BAND * bw * 4) as usize)
        .enumerate()
        .for_each(|(band, rows)| {
            let top = y0 + band as i64 * BAND;
            let bottom = top + rows.len() as i64 / (bw * 4);
            for l in placed
                .iter()
                .filter(|l| l.bounds[1] < bottom && l.bounds[3] > top)
            {
                let d = l.drawing;
                let (dw, dh) = (d.width as i64, d.height as i64);
                let texel = |x: i64, y: i64| -> [f32; 4] {
                    if (0..dw).contains(&x) && (0..dh).contains(&y) {
                        d.px[(y * dw + x) as usize]
                    } else {
                        [0.0; 4]
                    }
                };
                for y in l.bounds[1].max(top)..l.bounds[3].min(bottom) {
                    for x in l.bounds[0].max(x0)..l.bounds[2].min(x0 + bw) {
                        let (dx, dy) = (x as f32 + 0.5 - l.at[0], y as f32 + 0.5 - l.at[1]);
                        let u = (dx * l.cos + dy * l.sin) / l.shrink + l.anchor[0] - 0.5;
                        let v = (-dx * l.sin + dy * l.cos) / l.shrink + l.anchor[1] - 0.5;
                        if u <= -1.0 || v <= -1.0 || u >= dw as f32 || v >= dh as f32 {
                            continue;
                        }
                        let (fu, fv) = (u.floor(), v.floor());
                        let (tu, tv) = (u - fu, v - fv);
                        let (iu, iv) = (fu as i64, fv as i64);
                        let (a, b, c, e) = (
                            texel(iu, iv),
                            texel(iu + 1, iv),
                            texel(iu, iv + 1),
                            texel(iu + 1, iv + 1),
                        );
                        let s: [f32; 4] = std::array::from_fn(|n| {
                            let upper = a[n] + (b[n] - a[n]) * tu;
                            let lower = c[n] + (e[n] - c[n]) * tu;
                            upper + (lower - upper) * tv
                        });
                        let alpha = s[3];
                        if alpha <= 0.0 {
                            continue;
                        }
                        // A flashing label keeps its letters' coverage and
                        // takes the flash colour in place of the Text layer's;
                        // one coloured by strength keeps its own, multiplied.
                        let over = match l.tint {
                            Some(Tint::Replace(c)) => [c[0] * alpha, c[1] * alpha, c[2] * alpha],
                            Some(Tint::Multiply(m)) => [s[0] * m[0], s[1] * m[1], s[2] * m[2]],
                            None => [s[0], s[1], s[2]],
                        };
                        let at = (((y - top) * bw + (x - x0)) * 4) as usize;
                        let Some(under) = rows.get_mut(at..at + 4) else {
                            continue;
                        };
                        for (under, top) in under.iter_mut().zip(over.into_iter().chain([alpha])) {
                            *under = top + *under * (1.0 - alpha);
                        }
                    }
                }
            }
        });
    px
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
    use super::{random_text, LabelPoints, Tint, LABEL_DRIVEN_ROWS};
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
        assert!(
            a[1].tint == Some(Tint::Replace(l.colour_a))
                || a[1].tint == Some(Tint::Replace(l.colour_b))
        );
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

    /// SICELIOT's ring look on Label points: Text step moves the resting
    /// text on and keeps it, By strength tints smoothly by Change (none at
    /// nought, more colour and darker as it grows), Grow sizes by Change, and
    /// Label rotation is read per point. Guards the text a ring leaves behind
    /// and the colours and sizes it gives.
    #[test]
    fn labels_keep_the_text_a_ring_leaves_and_colour_by_strength() {
        let mut l = LabelPoints::read(Params::EMPTY);
        l.text = crate::fx::params::ShortText::new("{chars:4}");
        l.flash_colour = 3;
        l.change_above = 80.0;
        l.grow = 50.0;
        assert_eq!(l.in_wave(0.0, 0.0, 5, 0.0), (l.seed, None));
        let (moved, _) = l.in_wave(0.0, 3.0, 5, 0.0);
        assert_ne!(moved, l.seed, "a whole step rolls a new text");
        assert_eq!(moved, l.in_wave(0.0, 3.7, 5, 9.0).0, "and keeps it");
        let tint = |change: f32| match l.in_wave(change, 0.0, 5, 0.0).1 {
            Some(Tint::Multiply(c)) => c,
            other => panic!("{other:?}"),
        };
        let (weak, strong) = (tint(10.0), tint(70.0));
        let spread = |c: [f32; 4]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
        assert!(spread(strong) > spread(weak), "{weak:?} {strong:?}");
        assert!(strong.iter().take(3).all(|v| *v <= 1.0));
        let mut s = PointsStream {
            position: vec![[0.0; 3]; 2],
            speed: vec![[0.0; 3]; 2],
            age: vec![0.0; 2],
            life: vec![1.0; 2],
            size: vec![8.0; 2],
            rotation: vec![0.0; 2],
            colour: vec![[1.0; 4]; 2],
            id: vec![0, 1],
            ..PointsStream::default()
        };
        let mut write = |row: &str, values: [f32; 2]| {
            let name = LABEL_DRIVEN_ROWS.iter().find(|(r, _)| *r == row).unwrap().1;
            for (k, v) in s.named_mut(name, 1).unwrap().iter_mut().enumerate() {
                v[0] = values[k];
            }
        };
        write("change", [0.0, 100.0]);
        write("label_rotation", [-30.0, 45.0]);
        let labels = l.labels(&s, l.template(""), 1.0, 0.0);
        assert!((labels[0].scale - 1.0).abs() < 1e-6);
        assert!(
            (labels[1].scale - 1.5).abs() < 1e-6,
            "Grow 50 at full Change"
        );
        assert_eq!((labels[0].rotation, labels[1].rotation), (-30.0, 45.0));
        assert_eq!(labels[0].tint, None);
    }

    /// SICELIOT's drawing of labels: a label lands where it is asked, between
    /// pixels when it sits between them, turns about its anchor, shrinks from
    /// its drawing and takes its tint. Guards labels drawn in the wrong
    /// place, snapped back to whole pixels or never turned.
    #[test]
    fn labels_are_drawn_between_pixels_turned_and_tinted() {
        use super::{composite, Label, LabelDrawing, Placed};
        // A 4 x 2 white bar.
        let bar = LabelDrawing {
            width: 4,
            height: 2,
            px: vec![[1.0; 4]; 8],
        };
        let label = |at: [f32; 2], rotation: f32, tint: Option<Tint>| Label {
            text: String::new(),
            at,
            scale: 1.0,
            rotation,
            tint,
        };
        let alpha = |px: &[f32], x: usize, y: usize| px[(y * 10 + x) * 4 + 3];
        // Centred on (5, 5): it covers x 3..7 and y 4..6, nothing above.
        let flat = label([5.0, 5.0], 0.0, None);
        let px = composite(
            &[Placed::new(&bar, &flat, 1.0, 0.5).unwrap()],
            [0, 0],
            [10, 10],
        );
        assert!(alpha(&px, 4, 4) > 0.99 && alpha(&px, 5, 5) > 0.99);
        assert!(alpha(&px, 5, 2) == 0.0 && alpha(&px, 1, 5) == 0.0);
        // Half a pixel across, the edge pixels are half covered.
        let half = label([5.5, 5.0], 0.0, None);
        let px = composite(
            &[Placed::new(&bar, &half, 1.0, 0.5).unwrap()],
            [0, 0],
            [10, 10],
        );
        assert!(
            (alpha(&px, 3, 4) - 0.5).abs() < 0.26,
            "{}",
            alpha(&px, 3, 4)
        );
        // Turned a quarter, it stands up: y 3..7, x 4..6.
        let up = label([5.0, 5.0], 90.0, None);
        let px = composite(
            &[Placed::new(&bar, &up, 1.0, 0.5).unwrap()],
            [0, 0],
            [10, 10],
        );
        assert!(alpha(&px, 4, 3) > 0.99 && alpha(&px, 5, 6) > 0.99);
        assert!(alpha(&px, 2, 5) == 0.0 && alpha(&px, 7, 5) == 0.0);
        // Drawn twice as big as it is shown, it covers half as far.
        let small = label([5.0, 5.5], 0.0, Some(Tint::Multiply([1.0, 0.5, 0.0, 1.0])));
        let placed = Placed::new(&bar, &small, 2.0, 0.5).unwrap();
        let px = composite(&[placed], [0, 0], [10, 10]);
        assert!(alpha(&px, 4, 5) > 0.99 && alpha(&px, 2, 5) == 0.0 && alpha(&px, 4, 3) == 0.0);
        assert_eq!(
            &px[(5 * 10 + 4) * 4..(5 * 10 + 4) * 4 + 3],
            &[1.0, 0.5, 0.0]
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
