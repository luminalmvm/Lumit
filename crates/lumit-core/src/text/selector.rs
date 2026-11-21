//! Selectors: which letters an animator moves, and by how much.

use serde::{Deserialize, Serialize};

use super::{hundred, is_default, is_static_hundred, is_static_zero, is_zero};
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
    /// Every letter but the spaces, as After Effects' Characters Excluding
    /// Spaces: a space is not counted, so the cascade does not pause on it,
    /// and it goes with the letter before it.
    CharactersExcludingSpaces,
    /// Whole lines, broken where the words were broken by hand.
    Lines,
}

/// How a selector after the first is put together with the ones above it,
/// After Effects' Mode. The first selector's Add is its own weight and its
/// Subtract is that turned over; Subtract is stored as
/// [`RangeSelector::invert`], so a file from before there were more modes
/// reads the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorCombine {
    /// The weights add, held to the whole way.
    #[default]
    Add,
    /// Each letter moved as far as both selectors agree on: one times the
    /// other.
    Intersect,
    /// The smaller of the two.
    Min,
    /// The larger of the two.
    Max,
    /// How far apart the two are.
    Difference,
}

/// What a Range selector's Start, End and Offset count in, After Effects'
/// Units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorUnits {
    /// Per cent of the run.
    #[default]
    Percentage,
    /// Units: from 0 to 2 is the first two letters (or words, or lines).
    Index,
}

/// How a Stagger selector plays Amount's keys once it reaches the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StaggerRepeat {
    /// Once, holding the last key.
    #[default]
    Once,
    /// From the first key again.
    Loop,
    /// Back to the first key, then forwards again.
    PingPong,
}

/// How the weight falls off across the range.
///
/// `Square` is in-or-out: everything inside the range is moved the whole way,
/// everything outside is left alone. `Ramp` rises evenly from nothing at the
/// range's start to the whole way at its end, and stays there afterwards —
/// which is what turns a cascade into a sweep.
///
/// `Smooth` is the ramp eased in and out, so the letters at either end of the
/// slope barely move and the ones in the middle of it move most. `Bounce`
/// overshoots by about a tenth, then settles. Following spoken words, the
/// shape is how each word changes over the selector's ease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorShape {
    #[default]
    Square,
    Ramp,
    Smooth,
    Bounce,
}

/// What the selector follows: its own range, or the words as they are said.
///
/// The spoken modes need the words' times, which only a caption hands in.
/// Without them they reach no letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorMode {
    #[default]
    Range,
    /// The word being said.
    Current,
    /// Every word said so far.
    Said,
    /// The words still to come.
    Unsaid,
    /// The words picked out by hand.
    Emphasis,
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
    /// **Every unit plays the same animation, one after another**, as
    /// Cavalry's Stagger: each unit's weight is `amount` read `delay` seconds
    /// later than the unit before it, in `order`. The curve drawn on Amount in
    /// the graph editor is the curve every word or letter moves on.
    Stagger,
}

/// The order a Stagger selector starts its units in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StaggerOrder {
    /// First to last.
    #[default]
    Forward,
    /// Last to first.
    Backward,
    /// The middle first, out to both ends.
    FromCentre,
    /// Both ends first, in to the middle.
    FromEdges,
    /// An order of its own for each Seed.
    Random,
}

fn three() -> Property {
    Property::fixed(3.0)
}

fn is_static_three(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 3.0) && p.extra.is_empty()
}

fn is_default_order(o: &StaggerOrder) -> bool {
    *o == StaggerOrder::Forward
}

/// serde default for [`RangeSelector::delay`]: a tenth of a second.
fn tenth() -> Property {
    Property::fixed(0.1)
}

fn is_static_tenth(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 0.1) && p.extra.is_empty()
}

fn is_static_hundred_amount(p: &Property) -> bool {
    matches!(p.animation, crate::anim::Animation::Static(v) if v == 100.0) && p.extra.is_empty()
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
    #[serde(default, skip_serializing_if = "is_default")]
    pub mode: SelectorMode,
    /// Seconds a spoken word takes to change.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ease: f64,
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
    /// the near ones are left alone. On a Range selector this is After
    /// Effects' Mode **Subtract**: the letters the range reaches are left
    /// alone and the others are moved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
    /// Per cent, −100 to 100: how a Range selector's ramp eases into the
    /// whole way (High) and out of nothing (Low), as After Effects' Ease High
    /// and Ease Low, bending it as a keyframe's ease handles bend a curve in
    /// the graph editor: 0 is the even ramp, 100 lands that end as softly as a
    /// full easy ease, −100 stands the handle upright.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub ease_high: Property,
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub ease_low: Property,
    /// A Stagger selector's animation, per cent: what every unit plays, keyed
    /// and curved as the user draws it.
    #[serde(
        default = "hundred_wiggly",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_hundred_amount"
    )]
    pub amount: Property,
    /// Seconds between one unit's start and the next's.
    #[serde(
        default = "tenth",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_tenth"
    )]
    pub delay: Property,
    /// Per cent, −100 to 100: how the delays are shared out. 0 keeps them
    /// even; above 0 the first units start close together and the later ones
    /// further apart; below 0 the other way round.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub delay_ease: Property,
    /// The order the units start in. Random uses `seed`.
    #[serde(default, skip_serializing_if = "is_default_order")]
    pub order: StaggerOrder,
    /// A Stagger selector's **bounce**, the inertial bounce of After Effects'
    /// well-known expression, on every unit: after each of Amount's keys the
    /// unit swings on past it by the speed it arrived with, times
    /// `bounce_amplitude`, `bounce_frequency` times a second, dying away by
    /// `bounce_decay`. An amplitude of 0 is no bounce.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub bounce_amplitude: Property,
    #[serde(
        default = "two",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_two"
    )]
    pub bounce_frequency: Property,
    #[serde(
        default = "three",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_static_three"
    )]
    pub bounce_decay: Property,
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
    /// How this selector is put together with the ones above it; see
    /// [`SelectorCombine`].
    #[serde(default, skip_serializing_if = "is_default")]
    pub combine: SelectorCombine,
    /// What a Range selector's Start, End and Offset count in.
    #[serde(default, skip_serializing_if = "is_default")]
    pub units: SelectorUnits,
    /// A Range selector's Randomize order: the units are reached in an order
    /// of their own for each Seed, rather than first to last.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub randomize: bool,
    /// Show a Stagger's Delay in frames rather than seconds. Only the panels
    /// read it: Delay is kept in seconds either way.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub delay_frames: bool,
    /// A Stagger's **Total duration**: Delay is the time from the first unit's
    /// start to the last one's, however many units there are, rather than the
    /// time between two.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub total: bool,
    /// What a Stagger does after Amount's last key: hold, loop or ping-pong.
    #[serde(default, skip_serializing_if = "is_default")]
    pub repeat: StaggerRepeat,
    /// Bounce only after Amount's last key, rather than after every key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bounce_last: bool,
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
            mode: SelectorMode::default(),
            ease: 0.0,
            kind: SelectorKind::default(),
            target: None,
            radius: two_hundred(),
            invert: false,
            ease_high: Property::zero(),
            ease_low: Property::zero(),
            amount: hundred_wiggly(),
            delay: tenth(),
            delay_ease: Property::zero(),
            order: StaggerOrder::default(),
            bounce_amplitude: Property::zero(),
            bounce_frequency: two(),
            bounce_decay: three(),
            min_amount: Property::zero(),
            max_amount: hundred_wiggly(),
            wiggles: two(),
            correlation: fifty(),
            seed: 0,
            combine: SelectorCombine::Add,
            units: SelectorUnits::Percentage,
            randomize: false,
            delay_frames: false,
            total: false,
            repeat: StaggerRepeat::Once,
            bounce_last: false,
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
    /// The weight a Stagger selector gives each of `units` at layer time `lt`:
    /// Amount, per cent, read as many delays earlier as the unit is from the
    /// first to start. Amount is not held to 0–100, so a curve that overshoots
    /// in the graph editor overshoots on every unit. Mode Subtract turns the
    /// weights over, as on a Range selector.
    #[must_use]
    pub fn stagger_weights(&self, units: usize, lt: f64) -> Vec<f32> {
        if units == 0 {
            return Vec::new();
        }
        let delay = self.delay.value_at(lt);
        let delay = if delay.is_finite() { delay } else { 0.0 };
        let ease = (self.delay_ease.value_at(lt) / 100.0).clamp(-1.0, 1.0);
        let ease = if ease.is_finite() { ease } else { 0.0 };
        let ranks = self.stagger_ranks(units);
        #[allow(clippy::cast_precision_loss)] // a line is short
        let last = ranks.iter().copied().max().unwrap_or(0) as f64;
        // Total duration: Delay is the whole spread, shared between the gaps.
        let delay = match (self.total, last > 0.0) {
            (false, _) => delay,
            (true, true) => delay / last,
            (true, false) => 0.0,
        };
        ranks
            .iter()
            .map(|&rank| {
                // Where the unit comes in the order, 0–1, bent by Delay ease:
                // a power above 1 keeps the first starts together, below 1
                // the last ones.
                #[allow(clippy::cast_precision_loss)] // a line is short
                let s = if last > 0.0 { rank as f64 / last } else { 0.0 };
                let s = if ease >= 0.0 {
                    s.powf(1.0 + 3.0 * ease)
                } else {
                    1.0 - (1.0 - s).powf(1.0 - 3.0 * ease)
                };
                let at = self.repeated(lt - s * last * delay);
                let w = (self.amount.value_at(at) + self.bounce_at(at, lt)) / 100.0;
                let w = if w.is_finite() { w } else { 0.0 };
                #[allow(clippy::cast_possible_truncation)] // a weight
                let w = if self.invert { 1.0 - w } else { w } as f32;
                w
            })
            .collect()
    }

    /// The bounce on Amount at `at`, per cent, as the inertial-bounce
    /// expression reads it: from the last of Amount's keys at or before `at`,
    /// the speed Amount arrived at it with, times the amplitude, swung on a
    /// sine of the frequency and dying away by the decay. Nothing before the
    /// first key, nothing at an amplitude of 0, nothing on an Amount that is
    /// not keyed. The three settings are read at the layer's own time `lt`.
    /// `at` folded back into Amount's keys by [`Self::repeat`]: after the
    /// last key a Loop starts again from the first, a Ping-pong plays back to
    /// it. Before the last key, or on an Amount with fewer than two keys, `at`
    /// as it is.
    #[must_use]
    pub fn repeated(&self, at: f64) -> f64 {
        if self.repeat == StaggerRepeat::Once {
            return at;
        }
        let crate::anim::Animation::Keyframed(keys) = &self.amount.animation else {
            return at;
        };
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
            return at;
        };
        let (k0, k1) = (first.time.to_f64(), last.time.to_f64());
        let len = k1 - k0;
        if at <= k1 || len <= 0.0 || !at.is_finite() {
            return at;
        }
        let u = at - k0;
        let lap = (u / len).floor();
        let r = u - lap * len;
        match self.repeat {
            // An odd lap of a Ping-pong runs backwards.
            StaggerRepeat::PingPong if lap.rem_euclid(2.0) >= 1.0 => k1 - r,
            _ => k0 + r,
        }
    }

    fn bounce_at(&self, at: f64, lt: f64) -> f64 {
        let amp = self.bounce_amplitude.value_at(lt);
        if amp == 0.0 || !amp.is_finite() {
            return 0.0;
        }
        let crate::anim::Animation::Keyframed(keys) = &self.amount.animation else {
            return 0.0;
        };
        let Some(at_key) = keys.iter().rposition(|k| k.time.to_f64() <= at) else {
            return 0.0;
        };
        if self.bounce_last && at_key + 1 != keys.len() {
            return 0.0;
        }
        let key = &keys[at_key];
        let k = key.time.to_f64();
        let t = at - k;
        // The speed just before the key, per cent a second, as the
        // expression's velocityAtTime a tenth of a frame before it.
        let h = 1.0 / 250.0;
        let v = (self.amount.value_at(k - h) - self.amount.value_at(k - 2.0 * h)) / h;
        let freq = self.bounce_frequency.value_at(lt);
        let decay = self.bounce_decay.value_at(lt);
        let b = v / 100.0 * amp * (freq * t * std::f64::consts::TAU).sin() / (decay * t).exp();
        if b.is_finite() {
            b
        } else {
            0.0
        }
    }

    /// Each unit's place in a Stagger selector's order, 0 for the first to
    /// start. Units the same distance from the middle (or the edges) start
    /// together.
    fn stagger_ranks(&self, units: usize) -> Vec<usize> {
        #[allow(clippy::cast_precision_loss)] // a line is short
        let mid = (units as f64 - 1.0) * 0.5;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let from_mid = |i: usize| ((i as f64 - mid).abs() + 0.25).floor() as usize;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let half = (mid + 0.25).floor() as usize;
        match self.order {
            StaggerOrder::Forward => (0..units).collect(),
            StaggerOrder::Backward => (0..units).map(|i| units - 1 - i).collect(),
            StaggerOrder::FromCentre => (0..units).map(from_mid).collect(),
            StaggerOrder::FromEdges => (0..units).map(|i| half - from_mid(i).min(half)).collect(),
            StaggerOrder::Random => random_ranks(self.seed, units),
        }
    }
}

/// An order of `units` of their own for `seed`: each unit's place in it.
fn random_ranks(seed: u32, units: usize) -> Vec<usize> {
    let mut by: Vec<(f64, usize)> = (0..units)
        .map(|i| {
            #[allow(clippy::cast_possible_truncation)] // a line is short
            let channel = 7 + i as u32;
            (crate::fx::maths::value_noise_1d(seed, channel, 0.0), i)
        })
        .collect();
    by.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut ranks = vec![0; units];
    for (rank, (_, i)) in by.into_iter().enumerate() {
        ranks[i] = rank;
    }
    ranks
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
        // Index counts units, so the whole run is `units` of them.
        #[allow(clippy::cast_precision_loss)] // a line is short
        let whole = match self.units {
            SelectorUnits::Percentage => 100.0,
            SelectorUnits::Index => units as f64,
        };
        let (mut lo, mut hi) = (
            (self.start.value_at(lt) + offset) / whole,
            (self.end.value_at(lt) + offset) / whole,
        );
        // A range dragged inside out still means the stretch between its ends.
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let shape = self.shape;
        let ease = Ease {
            high: (self.ease_high.value_at(lt) / 100.0).clamp(-1.0, 1.0),
            low: (self.ease_low.value_at(lt) / 100.0).clamp(-1.0, 1.0),
        };
        // Randomize order: each unit stands in the run where the seed puts it.
        let places = self.randomize.then(|| random_ranks(self.seed, units));
        (0..units)
            .map(|i| {
                let i = places.as_ref().map_or(i, |p| p[i]);
                // The middle of the unit's own share of the run, so the first
                // and last units are treated the same way as the ones between
                // them — a selector at 0 % has not reached the first letter's
                // middle yet, and one at 100 % has passed the last letter's.
                #[allow(clippy::cast_precision_loss)] // a run of 2^24 letters is not a line
                let p = (i as f64 + 0.5) / units as f64;
                let w = weight(shape, lo, hi, p, ease);
                // Subtract: the letters the range reaches are the ones left
                // alone.
                if self.invert {
                    1.0 - w
                } else {
                    w
                }
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
pub(super) fn near_weight(shape: SelectorShape, d: f32, radius: f32) -> f32 {
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
        SelectorShape::Bounce => {
            #[allow(clippy::cast_possible_truncation)] // 0 to about 1.1
            let b = super::spoken::curve(shape, f64::from(u)) as f32;
            b
        }
    }
}

/// One unit's weight for a range `lo`–`hi` at position `p`, all in 0–1.
#[must_use]
/// Ease High and Ease Low as fractions, −1 to 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Ease {
    high: f64,
    low: f64,
}

impl Ease {
    /// `t` (0–1 across the slope) bent by the two eases, as a keyframe's ease
    /// handles bend a curve in the graph editor: a cubic Bézier from (0, 0) to
    /// (1, 1) whose two handles are the eases' **influence**. A positive ease
    /// lays its handle flat, as far along as the per cent says, so 100 % lands
    /// as softly as a full easy ease; a negative one stands it upright, so the
    /// curve leaves or reaches that end steeply. 0 % at both ends is the even
    /// ramp.
    fn bend(self, t: f64) -> f64 {
        if self.high == 0.0 && self.low == 0.0 {
            return t;
        }
        let t = t.clamp(0.0, 1.0);
        // The handle out of 0 and the handle into 1.
        let p1 = if self.low >= 0.0 {
            [self.low, 0.0]
        } else {
            [0.0, -self.low]
        };
        let p2 = if self.high >= 0.0 {
            [1.0 - self.high, 1.0]
        } else {
            [1.0, 1.0 + self.high]
        };
        let bezier = |s: f64, a: f64, b: f64| {
            let r = 1.0 - s;
            3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
        };
        // x(s) rises from 0 to 1 whatever the handles, since both handles'
        // x lie in 0–1, so halving finds the s whose x is t.
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if bezier(mid, p1[0], p2[0]) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        bezier(0.5 * (lo + hi), p1[1], p2[1]).clamp(0.0, 1.0)
    }
}

fn weight(shape: SelectorShape, lo: f64, hi: f64, p: f64, ease: Ease) -> f32 {
    let w = match shape {
        SelectorShape::Square => f64::from(u8::from(p >= lo && p < hi)),
        // A zero-width ramp is a step, which is the honest limit of the ramp
        // rather than a division by nothing.
        _ if hi <= lo => f64::from(u8::from(p >= hi)),
        _ => super::spoken::curve(shape, ease.bend((p - lo) / (hi - lo))),
    };
    #[allow(clippy::cast_possible_truncation)] // 0–1
    let w = w as f32;
    w
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
    pub(super) fn target(&self, id: Option<uuid::Uuid>) -> Option<[f32; 2]> {
        let id = id?;
        self.targets.iter().find(|(t, _)| *t == id).map(|(_, p)| *p)
    }
}

/// The places [`super::ports::layer_glyph_xforms`] measures from. Nothing at
/// all while no animator has a Proximity selector, so a layer without one
/// costs no layout.
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
        .flat_map(super::TextAnimator::all_selectors)
        .filter(|s| s.kind == SelectorKind::Proximity)
        .filter_map(|s| s.target)
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
        middles: super::GLYPH_MIDDLES
            .get()
            .map(|middles| middles(layer, document, line, lt))
            .unwrap_or_default(),
        targets,
    }
}
