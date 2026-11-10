//! Text animators: a Text layer's words moved a letter at a time.
//!
//! # In plain terms
//!
//! A Text layer normally moves as one picture — the whole line slides, turns
//! and fades together. An **animator** lets the letters move *separately*, and
//! a **range selector** says which letters are moved and by how much. Slide the
//! range along the words over time and you have the cascade every title
//! sequence is made of: each letter drops in, turns up, or fades on as the
//! range reaches it, and settles once the range has gone past.
//!
//! Three things make one up:
//!
//! - the **properties** — how far a moved letter is pushed, turned, scaled,
//!   faded and tinted. Ordinary keyframeable numbers, so they animate like
//!   everything else in the document;
//! - the **selector** — `start`, `end` and `offset`, all per cent of the run,
//!   which mark out the stretch of the words the animator applies to;
//! - the **weight** — what the selector hands each letter: `1` for a letter
//!   fully inside the range, `0` for one outside it, and something in between
//!   where the shape says so. The properties are applied *times* the weight, so
//!   a letter half in the range is moved half as far.
//!
//! **Deliberately small for v1** (the decision entry argues it): one selector
//! per animator, two shapes rather than After Effects' six, no random order, no
//! wiggle, and every animator carries the same five property groups rather than
//! a menu of thirty. The shape of the model is AE's, so the rest bolts on; what
//! is here is the part that makes cascade titles.

use serde::{Deserialize, Serialize};

use crate::anim::Property;

/// What the selector counts: single letters, or whole words.
///
/// The difference is what a weight is *attached* to. Counting characters gives
/// the letter-by-letter cascade; counting words moves each word as a unit, so
/// the letters of one word arrive together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorBasis {
    #[default]
    Characters,
    Words,
}

/// How the weight falls off across the range.
///
/// `Square` is in-or-out: everything inside the range is moved the whole way,
/// everything outside is left alone. `Ramp` rises evenly from nothing at the
/// range's start to the whole way at its end, and stays there afterwards —
/// which is what turns a cascade into a sweep.
///
/// `Smooth` is the ramp eased in and out, so the letters at either end of the
/// slope barely move and the ones in the middle of it move most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorShape {
    #[default]
    Square,
    Ramp,
    Smooth,
}

/// How a selector picks its letters: by where they come in the words, or by
/// how near they are to another layer.
///
/// `Proximity` weighs each letter by its distance from a **target layer**,
/// such as a null: a letter on the target is moved the whole way, one
/// `radius` or further from it is left alone, and the shape says how the
/// weight falls off between the two. Dragging the target across the words
/// sweeps the change through them, which is what After Effects does with a
/// null and an expression on every letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorKind {
    #[default]
    Range,
    Proximity,
    /// Each letter's weight wanders on its own between `min_amount` and
    /// `max_amount`, `wiggles` times a second, After Effects' Wiggly selector.
    /// `correlation` is how much the letters wander together: 100 % moves
    /// them as one, 0 % each on its own.
    Wiggly,
}

fn hundred_wiggly() -> Property {
    Property::fixed(100.0)
}

fn two() -> Property {
    Property::fixed(2.0)
}

fn fifty() -> Property {
    Property::fixed(50.0)
}

fn is_static_two(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 2.0) && p.extra.is_empty()
}

fn is_static_fifty(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 50.0) && p.extra.is_empty()
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// serde default for [`RangeSelector::radius`].
fn two_hundred() -> Property {
    Property::fixed(200.0)
}

fn is_static_two_hundred(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 200.0) && p.extra.is_empty()
}

/// serde default for [`RangeSelector::end`]: the whole run.
fn hundred() -> Property {
    Property::fixed(100.0)
}

fn is_static_hundred(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 100.0) && p.extra.is_empty()
}

fn is_static_zero(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 0.0) && p.extra.is_empty()
}

/// Which stretch of the words an animator applies to, in **per cent of the
/// run** — 0 is before the first unit, 100 is past the last.
///
/// Per cent rather than a letter count, so the same selector reads the same on
/// a word and on a sentence, and so an expression-driven line whose length
/// changes every frame does not need its keyframes rewriting. `offset` slides
/// the whole `start`–`end` window along without disturbing its width, which is
/// the one number a cascade is usually keyed on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeSelector {
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub start: Property,
    #[serde(
        default = "hundred",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_hundred"
    )]
    pub end: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub offset: Property,
    #[serde(default, skip_serializing_if = "is_default_basis")]
    pub basis: SelectorBasis,
    #[serde(default, skip_serializing_if = "is_default_shape")]
    pub shape: SelectorShape,
    /// Range or Proximity. A Proximity selector ignores `start`, `end` and
    /// `offset` and reads the three fields below instead.
    #[serde(default, skip_serializing_if = "is_default_kind")]
    pub kind: SelectorKind,
    /// The layer whose anchor the letters are measured from. Unset, or a layer
    /// since deleted, reaches no letter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<uuid::Uuid>,
    /// How far from the target a letter is still reached, in the text layer's
    /// own pixels.
    #[serde(
        default = "two_hundred",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_two_hundred"
    )]
    pub radius: Property,
    /// Turns the weights over: the letters far from the target are moved and
    /// the near ones are left alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
    /// A Wiggly selector's range of weights, per cent: each letter wanders
    /// between the two.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub min_amount: Property,
    #[serde(
        default = "hundred_wiggly",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_hundred"
    )]
    pub max_amount: Property,
    /// How many times a second a Wiggly selector's weights change course.
    #[serde(
        default = "two",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_two"
    )]
    pub wiggles: Property,
    /// Per cent: how much the letters wander together.
    #[serde(
        default = "fifty",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_fifty"
    )]
    pub correlation: Property,
    /// Picks another wander for the same settings.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub seed: u32,
}

fn is_default_kind(k: &SelectorKind) -> bool {
    *k == SelectorKind::Range
}

fn is_default_basis(b: &SelectorBasis) -> bool {
    *b == SelectorBasis::Characters
}

fn is_default_shape(s: &SelectorShape) -> bool {
    *s == SelectorShape::Square
}

impl Default for RangeSelector {
    fn default() -> Self {
        Self {
            start: Property::zero(),
            end: hundred(),
            offset: Property::zero(),
            basis: SelectorBasis::default(),
            shape: SelectorShape::default(),
            kind: SelectorKind::default(),
            target: None,
            radius: two_hundred(),
            invert: false,
            min_amount: Property::zero(),
            max_amount: hundred_wiggly(),
            wiggles: two(),
            correlation: fifty(),
            seed: 0,
        }
    }
}

impl RangeSelector {
    /// The weight a Wiggly selector gives each of `units` at layer time `lt`.
    #[must_use]
    pub fn wiggly_weights(&self, units: usize, lt: f64) -> Vec<f32> {
        let rate = self.wiggles.value_at(lt);
        let x = if rate.is_finite() { lt * rate } else { 0.0 };
        let together = (self.correlation.value_at(lt) / 100.0).clamp(0.0, 1.0);
        let (lo, hi) = (
            self.min_amount.value_at(lt) / 100.0,
            self.max_amount.value_at(lt) / 100.0,
        );
        let shared = crate::fx::maths::value_noise_1d(self.seed, 0, x);
        (0..units)
            .map(|i| {
                #[allow(clippy::cast_possible_truncation)] // a line is short
                let own = crate::fx::maths::value_noise_1d(self.seed, 1 + i as u32, x);
                let n = together * shared + (1.0 - together) * own;
                #[allow(clippy::cast_possible_truncation)] // a weight
                let w = (lo + (hi - lo) * (n + 1.0) * 0.5) as f32;
                if w.is_finite() {
                    w
                } else {
                    0.0
                }
            })
            .collect()
    }
}

impl RangeSelector {
    /// The weight this selector gives each **unit** of a run of `units`, at
    /// layer time `lt`.
    ///
    /// A run with nothing in it has no weights, which is the empty line.
    #[must_use]
    pub fn weights_at(&self, units: usize, lt: f64) -> Vec<f32> {
        if units == 0 {
            return Vec::new();
        }
        let offset = self.offset.value_at(lt);
        let (mut lo, mut hi) = (
            (self.start.value_at(lt) + offset) / 100.0,
            (self.end.value_at(lt) + offset) / 100.0,
        );
        // A range dragged inside out still means the stretch between its ends.
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let shape = self.shape;
        (0..units)
            .map(|i| {
                // The middle of the unit's own share of the run, so the first
                // and last units are treated the same way as the ones between
                // them — a selector at 0 % has not reached the first letter's
                // middle yet, and one at 100 % has passed the last letter's.
                #[allow(clippy::cast_precision_loss)] // a run of 2^24 letters is not a line
                let p = (i as f64 + 0.5) / units as f64;
                weight(shape, lo, hi, p)
            })
            .collect()
    }

    /// The weight a Proximity selector gives each unit, from the units'
    /// middles in the text layer's pixels and the target's place there. A unit
    /// with no middle (a line break) and a selector with no target come out
    /// nought, so they are left alone.
    #[must_use]
    pub fn proximity_weights(
        &self,
        middles: &[Option<[f32; 2]>],
        target: Option<[f32; 2]>,
        lt: f64,
    ) -> Vec<f32> {
        let Some(target) = target else {
            return vec![0.0; middles.len()];
        };
        #[allow(clippy::cast_possible_truncation)] // px
        let radius = self.radius.value_at(lt).max(0.0) as f32;
        middles
            .iter()
            .map(|m| {
                let Some(m) = m else { return 0.0 };
                let d = (m[0] - target[0]).hypot(m[1] - target[1]);
                let w = near_weight(self.shape, d, radius);
                if self.invert {
                    1.0 - w
                } else {
                    w
                }
            })
            .collect()
    }
}

/// How much a letter `d` px from the target is reached by a selector of
/// `radius`: all of it on the target, none at the radius and beyond.
fn near_weight(shape: SelectorShape, d: f32, radius: f32) -> f32 {
    if !d.is_finite() {
        return 0.0;
    }
    if radius <= 0.0 {
        return f32::from(u8::from(d <= 0.0));
    }
    let u = (1.0 - d / radius).clamp(0.0, 1.0);
    match shape {
        SelectorShape::Square => f32::from(u8::from(d <= radius)),
        SelectorShape::Ramp => u,
        SelectorShape::Smooth => u * u * (3.0 - 2.0 * u),
    }
}

/// One unit's weight for a range `lo`–`hi` at position `p`, all in 0–1.
#[must_use]
fn weight(shape: SelectorShape, lo: f64, hi: f64, p: f64) -> f32 {
    let w = match shape {
        SelectorShape::Square => f64::from(u8::from(p >= lo && p < hi)),
        // A zero-width ramp is a step, which is the honest limit of the ramp
        // rather than a division by nothing.
        SelectorShape::Ramp | SelectorShape::Smooth if hi <= lo => f64::from(u8::from(p >= hi)),
        SelectorShape::Ramp => ((p - lo) / (hi - lo)).clamp(0.0, 1.0),
        SelectorShape::Smooth => {
            let u = ((p - lo) / (hi - lo)).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        }
    };
    #[allow(clippy::cast_possible_truncation)] // 0–1
    let w = w as f32;
    w
}

/// One animator group: a set of per-letter offsets and the range they apply to.
///
/// **Every animator carries all five property groups**, defaulted to values
/// that change nothing — After Effects offers a menu of properties to add one
/// at a time, and a menu of thirty is not what makes a cascade. Adding an
/// animator here gives you the five rows, four of which you leave alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextAnimator {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_default_selector")]
    pub selector: RangeSelector,
    /// How far a moved letter is pushed, px@comp — measured in the letter's own
    /// frame, so a letter on a curve is pushed along and away from the curve
    /// rather than across the picture.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub position_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub position_y: Property,
    /// Degrees, turned about the letter's own middle.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub rotation: Property,
    /// How far a letter moves away from the eye, and how far it turns about
    /// x and y through its own middle on the baseline, degrees. Only a solid
    /// layer's letters can.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub position_z: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub rotation_x: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub rotation_y: Property,
    /// Per cent; 100 leaves the letter its own size.
    #[serde(
        default = "hundred",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_hundred"
    )]
    pub scale_x: Property,
    #[serde(
        default = "hundred",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_hundred"
    )]
    pub scale_y: Property,
    /// Per cent; 100 leaves the letter alone, 0 takes it away entirely.
    #[serde(
        default = "hundred",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_hundred"
    )]
    pub opacity: Property,
    /// Added to the layer's fill in scene-linear, so 0 leaves the colour alone
    /// and a positive red lifts the letter's red — an **offset**, not a second
    /// colour, because that composes when two animators reach the same letter.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub fill_r: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub fill_g: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub fill_b: Property,
    /// **Variable font axes**: where a reached letter is set on its font's
    /// own axes, such as weight or width. Unlike the offsets above, each is a
    /// place on the axis rather than an amount added, so a weight of 900 asks
    /// for the font's heaviest whatever the layer is set at; a letter half
    /// reached is set half way there. Only the axes the font has do anything,
    /// and a layer moves at most [`MAX_ANIMATED_AXES`] of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<AnimatorAxis>,
    /// **Which property groups the animator shows**, by name (`position`,
    /// `rotation`, `scale`, `opacity`, `fill`, `position_z`, `rotation_x`,
    /// `rotation_y`), in the order they were added, as After Effects' Add ▸
    /// Property builds an animator up. Unset shows them all, which is every
    /// animator made before there was a choice. A group whose numbers have
    /// been moved shows whatever this says. It decides only what is listed:
    /// every number counts whether listed or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
    /// Unknown fields from newer Lumit versions (docs/10 §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// How many font axes one Text layer's animators can move, as many as After
/// Effects allows.
pub const MAX_ANIMATED_AXES: usize = 8;

/// One font axis an animator moves the letters along.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimatorAxis {
    /// The axis's four-letter tag, such as `wght`, `wdth`, `slnt` or a
    /// font's own upper-case one.
    pub tag: String,
    /// The place on the axis, in the font's own numbers (a weight of 100 to
    /// 900, a width in per cent, a slant in degrees).
    #[serde(with = "crate::mask::still_or_keyed")]
    pub value: Property,
}

/// A place the whole layer is set at on one of its font's axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontAxisValue {
    pub tag: String,
    pub value: f64,
}

/// An axis tag as the four bytes a font stores it as, spaces filling a short
/// one. `None` for a tag that isn't one to four printable ASCII letters.
#[must_use]
pub fn axis_tag(tag: &str) -> Option<[u8; 4]> {
    let bytes = tag.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let mut out = [b' '; 4];
    out[..bytes.len()].copy_from_slice(bytes);
    Some(out)
}

fn is_default_selector(s: &RangeSelector) -> bool {
    *s == RangeSelector::default()
}

impl TextAnimator {
    /// A fresh animator that changes nothing until a number is moved.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            selector: RangeSelector::default(),
            position_x: Property::zero(),
            position_y: Property::zero(),
            rotation: Property::zero(),
            position_z: Property::zero(),
            rotation_x: Property::zero(),
            rotation_y: Property::zero(),
            scale_x: hundred(),
            scale_y: hundred(),
            opacity: hundred(),
            fill_r: Property::zero(),
            fill_g: Property::zero(),
            fill_b: Property::zero(),
            axes: Vec::new(),
            properties: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// What one letter is asked to do, once every animator has had its say.
///
/// The identity — no push, no turn, full size, full opacity, no tint — is what
/// a letter no animator reaches comes out as, and is what makes a layer with no
/// animators draw exactly the bytes it drew before there were any.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphXform {
    /// px@comp, in the letter's own frame.
    pub position: [f32; 2],
    /// Degrees about the letter's middle.
    pub rotation: f32,
    /// Multipliers; 1.0 is the letter's own size.
    pub scale: [f32; 2],
    /// 0–1.
    pub opacity: f32,
    /// Added to the layer's fill, scene-linear.
    pub fill: [f32; 3],
    /// px@comp away from the eye, and degrees about x and y. A solid's alone.
    pub depth: f32,
    pub turn: [f32; 2],
    /// Where the letter is set on its font's axes, relative to the layer's own
    /// setting, which only the font knows.
    pub axes: GlyphAxes,
}

impl Default for GlyphXform {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            rotation: 0.0,
            scale: [1.0, 1.0],
            opacity: 1.0,
            fill: [0.0, 0.0, 0.0],
            depth: 0.0,
            turn: [0.0, 0.0],
            axes: GlyphAxes::default(),
        }
    }
}

/// A letter's places on up to [`MAX_ANIMATED_AXES`] font axes, each written
/// as `keep × layer's value + add`.
///
/// The animators are worked out before the font is opened, so the layer's own
/// place on an axis is not known yet. Each animator moves the letter from
/// where it was towards its own value by its weight, and that is a straight
/// line in the layer's value, so two numbers carry it: a letter no animator
/// reaches keeps all of the layer's value and adds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GlyphAxes {
    pub tags: [[u8; 4]; MAX_ANIMATED_AXES],
    pub keep: [f32; MAX_ANIMATED_AXES],
    pub add: [f32; MAX_ANIMATED_AXES],
    pub len: u8,
}

impl GlyphAxes {
    /// Move the letter towards `value` on `tag` by `weight`. A ninth axis is
    /// dropped.
    pub fn mix(&mut self, tag: [u8; 4], value: f32, weight: f32) {
        let len = usize::from(self.len);
        let i = match self.tags[..len].iter().position(|t| *t == tag) {
            Some(i) => i,
            None if len < MAX_ANIMATED_AXES => {
                self.tags[len] = tag;
                self.keep[len] = 1.0;
                self.add[len] = 0.0;
                self.len += 1;
                len
            }
            None => return,
        };
        self.keep[i] *= 1.0 - weight;
        self.add[i] = self.add[i] * (1.0 - weight) + weight * value;
    }

    /// Each axis this letter is moved on, and its place there given the
    /// layer's value from `base`.
    pub fn resolved<'a>(
        &'a self,
        base: &'a dyn Fn([u8; 4]) -> f32,
    ) -> impl Iterator<Item = ([u8; 4], f32)> + 'a {
        (0..usize::from(self.len)).map(move |i| {
            let tag = self.tags[i];
            (tag, self.keep[i] * base(tag) + self.add[i])
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl GlyphXform {
    /// True when this letter is left exactly as the font drew it.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// Which **unit** each character of `text` belongs to, and how many units there
/// are, for a selector counting the way `basis` says.
///
/// Counting characters, every character is its own unit and this is simply
/// `0, 1, 2, …`. Counting words, the characters of one word share its index —
/// and **the spaces between words go with the word before them**, so a range
/// sweeping across a sentence does not leave the gaps behaving like a word of
/// their own. Leading spaces go with the first word, for the same reason.
#[must_use]
pub fn unit_indices(text: &str, basis: SelectorBasis) -> (Vec<usize>, usize) {
    match basis {
        SelectorBasis::Characters => {
            let n = text.chars().count();
            ((0..n).collect(), n)
        }
        SelectorBasis::Words => {
            let mut out = Vec::with_capacity(text.len());
            let mut word = 0usize;
            let mut in_word = false;
            let mut any = false;
            for ch in text.chars() {
                if ch.is_whitespace() {
                    // A gap belongs to the word it follows.
                    out.push(word);
                    in_word = false;
                } else {
                    if !in_word {
                        if any {
                            word += 1;
                        }
                        in_word = true;
                        any = true;
                    }
                    out.push(word);
                }
            }
            (out, usize::from(any) * (word + 1))
        }
    }
}

/// What each character of `text` is asked to do at layer time `lt`.
///
/// Empty when there are no animators, which is the whole of the byte-identity
/// guarantee: the caller draws the line the way it always drew it rather than
/// taking a second code path that happens to agree.
///
/// Two animators reaching the same letter **compose**: their pushes, turns and
/// tints add, their scales and opacities multiply. That is the only combination
/// that reads as "and also" rather than "instead of", and it is what lets a
/// fade animator and a drop animator be written separately.
#[must_use]
pub fn glyph_xforms(animators: &[TextAnimator], text: &str, lt: f64) -> Vec<GlyphXform> {
    glyph_xforms_near(animators, text, lt, &Near::default())
}

/// Where a Proximity selector measures from: the middle of every character
/// in the text layer's own pixels, and the targets' anchors there.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Near {
    /// One per character; `None` for a character with no place, such as a
    /// line break. Empty when the letters weren't laid out.
    pub middles: Vec<Option<[f32; 2]>>,
    pub targets: Vec<(uuid::Uuid, [f32; 2])>,
}

impl Near {
    fn target(&self, id: Option<uuid::Uuid>) -> Option<[f32; 2]> {
        let id = id?;
        self.targets.iter().find(|(t, _)| *t == id).map(|(_, p)| *p)
    }
}

/// [`glyph_xforms`] with the letters' places, which a Proximity selector
/// needs.
#[must_use]
pub fn glyph_xforms_near(
    animators: &[TextAnimator],
    text: &str,
    lt: f64,
    near: &Near,
) -> Vec<GlyphXform> {
    if animators.is_empty() {
        return Vec::new();
    }
    let count = text.chars().count();
    if count == 0 {
        return Vec::new();
    }
    let mut out = vec![GlyphXform::default(); count];
    for animator in animators {
        let (units, total) = unit_indices(text, animator.selector.basis);
        let weights = match animator.selector.kind {
            SelectorKind::Range => animator.selector.weights_at(total, lt),
            SelectorKind::Wiggly => animator.selector.wiggly_weights(total, lt),
            SelectorKind::Proximity => {
                // A unit's middle is the mean of its characters' middles, so a
                // word is measured from its own middle.
                let mut sums = vec![([0.0f32; 2], 0u32); total];
                for (ch, unit) in units.iter().enumerate() {
                    if let (Some(Some(m)), Some(sum)) = (near.middles.get(ch), sums.get_mut(*unit))
                    {
                        sum.0[0] += m[0];
                        sum.0[1] += m[1];
                        sum.1 += 1;
                    }
                }
                #[allow(clippy::cast_precision_loss)] // a word is short
                let middles: Vec<Option<[f32; 2]>> = sums
                    .iter()
                    .map(|(s, n)| (*n > 0).then(|| [s[0] / *n as f32, s[1] / *n as f32]))
                    .collect();
                animator.selector.proximity_weights(
                    &middles,
                    near.target(animator.selector.target),
                    lt,
                )
            }
        };
        if weights.is_empty() {
            continue;
        }
        #[allow(clippy::cast_possible_truncation)] // a place on a font's axis
        let axes: Vec<([u8; 4], f32)> = animator
            .axes
            .iter()
            .filter_map(|a| Some((axis_tag(&a.tag)?, a.value.value_at(lt) as f32)))
            .filter(|(_, v)| v.is_finite())
            .collect();
        #[allow(clippy::cast_possible_truncation)] // px, degrees and per cent, all f32 pictures
        let (px, py, rot, sx, sy, opacity, fr, fg, fb) = (
            animator.position_x.value_at(lt) as f32,
            animator.position_y.value_at(lt) as f32,
            animator.rotation.value_at(lt) as f32,
            animator.scale_x.value_at(lt) as f32 / 100.0,
            animator.scale_y.value_at(lt) as f32 / 100.0,
            animator.opacity.value_at(lt) as f32 / 100.0,
            animator.fill_r.value_at(lt) as f32,
            animator.fill_g.value_at(lt) as f32,
            animator.fill_b.value_at(lt) as f32,
        );
        #[allow(clippy::cast_possible_truncation)] // px and degrees, f32 pictures
        let (depth, turn_x, turn_y) = (
            animator.position_z.value_at(lt) as f32,
            animator.rotation_x.value_at(lt) as f32,
            animator.rotation_y.value_at(lt) as f32,
        );
        for (i, x) in out.iter_mut().enumerate() {
            let Some(w) = units.get(i).and_then(|u| weights.get(*u)).copied() else {
                continue;
            };
            if w == 0.0 {
                continue;
            }
            x.position[0] += w * px;
            x.position[1] += w * py;
            x.rotation += w * rot;
            x.depth += w * depth;
            x.turn[0] += w * turn_x;
            x.turn[1] += w * turn_y;
            // A scale of 100 % has to leave the letter alone at every weight,
            // so the weight interpolates from 1 rather than from 0.
            x.scale[0] *= 1.0 + w * (sx - 1.0);
            x.scale[1] *= 1.0 + w * (sy - 1.0);
            x.opacity *= 1.0 + w * (opacity - 1.0);
            x.fill[0] += w * fr;
            x.fill[1] += w * fg;
            x.fill[2] += w * fb;
            for (tag, value) in &axes {
                x.axes.mix(*tag, *value, w);
            }
        }
    }
    out
}

// ---- Character and paragraph style --------------------------------------

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_hundred(v: &f64) -> bool {
    *v == 100.0
}

fn is_one(v: &f64) -> bool {
    *v == 1.0
}

fn is_true(v: &bool) -> bool {
    *v
}

fn yes() -> bool {
    true
}

fn hundred_per_cent() -> f64 {
    100.0
}

fn one() -> f64 {
    1.0
}

fn black() -> crate::model::LinearColour {
    crate::model::LinearColour::BLACK
}

fn is_black(c: &crate::model::LinearColour) -> bool {
    *c == crate::model::LinearColour::BLACK
}

/// Whether pairs of letters are pulled together by the font's own kerning.
///
/// `Off` is what every Text layer drew before there was a choice, so it stays
/// the value a file with no key reads as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Kerning {
    #[default]
    Off,
    Metrics,
}

/// Capitals: as typed, all capitals, or small capitals for the lower case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Caps {
    #[default]
    Normal,
    All,
    Small,
}

/// Where the letters sit against the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Script {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

/// How the letters make room for each other when an animator sets them wider
/// or narrower on a font axis.
///
/// - `Adaptive`: each letter takes the room it now needs, and every line is
///   lined up again the way its paragraph says, so centred words stay centred
///   as they grow.
/// - `PerCharacter`: every letter keeps the room it has at the layer's own
///   setting and grows or shrinks about its middle there, so nothing else on
///   the line moves (letters may touch).
/// - `Default`: each letter takes the room it now needs and the line grows
///   from its start, the way the font's own widths lay it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AxisSpacing {
    #[default]
    Adaptive,
    PerCharacter,
    Default,
}

/// Which side the lines of a block line up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Centre,
    Right,
}

/// How a Text layer's letters are set: the font, the spacing, the scale and
/// the outline.
///
/// One style for the whole layer. Every field is left out of the file while
/// it holds its default, so a layer nobody has styled writes no `style` key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// The font family as the system lists it. Empty is the built-in Inter.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub family: String,
    /// The face inside the family, such as "Bold Italic". Empty is the
    /// family's regular face.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub face: String,
    /// **A variable font's axes**, set for the whole layer on top of the face:
    /// the face is a named place in the font's design space, and these move
    /// the letters off it. An axis the font doesn't have is kept and does
    /// nothing, so a project opened where the font is missing keeps its
    /// settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<FontAxisValue>,
    /// How the letters make room for each other while animators move them on
    /// the font's axes, After Effects' Variable Font Spacing.
    #[serde(default, skip_serializing_if = "is_default")]
    pub axis_spacing: AxisSpacing,
    /// Baseline to baseline in px. Unset is auto, 120 % of the size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leading: Option<f64>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub kerning: Kerning,
    /// Extra space after every letter, in thousandths of an em.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tracking: f64,
    /// Per cent. Stretches the letters and their advances sideways.
    #[serde(default = "hundred_per_cent", skip_serializing_if = "is_hundred")]
    pub scale_x: f64,
    /// Per cent. Stretches the letters up from the baseline.
    #[serde(default = "hundred_per_cent", skip_serializing_if = "is_hundred")]
    pub scale_y: f64,
    /// Px the letters are lifted off the baseline, positive is up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub baseline_shift: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub caps: Caps,
    #[serde(default, skip_serializing_if = "is_default")]
    pub script: Script,
    #[serde(default, skip_serializing_if = "is_default")]
    pub faux_bold: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub faux_italic: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub ligatures: bool,
    /// Off draws the outline alone.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub fill_on: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub stroke_on: bool,
    /// Kept while the outline is off, so turning it back on brings the same
    /// colour and width.
    #[serde(default = "black", skip_serializing_if = "is_black")]
    pub stroke: crate::model::LinearColour,
    /// Px, centred on the letter's edge.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub stroke_width: f64,
    /// The outline is drawn over the fill. Off puts the fill on top, which
    /// hides the inner half of the outline.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub stroke_over: bool,
    /// Every letter's lower half of that pair goes down before any letter's
    /// upper half, After Effects' All fills over all strokes and All strokes
    /// over all fills. Off goes letter by letter, so a wide outline can cover
    /// the letter beside it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub strokes_together: bool,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: String::new(),
            face: String::new(),
            axes: Vec::new(),
            axis_spacing: AxisSpacing::Adaptive,
            leading: None,
            kerning: Kerning::Off,
            tracking: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
            baseline_shift: 0.0,
            caps: Caps::Normal,
            script: Script::Normal,
            faux_bold: false,
            faux_italic: false,
            ligatures: true,
            fill_on: true,
            stroke_on: false,
            stroke: crate::model::LinearColour::BLACK,
            stroke_width: 1.0,
            stroke_over: true,
            strokes_together: false,
            extra: serde_json::Map::new(),
        }
    }
}

impl TextStyle {
    /// True while nothing has been styled, which is when the layer draws the
    /// way it always did.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The style as bytes for the frame key. Only the fields that differ from
    /// the default are in it, the same as the file.
    #[must_use]
    pub fn key_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// How the lines of a Text layer are laid out against each other.
///
/// Each line the user breaks is its own paragraph, as it is for point text in
/// After Effects, so without a box the first line indent reaches every line.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParagraphStyle {
    #[serde(default, skip_serializing_if = "is_default")]
    pub align: TextAlign,
    /// Px the words wrap to. Nought is point text, where a line only ends
    /// where it was broken.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub box_width: f64,
    /// Stretch every line but a paragraph's last to the box, by its spaces.
    /// Does nothing without a box.
    #[serde(default, skip_serializing_if = "is_default")]
    pub justify: bool,
    /// Px in from the left edge of the block.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub indent_left: f64,
    /// Px in from the right edge of the block.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub indent_right: f64,
    /// Px added to the left indent of a paragraph's first line.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub indent_first: f64,
    /// Px of room above every line but the first.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub space_before: f64,
    /// Px of room below every line but the last.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub space_after: f64,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ParagraphStyle {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The same idea as [`TextStyle::key_bytes`].
    #[must_use]
    pub fn key_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// Where a Text layer's words sit in the layer's own pixels, as left, top,
/// right and bottom, for these resolved words.
pub type WordsBox = fn(&crate::model::TextDocument, &str) -> Option<[f64; 4]>;

static WORDS_BOX: std::sync::OnceLock<WordsBox> = std::sync::OnceLock::new();

/// Hand this crate the text engine's measuring. The fonts live in
/// `lumit-text`, which sits above this crate, so whoever links both installs
/// it once before anything is measured. A second call changes nothing.
pub fn install_words_box(measure: WordsBox) {
    let _ = WORDS_BOX.set(measure);
}

/// The middle of each character of a Text layer's words in the layer's own
/// pixels, as they are drawn at layer time `lt`; `None` for one with no place.
pub type GlyphMiddles =
    fn(&crate::model::Layer, &crate::model::TextDocument, &str, f64) -> Vec<Option<[f32; 2]>>;

static GLYPH_MIDDLES: std::sync::OnceLock<GlyphMiddles> = std::sync::OnceLock::new();

/// Hand this crate the text engine's letter places, as [`install_words_box`]
/// does its measuring. Until it is installed a Proximity selector reaches no
/// letter.
pub fn install_glyph_middles(middles: GlyphMiddles) {
    let _ = GLYPH_MIDDLES.set(middles);
}

/// What each character of `layer`'s words `line` is asked to do at comp time
/// `t_comp`. The same as [`glyph_xforms`], with every Proximity selector
/// measured against where its target layer is at that moment.
#[must_use]
pub fn layer_glyph_xforms(
    doc: &std::sync::Arc<crate::model::Document>,
    comp: &crate::model::Composition,
    layer: &crate::model::Layer,
    document: &crate::model::TextDocument,
    line: &str,
    t_comp: f64,
) -> Vec<GlyphXform> {
    let lt = crate::time::layer_time(t_comp, layer.start_offset.0);
    let near = near_of(doc, comp, layer, document, line, t_comp);
    // A number a driver is wired into on the layer's Source box follows the
    // driver instead of its own keys.
    let driven = if layer.graph.edges.iter().any(|e| is_text_port(&e.to)) {
        let context = std::sync::Arc::new(crate::expression::ExpressionContext {
            document: doc.clone(),
            comp: Some(comp.id),
            layer: Some(layer.id),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });
        crate::fx::drivers::driven_source_params(&layer.graph, lt, context, None)
    } else {
        Vec::new()
    };
    if driven.is_empty() {
        return glyph_xforms_near(&document.animators, line, lt, &near);
    }
    let mut animators = document.animators.clone();
    for (port, value) in driven {
        let Some((index, name)) = parse_animator_port(&port) else {
            continue;
        };
        if let Some(p) = animators.get_mut(index).and_then(|a| a.number_mut(name)) {
            *p = Property::fixed(value);
        }
    }
    glyph_xforms_near(&animators, line, lt, &near)
}

/// What a text animator's numbers are called on its layer's Source box:
/// `text.<animator>.<number>`, the animator by its place in the list.
pub const TEXT_PORT_PREFIX: &str = "text.";

/// The Source box port of animator `index`'s number `name`.
#[must_use]
pub fn animator_port(index: usize, name: &str) -> String {
    format!("{TEXT_PORT_PREFIX}{index}.{name}")
}

/// The animator and number a Source box port names.
#[must_use]
pub fn parse_animator_port(port: &str) -> Option<(usize, &str)> {
    let rest = port.strip_prefix(TEXT_PORT_PREFIX)?;
    let (index, name) = rest.split_once('.')?;
    Some((index.parse().ok()?, name))
}

fn is_text_port(to: &crate::graph::InputRef) -> bool {
    matches!(to, crate::graph::InputRef::Param { node: crate::graph::NodeRef::Source, port }
        if port.starts_with(TEXT_PORT_PREFIX))
}

impl TextAnimator {
    /// The keyable numbers a driver can be wired into, by the name its port
    /// carries and as the panels call them: the selector's own, the property
    /// groups the animator lists, and its font axes (`axis.<tag>`).
    #[must_use]
    pub fn wireable_numbers(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut add = |name: &str, label: &str| out.push((name.to_owned(), label.to_owned()));
        match self.selector.kind {
            SelectorKind::Range => {
                add("start", "Range start");
                add("end", "Range end");
                add("offset", "Range offset");
            }
            SelectorKind::Proximity => add("radius", "Radius"),
            SelectorKind::Wiggly => {
                add("max_amount", "Max amount");
                add("min_amount", "Min amount");
                add("wiggles", "Wiggles/second");
                add("correlation", "Correlation");
            }
        }
        let shows = |group: &str| {
            self.properties
                .as_ref()
                .is_none_or(|list| list.iter().any(|p| p == group))
        };
        for (group, numbers) in [
            (
                "position",
                &[("position_x", "Position x"), ("position_y", "Position y")][..],
            ),
            ("rotation", &[("rotation", "Rotation")][..]),
            (
                "scale",
                &[("scale_x", "Scale x"), ("scale_y", "Scale y")][..],
            ),
            ("opacity", &[("opacity", "Opacity")][..]),
            (
                "fill",
                &[
                    ("fill_r", "Fill offset r"),
                    ("fill_g", "Fill offset g"),
                    ("fill_b", "Fill offset b"),
                ][..],
            ),
        ] {
            if shows(group) {
                for (name, label) in numbers {
                    add(name, label);
                }
            }
        }
        for axis in &self.axes {
            out.push((
                format!("axis.{}", axis.tag),
                format!("Font axis {}", axis.tag),
            ));
        }
        out
    }

    /// The keyable number `name` names, the way [`Self::wireable_numbers`]
    /// calls it.
    pub fn number_mut(&mut self, name: &str) -> Option<&mut Property> {
        if let Some(tag) = name.strip_prefix("axis.") {
            return self
                .axes
                .iter_mut()
                .find(|a| a.tag == tag)
                .map(|a| &mut a.value);
        }
        let s = &mut self.selector;
        Some(match name {
            "start" => &mut s.start,
            "end" => &mut s.end,
            "offset" => &mut s.offset,
            "radius" => &mut s.radius,
            "min_amount" => &mut s.min_amount,
            "max_amount" => &mut s.max_amount,
            "wiggles" => &mut s.wiggles,
            "correlation" => &mut s.correlation,
            "position_x" => &mut self.position_x,
            "position_y" => &mut self.position_y,
            "position_z" => &mut self.position_z,
            "rotation" => &mut self.rotation,
            "rotation_x" => &mut self.rotation_x,
            "rotation_y" => &mut self.rotation_y,
            "scale_x" => &mut self.scale_x,
            "scale_y" => &mut self.scale_y,
            "opacity" => &mut self.opacity,
            "fill_r" => &mut self.fill_r,
            "fill_g" => &mut self.fill_g,
            "fill_b" => &mut self.fill_b,
            _ => return None,
        })
    }
}

/// The places [`layer_glyph_xforms`] measures from. Nothing at all while no
/// animator has a Proximity selector, so a layer without one costs no layout.
#[must_use]
pub fn near_of(
    doc: &std::sync::Arc<crate::model::Document>,
    comp: &crate::model::Composition,
    layer: &crate::model::Layer,
    document: &crate::model::TextDocument,
    line: &str,
    t_comp: f64,
) -> Near {
    let mut ids: Vec<uuid::Uuid> = document
        .animators
        .iter()
        .filter(|a| a.selector.kind == SelectorKind::Proximity)
        .filter_map(|a| a.selector.target)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Near::default();
    }
    #[allow(clippy::cast_possible_truncation)] // px
    let targets = ids
        .into_iter()
        .filter(|id| *id != layer.id)
        .filter_map(|id| {
            let target = comp.layers.iter().find(|l| l.id == id)?;
            let (x, y) = crate::expression::space::anchor_in(doc, comp, target, layer, t_comp)?;
            Some((id, [x as f32, y as f32]))
        })
        .collect();
    let lt = crate::time::layer_time(t_comp, layer.start_offset.0);
    Near {
        middles: GLYPH_MIDDLES
            .get()
            .map(|middles| middles(layer, document, line, lt))
            .unwrap_or_default(),
        targets,
    }
}

/// The box [`install_words_box`] measures, or `None` before it is installed.
#[must_use]
pub fn words_box(document: &crate::model::TextDocument, text: &str) -> Option<[f64; 4]> {
    WORDS_BOX.get()?(document, text)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// An unstyled layer writes nothing, and a styled one writes only what was
    /// changed and reads back the same.
    #[test]
    fn a_style_writes_only_what_was_changed() {
        assert_eq!(serde_json::to_string(&TextStyle::default()).unwrap(), "{}");
        assert_eq!(
            serde_json::to_string(&ParagraphStyle::default()).unwrap(),
            "{}"
        );
        assert_eq!(
            serde_json::from_str::<TextStyle>("{}").unwrap(),
            TextStyle::default()
        );

        let style = TextStyle {
            family: "Arial".into(),
            face: "Bold".into(),
            leading: Some(90.0),
            kerning: Kerning::Metrics,
            tracking: 50.0,
            scale_x: 87.0,
            stroke_on: true,
            stroke_width: 4.0,
            ligatures: false,
            ..TextStyle::default()
        };
        let json = serde_json::to_string(&style).unwrap();
        assert!(!json.contains("scale_y"), "{json}");
        assert!(!json.contains("caps"), "{json}");
        assert_eq!(serde_json::from_str::<TextStyle>(&json).unwrap(), style);
        assert_ne!(style.key_bytes(), TextStyle::default().key_bytes());

        let paragraph = ParagraphStyle {
            align: TextAlign::Centre,
            space_after: 12.0,
            ..ParagraphStyle::default()
        };
        let json = serde_json::to_string(&paragraph).unwrap();
        assert!(!json.contains("indent"), "{json}");
        assert_eq!(
            serde_json::from_str::<ParagraphStyle>(&json).unwrap(),
            paragraph
        );
    }

    fn selector(start: f64, end: f64, offset: f64) -> RangeSelector {
        RangeSelector {
            start: Property::fixed(start),
            end: Property::fixed(end),
            offset: Property::fixed(offset),
            ..RangeSelector::default()
        }
    }

    /// Counting words, the letters of one word move **together** — which is the
    /// whole difference between the two bases and the reason both exist.
    #[test]
    fn a_word_selector_moves_a_whole_word_at_once() {
        let mut a = TextAnimator::new("Word");
        a.selector = selector(0.0, 50.0, 0.0);
        a.selector.basis = SelectorBasis::Words;
        a.position_y = Property::fixed(-40.0);
        let x = glyph_xforms(&[a], "ab cd", 0.0);
        assert_eq!(x.len(), 5);
        // "ab " is the first of two words, and every one of its characters —
        // the space included — is pushed the same way.
        for c in &x[0..3] {
            assert!((c.position[1] + 40.0).abs() < 1e-4, "{c:?}");
        }
        for c in &x[3..5] {
            assert_eq!(c.position[1], 0.0, "the second word moved");
        }
    }

    /// **Two animators compose.** A fade and a drop written separately have to
    /// arrive together on the letters both of them reach — pushes add, scales
    /// and opacities multiply.
    #[test]
    fn two_animators_compose_rather_than_replace() {
        let mut drop = TextAnimator::new("Drop");
        drop.position_y = Property::fixed(-30.0);
        drop.scale_x = Property::fixed(50.0);
        drop.opacity = Property::fixed(50.0);
        let mut fade = TextAnimator::new("Fade");
        fade.position_y = Property::fixed(-10.0);
        fade.scale_x = Property::fixed(50.0);
        fade.opacity = Property::fixed(50.0);
        let x = glyph_xforms(&[drop, fade], "A", 0.0);
        assert!((x[0].position[1] + 40.0).abs() < 1e-4, "{x:?}");
        assert!((x[0].scale[0] - 0.25).abs() < 1e-6, "{x:?}");
        assert!((x[0].opacity - 0.25).abs() < 1e-6, "{x:?}");
    }

    /// A weight of a half moves a letter half as far — the property is applied
    /// *times* the weight, which is what makes a ramp read as a sweep.
    #[test]
    fn a_half_weighted_letter_is_moved_half_as_far() {
        let mut a = TextAnimator::new("Half");
        a.selector = RangeSelector {
            shape: SelectorShape::Ramp,
            ..selector(0.0, 100.0, 0.0)
        };
        a.position_x = Property::fixed(100.0);
        a.scale_x = Property::fixed(200.0);
        let x = glyph_xforms(&[a], "ab", 0.0);
        // Two letters: their middles sit at 25 % and 75 % of the run.
        assert!((x[0].position[0] - 25.0).abs() < 1e-4, "{x:?}");
        assert!((x[1].position[0] - 75.0).abs() < 1e-4, "{x:?}");
        assert!((x[0].scale[0] - 1.25).abs() < 1e-6, "{x:?}");
    }

    fn proximity(radius: f64, shape: SelectorShape, target: uuid::Uuid) -> TextAnimator {
        TextAnimator {
            selector: RangeSelector {
                kind: SelectorKind::Proximity,
                target: Some(target),
                radius: Property::fixed(radius),
                shape,
                ..RangeSelector::default()
            },
            position_y: Property::fixed(-10.0),
            ..TextAnimator::new("Near")
        }
    }

    /// **A Proximity selector moves the letters near its target**: all the
    /// way on it, none from the radius out, and the shape in between.
    #[test]
    fn a_proximity_selector_reaches_the_letters_near_its_target() {
        let target = uuid::Uuid::from_u128(7);
        let near = Near {
            middles: vec![
                Some([0.0, 0.0]),
                Some([50.0, 0.0]),
                Some([100.0, 0.0]),
                None,
            ],
            targets: vec![(target, [0.0, 0.0])],
        };
        let ramp = glyph_xforms_near(
            &[proximity(100.0, SelectorShape::Ramp, target)],
            "abc\n",
            0.0,
            &near,
        );
        let pushes: Vec<f32> = ramp.iter().map(|x| x.position[1]).collect();
        assert_eq!(pushes, vec![-10.0, -5.0, 0.0, 0.0]);

        let square = glyph_xforms_near(
            &[proximity(60.0, SelectorShape::Square, target)],
            "abc\n",
            0.0,
            &near,
        );
        let pushes: Vec<f32> = square.iter().map(|x| x.position[1]).collect();
        assert_eq!(pushes, vec![-10.0, -10.0, 0.0, 0.0]);

        // Smooth eases the slope: half way is still half, a quarter in is less.
        let smooth = near_weight(SelectorShape::Smooth, 25.0, 100.0);
        assert!((near_weight(SelectorShape::Smooth, 50.0, 100.0) - 0.5).abs() < 1e-6);
        assert!(smooth > 0.75 && smooth < 1.0, "{smooth}");

        // Inverted, the far letters move and the near ones stay.
        let mut inverted = proximity(100.0, SelectorShape::Ramp, target);
        inverted.selector.invert = true;
        let x = glyph_xforms_near(&[inverted], "abc", 0.0, &near);
        assert_eq!(x[0].position[1], 0.0);
        assert_eq!(x[2].position[1], -10.0);
    }

    /// A Proximity selector whose target isn't there reaches nothing, the same
    /// as an empty range.
    #[test]
    fn a_proximity_selector_without_its_target_reaches_nothing() {
        let animator = proximity(100.0, SelectorShape::Ramp, uuid::Uuid::from_u128(1));
        let x = glyph_xforms(&[animator], "ab", 0.0);
        assert!(x.iter().all(GlyphXform::is_identity), "{x:?}");
    }

    /// **A font axis is a place, mixed by the weight**: a letter half reached
    /// is set half way from the layer's place to the animator's, and a second
    /// animator carries on from where the first left it.
    #[test]
    fn a_font_axis_is_set_half_way_by_half_a_weight() {
        let mut a = TextAnimator::new("Weight");
        a.selector.shape = SelectorShape::Ramp;
        a.axes.push(AnimatorAxis {
            tag: "wght".into(),
            value: Property::fixed(900.0),
        });
        // Two letters: the ramp gives the first a quarter, the second three.
        let x = glyph_xforms(&[a.clone()], "ab", 0.0);
        let base = |_: [u8; 4]| 100.0f32;
        let at = |x: &GlyphXform| x.axes.resolved(&base).next().unwrap();
        assert_eq!(at(&x[0]).0, *b"wght");
        assert!((at(&x[0]).1 - 300.0).abs() < 1e-3, "{:?}", at(&x[0]));
        assert!((at(&x[1]).1 - 700.0).abs() < 1e-3, "{:?}", at(&x[1]));

        // A second animator taking the whole word to 500 wins over the first.
        let mut b = TextAnimator::new("Back");
        b.axes.push(AnimatorAxis {
            tag: "wght".into(),
            value: Property::fixed(500.0),
        });
        let x = glyph_xforms(&[a, b], "ab", 0.0);
        assert!((at(&x[0]).1 - 500.0).abs() < 1e-3);
        assert_eq!(x[0].axes.len, 1);
    }

    /// No more than eight axes, and a tag that isn't one is left out.
    #[test]
    fn a_layer_moves_at_most_eight_axes() {
        let mut a = TextAnimator::new("Many");
        for tag in [
            "AAAA", "BBBB", "CCCC", "DDDD", "EEEE", "FFFF", "GGGG", "HHHH", "IIII", "toolong",
        ] {
            a.axes.push(AnimatorAxis {
                tag: tag.into(),
                value: Property::fixed(1.0),
            });
        }
        let x = glyph_xforms(&[a], "a", 0.0);
        assert_eq!(usize::from(x[0].axes.len), MAX_ANIMATED_AXES);
        assert_eq!(axis_tag("wdth"), Some(*b"wdth"));
        assert_eq!(axis_tag("ab"), Some(*b"ab  "));
        assert_eq!(axis_tag(""), None);
    }

    /// The new settings write nothing while they hold their defaults, and
    /// read back what was set.
    #[test]
    fn axes_and_proximity_write_only_what_was_set() {
        let plain = TextAnimator::new("A");
        let json = serde_json::to_value(&plain).unwrap();
        assert!(
            json.get("axes").is_none() && json.get("selector").is_none(),
            "{json}"
        );
        let mut set = proximity(150.0, SelectorShape::Smooth, uuid::Uuid::from_u128(9));
        set.axes.push(AnimatorAxis {
            tag: "wdth".into(),
            value: Property::fixed(75.0),
        });
        let back: TextAnimator =
            serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
        assert_eq!(back, set);

        let style = TextStyle {
            axes: vec![FontAxisValue {
                tag: "wght".into(),
                value: 650.0,
            }],
            axis_spacing: AxisSpacing::PerCharacter,
            ..TextStyle::default()
        };
        let back: TextStyle =
            serde_json::from_str(&serde_json::to_string(&style).unwrap()).unwrap();
        assert_eq!(back, style);
    }

    /// **A Wiggly selector wanders**: every weight stays between the two
    /// amounts, changes over time, and at full correlation the letters move
    /// as one.
    #[test]
    fn a_wiggly_selector_wanders_between_its_amounts() {
        let mut s = RangeSelector {
            kind: SelectorKind::Wiggly,
            min_amount: Property::fixed(20.0),
            max_amount: Property::fixed(80.0),
            correlation: Property::fixed(0.0),
            ..RangeSelector::default()
        };
        let a = s.wiggly_weights(6, 0.3);
        let b = s.wiggly_weights(6, 1.7);
        assert!(
            a.iter().chain(&b).all(|w| (0.2..=0.8).contains(w)),
            "{a:?} {b:?}"
        );
        assert_ne!(a, b, "the weights move over time");
        assert!(
            a.windows(2).any(|p| (p[0] - p[1]).abs() > 1e-3),
            "letters apart: {a:?}"
        );
        s.correlation = Property::fixed(100.0);
        let together = s.wiggly_weights(6, 0.3);
        assert!(
            together.windows(2).all(|p| (p[0] - p[1]).abs() < 1e-6),
            "{together:?}"
        );
        // The same settings wander the same way every time.
        assert_eq!(s.wiggly_weights(6, 0.3), together);
    }

    /// An animator's list of property groups is written only once there is
    /// one, and reads back.
    #[test]
    fn an_animators_property_list_reads_back() {
        let mut a = TextAnimator::new("A");
        a.properties = Some(vec!["scale".into(), "position".into()]);
        let back: TextAnimator = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back.properties, a.properties);
        let plain = serde_json::to_value(TextAnimator::new("B")).unwrap();
        assert!(plain.get("properties").is_none());
    }
}
