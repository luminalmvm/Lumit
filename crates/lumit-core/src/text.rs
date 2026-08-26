//! Text animators: a Text layer's words moved a letter at a time (K-609).
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectorShape {
    #[default]
    Square,
    Ramp,
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
        }
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
}

/// One unit's weight for a range `lo`–`hi` at position `p`, all in 0–1.
#[must_use]
fn weight(shape: SelectorShape, lo: f64, hi: f64, p: f64) -> f32 {
    let w = match shape {
        SelectorShape::Square => f64::from(u8::from(p >= lo && p < hi)),
        // A zero-width ramp is a step, which is the honest limit of the ramp
        // rather than a division by nothing.
        SelectorShape::Ramp if hi <= lo => f64::from(u8::from(p >= hi)),
        SelectorShape::Ramp => ((p - lo) / (hi - lo)).clamp(0.0, 1.0),
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
    /// Unknown fields from newer Lumit versions (docs/10 §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
            scale_x: hundred(),
            scale_y: hundred(),
            opacity: hundred(),
            fill_r: Property::zero(),
            fill_g: Property::zero(),
            fill_b: Property::zero(),
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
}

impl Default for GlyphXform {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            rotation: 0.0,
            scale: [1.0, 1.0],
            opacity: 1.0,
            fill: [0.0, 0.0, 0.0],
        }
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
/// Empty when there are no animators, which is the whole of the K-258
/// byte-identity guarantee: the caller draws the line the way it always drew it
/// rather than taking a second code path that happens to agree.
///
/// Two animators reaching the same letter **compose**: their pushes, turns and
/// tints add, their scales and opacities multiply. That is the only combination
/// that reads as "and also" rather than "instead of", and it is what lets a
/// fade animator and a drop animator be written separately.
#[must_use]
pub fn glyph_xforms(animators: &[TextAnimator], text: &str, lt: f64) -> Vec<GlyphXform> {
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
        let weights = animator.selector.weights_at(total, lt);
        if weights.is_empty() {
            continue;
        }
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
            // A scale of 100 % has to leave the letter alone at every weight,
            // so the weight interpolates from 1 rather than from 0.
            x.scale[0] *= 1.0 + w * (sx - 1.0);
            x.scale[1] *= 1.0 + w * (sy - 1.0);
            x.opacity *= 1.0 + w * (opacity - 1.0);
            x.fill[0] += w * fr;
            x.fill[1] += w * fg;
            x.fill[2] += w * fb;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn selector(start: f64, end: f64, offset: f64) -> RangeSelector {
        RangeSelector {
            start: Property::fixed(start),
            end: Property::fixed(end),
            offset: Property::fixed(offset),
            ..RangeSelector::default()
        }
    }

    /// **A square selector is in or out.** Half the run selected means half the
    /// letters moved the whole way and the rest not moved at all — the case
    /// every cascade is built out of, asserted letter by letter.
    #[test]
    fn a_square_selector_moves_the_letters_inside_it_and_no_others() {
        let s = selector(0.0, 50.0, 0.0);
        assert_eq!(s.weights_at(4, 0.0), vec![1.0, 1.0, 0.0, 0.0]);
        // And sliding it by the offset walks the selection along.
        let slid = selector(0.0, 50.0, 50.0);
        assert_eq!(slid.weights_at(4, 0.0), vec![0.0, 0.0, 1.0, 1.0]);
    }

    /// **A ramp rises across the range and stays up past it.** The first letter
    /// is barely touched, the last is moved the whole way, and letters past the
    /// end keep the whole way rather than snapping back.
    #[test]
    fn a_ramp_rises_across_the_range_and_holds_past_it() {
        let s = RangeSelector {
            shape: SelectorShape::Ramp,
            ..selector(0.0, 50.0, 0.0)
        };
        let w = s.weights_at(4, 0.0);
        assert!((w[0] - 0.25).abs() < 1e-6, "{w:?}");
        assert!((w[1] - 0.75).abs() < 1e-6, "{w:?}");
        assert_eq!((w[2], w[3]), (1.0, 1.0), "the ramp fell back down");
        // Rising, and never falling.
        assert!(w.windows(2).all(|p| p[1] >= p[0]));
    }

    /// A range with no width is a step rather than a division by nothing.
    #[test]
    fn a_zero_width_ramp_is_a_step() {
        let s = RangeSelector {
            shape: SelectorShape::Ramp,
            ..selector(50.0, 50.0, 0.0)
        };
        assert_eq!(s.weights_at(4, 0.0), vec![0.0, 0.0, 1.0, 1.0]);
        // A square one selects nothing at all, which is the honest reading of
        // a range between a point and itself.
        assert_eq!(selector(50.0, 50.0, 0.0).weights_at(4, 0.0), vec![0.0; 4]);
    }

    /// A range dragged inside out still means the stretch between its ends.
    #[test]
    fn an_inside_out_range_still_names_the_stretch_between_its_ends() {
        assert_eq!(
            selector(75.0, 25.0, 0.0).weights_at(4, 0.0),
            selector(25.0, 75.0, 0.0).weights_at(4, 0.0)
        );
    }

    /// **Words are counted whole, and the gaps go with the word before them.**
    /// Otherwise a range sweeping a sentence would pause on each space as if it
    /// were a word of its own.
    #[test]
    fn words_are_counted_whole_and_the_spaces_go_with_them() {
        let (units, total) = unit_indices("ab cd", SelectorBasis::Words);
        assert_eq!(total, 2);
        assert_eq!(units, vec![0, 0, 0, 1, 1], "the space left its word");
        // Runs of spaces, and leading spaces, behave the same way.
        let (units, total) = unit_indices("  a   b ", SelectorBasis::Words);
        assert_eq!(total, 2);
        assert_eq!(units, vec![0, 0, 0, 0, 0, 0, 1, 1]);
        // Counting characters, every character is its own unit.
        assert_eq!(
            unit_indices("ab cd", SelectorBasis::Characters),
            (vec![0, 1, 2, 3, 4], 5)
        );
        // Nothing to count is not an error.
        assert_eq!(unit_indices("", SelectorBasis::Words), (Vec::new(), 0));
        assert_eq!(
            unit_indices("   ", SelectorBasis::Words),
            (vec![0, 0, 0], 0)
        );
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

    /// **No animators, no transforms** — the promise the byte-identity gate
    /// rests on: the caller is handed nothing to apply, not a list of
    /// identities it has to notice are identities.
    #[test]
    fn a_layer_with_no_animators_asks_for_nothing() {
        assert!(glyph_xforms(&[], "Lumit", 0.0).is_empty());
        // And an animator on an empty line has nothing to move.
        assert!(glyph_xforms(&[TextAnimator::new("A")], "", 0.0).is_empty());
        // A fresh animator changes nothing until a number is moved.
        let fresh = glyph_xforms(&[TextAnimator::new("A")], "Lu", 0.0);
        assert!(fresh.iter().all(GlyphXform::is_identity), "{fresh:?}");
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

    /// A fresh animator writes almost nothing: every default is left out of the
    /// file, so an animator nobody has touched costs its name and no more.
    #[test]
    fn a_default_animator_writes_only_its_name() {
        let json = serde_json::to_string(&TextAnimator::new("Animator 1")).unwrap();
        assert_eq!(json, r#"{"name":"Animator 1"}"#, "{json}");
        let back: TextAnimator = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TextAnimator::new("Animator 1"));
    }
}
