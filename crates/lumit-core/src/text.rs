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
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: String::new(),
            face: String::new(),
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
/// After Effects, so the first line indent reaches every line.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParagraphStyle {
    #[serde(default, skip_serializing_if = "is_default")]
    pub align: TextAlign,
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
}
