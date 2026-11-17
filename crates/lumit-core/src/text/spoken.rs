//! The selectors that follow speech: a letter's weight from the words' times.

use super::selector::{RangeSelector, SelectorBasis, SelectorMode, SelectorShape};
use super::SpokenWord;

/// How far a change has got at `t`, 0 to 1 through it, in this shape.
pub(super) fn curve(shape: SelectorShape, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match shape {
        SelectorShape::Square => f64::from(u8::from(t >= 1.0)),
        SelectorShape::Ramp => t,
        SelectorShape::Smooth => t * t * (3.0 - 2.0 * t),
        // Overshoots by about a tenth on the way, then settles at one.
        SelectorShape::Bounce => {
            let (k, u) = (1.701_58, t - 1.0);
            1.0 + (k + 1.0) * u * u * u + k * u * u
        }
    }
}

/// How far a word has changed `since` seconds after its moment. Square, or no
/// ease, is an instant step.
fn change(shape: SelectorShape, ease: f64, since: f64) -> f64 {
    if shape == SelectorShape::Square || !(ease > 0.0 && ease.is_finite()) {
        return f64::from(u8::from(since >= 0.0));
    }
    curve(shape, since / ease)
}

/// Each of `count` characters' weight from the spoken words at layer time
/// `lt`. Nothing when there are no words, so the selector reaches no letter.
pub(super) fn weights(
    selector: &RangeSelector,
    count: usize,
    lt: f64,
    spoken: &[SpokenWord],
) -> Vec<f32> {
    if spoken.is_empty() {
        return Vec::new();
    }
    let mut owner: Vec<Option<&SpokenWord>> = vec![None; count];
    for word in spoken {
        for slot in owner
            .get_mut(word.chars.start.min(count)..word.chars.end.min(count))
            .into_iter()
            .flatten()
        {
            *slot = Some(word);
        }
    }
    let letters = selector.basis == SelectorBasis::Characters;
    if !letters {
        // The characters between two words go with the word before.
        let mut before = None;
        for slot in &mut owner {
            match slot {
                Some(_) => before = *slot,
                None => *slot = before,
            }
        }
    }
    let (mode, shape, ease) = (selector.mode, selector.shape, selector.ease);
    let said = |moment: f64| change(shape, ease, lt - moment);
    owner
        .iter()
        .enumerate()
        .map(|(c, word)| {
            let w = match (word, mode) {
                // A character in no word counts as already said.
                (None, SelectorMode::Said) => 1.0,
                (None, _) | (Some(_), SelectorMode::Range) => 0.0,
                (Some(word), SelectorMode::Said | SelectorMode::Unsaid) => {
                    // Letter by letter, each is said at its share of the word.
                    let moment = if letters {
                        #[allow(clippy::cast_precision_loss)] // a word is far shorter than 2^52
                        let at = c.saturating_sub(word.chars.start) as f64
                            / word.chars.len().max(1) as f64;
                        word.start + at * (word.end - word.start)
                    } else {
                        word.start
                    };
                    if mode == SelectorMode::Said {
                        said(moment)
                    } else {
                        1.0 - said(moment)
                    }
                }
                (Some(word), SelectorMode::Current) => said(word.start) - said(word.end),
                (Some(word), SelectorMode::Emphasis) => f64::from(u8::from(word.emphasis)),
            };
            #[allow(clippy::cast_possible_truncation)] // a weight, an f32 picture
            let w = w as f32;
            w
        })
        .collect()
}
