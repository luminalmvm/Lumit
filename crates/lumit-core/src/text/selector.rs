//! Selectors: which letters an animator moves, and by how much.

use serde::{Deserialize, Serialize};

use super::{hundred, is_static_hundred, is_static_zero};
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
