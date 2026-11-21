//! Keyed words: a Text layer whose words change at keyframes.
//!
//! The words are a short list and the keys are an ordinary [`Property`] that
//! indexes it, the shape a File parameter has. A key that holds switches the
//! words at the next key. A key that leaves Linear or eased fades from its
//! word to the next one, and the curve is the fade's timing.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::anim::{evaluate, Animation, Keyframe, Property, SideInterp};
use crate::expression::ExpressionContext;
use crate::model::TextDocument;
use crate::time::Rational;

/// The words a Text layer's keys point at, and the keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextKeys {
    /// The distinct words the keys point at, kept compact.
    pub words: Vec<String>,
    /// Keyed. Each key's value is a whole index into `words`.
    pub index: Property,
}

impl TextKeys {
    /// Keys made when the stopwatch is turned on: `text` held from `lt`.
    #[must_use]
    pub fn start(text: &str, lt: Rational) -> Self {
        Self {
            words: vec![text.to_owned()],
            index: Property {
                animation: Animation::Keyframed(vec![held(lt, 0.0)]),
                extra: serde_json::Map::new(),
            },
        }
    }

    /// The words to keep when the stopwatch is turned off at `lt`: the ones
    /// shown there.
    #[must_use]
    pub fn stop(&self, lt: f64) -> String {
        self.word_at(lt).to_owned()
    }

    /// The word shown at layer time `lt`. During a fade it is the one that
    /// has more than half the weight.
    #[must_use]
    pub fn word_at(&self, lt: f64) -> &str {
        let (from, fade) = self.span_at(lt);
        let shown = match fade {
            Some((to, p)) if p >= 0.5 => to,
            _ => from,
        };
        self.word(shown)
    }

    /// During a fade at `lt`, the word [`Self::word_at`] does not return and
    /// its weight, below or at a half. Nothing on a held span, at a key, or
    /// where both keys show the same word.
    #[must_use]
    pub fn blend_at(&self, lt: f64) -> Option<(&str, f32)> {
        let (from, (to, p)) = match self.span_at(lt) {
            (from, Some(fade)) => (from, fade),
            (_, None) => return None,
        };
        if p <= 0.0 || p >= 1.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)] // a weight, an f32 picture
        let blend = if p < 0.5 {
            (self.word(to), p as f32)
        } else {
            (self.word(from), (1.0 - p) as f32)
        };
        Some(blend)
    }

    /// These keys with `text` keyed at `lt`. A key already at `lt` takes the
    /// word and keeps its interpolation; otherwise a held key is added. The
    /// word list is then compacted so every key points at a whole index.
    #[must_use]
    pub fn with_word_at(mut self, lt: Rational, text: &str) -> Self {
        let at = match self.words.iter().position(|w| w == text) {
            Some(i) => i,
            None => {
                self.words.push(text.to_owned());
                self.words.len() - 1
            }
        };
        #[allow(clippy::cast_precision_loss)] // a short list of words
        let value = at as f64;
        let mut keys = match std::mem::replace(&mut self.index.animation, Animation::Static(0.0)) {
            Animation::Keyframed(keys) => keys,
            _ => Vec::new(),
        };
        match keys.iter_mut().find(|k| k.time == lt) {
            Some(key) => key.value = value,
            None => {
                let p = keys.partition_point(|k| k.time.to_f64() < lt.to_f64());
                keys.insert(p, held(lt, value));
            }
        }
        // Keep only the words a key points at, in their order, and renumber.
        let old: Vec<usize> = keys.iter().map(|k| self.index_of(k.value)).collect();
        let mut map = vec![0usize; self.words.len()];
        let mut kept = Vec::new();
        for (i, word) in std::mem::take(&mut self.words).into_iter().enumerate() {
            if old.contains(&i) {
                if let Some(slot) = map.get_mut(i) {
                    *slot = kept.len();
                }
                kept.push(word);
            }
        }
        for (key, i) in keys.iter_mut().zip(old) {
            #[allow(clippy::cast_precision_loss)]
            let renumbered = map.get(i).copied().unwrap_or(0) as f64;
            key.value = renumbered;
        }
        self.index.animation = Animation::Keyframed(keys);
        self.words = kept;
        self
    }

    fn word(&self, i: usize) -> &str {
        self.words.get(i).map_or("", String::as_str)
    }

    fn index_of(&self, v: f64) -> usize {
        #[allow(clippy::cast_precision_loss)]
        let last = self.words.len().saturating_sub(1) as f64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
        let i = v.round().clamp(0.0, last) as usize;
        i
    }

    /// The word of the span `lt` is on, and, when that span fades, the word it
    /// fades to and how far it has got, 0 to 1.
    ///
    /// The progress is read off the value, not off a fractional index, so a
    /// fade from word 0 to word 2 never shows word 1.
    fn span_at(&self, lt: f64) -> (usize, Option<(usize, f64)>) {
        let keys = match &self.index.animation {
            Animation::Keyframed(keys) if !keys.is_empty() => keys,
            _ => return (self.index_of(self.index.value_at(lt)), None),
        };
        let after = keys.partition_point(|k| k.time.to_f64() <= lt);
        let Some(prev) = after.checked_sub(1).and_then(|i| keys.get(i)) else {
            return (self.index_of(keys.first().map_or(0.0, |k| k.value)), None);
        };
        let from = self.index_of(prev.value);
        let Some(next) = keys.get(after) else {
            return (from, None);
        };
        let to = self.index_of(next.value);
        let gap = next.value - prev.value;
        if prev.is_hold() || to == from || gap.abs() < 1e-9 {
            return (from, None);
        }
        let v = evaluate(keys, lt).unwrap_or(prev.value);
        (from, Some((to, ((v - prev.value) / gap).clamp(0.0, 1.0))))
    }
}

fn held(time: Rational, value: f64) -> Keyframe {
    Keyframe {
        time,
        value,
        interp_in: SideInterp::Hold,
        interp_out: SideInterp::Hold,
    }
}

/// The layer time `context` names: its comp time on the layer it names, or
/// the comp time itself when it names no layer.
#[must_use]
pub fn layer_time(context: &ExpressionContext) -> f64 {
    let offset = context
        .comp
        .and_then(|id| context.document.comp(id))
        .and_then(|c| c.layers.iter().find(|l| Some(l.id) == context.layer))
        .map_or(Rational::ZERO, |l| l.start_offset.0);
    crate::time::layer_time(context.comp_time, offset)
}

impl TextDocument {
    /// During a fade between two keyed words, the word
    /// [`Self::resolved_text`] does not return and its weight. Only the
    /// rasteriser and the frame key ask. Captions and an expression win over
    /// keys, so they never fade.
    #[must_use]
    pub fn resolved_blend(&self, context: &ExpressionContext) -> Option<(Cow<'_, str>, f32)> {
        if self.captions.is_some() || self.expression.is_some() {
            return None;
        }
        let (word, weight) = self.keys.as_ref()?.blend_at(layer_time(context))?;
        Some((Cow::Borrowed(word), weight))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn s(n: i64) -> Rational {
        Rational::new(n, 1).unwrap()
    }

    fn values(keys: &TextKeys) -> Vec<f64> {
        match &keys.index.animation {
            Animation::Keyframed(k) => k.iter().map(|k| k.value).collect(),
            _ => Vec::new(),
        }
    }

    /// Held keys switch the words at each key and fade nowhere, and a fade
    /// span mixes by how far its value has got.
    #[test]
    fn keyed_words_switch_at_held_keys_and_fade_across_linear_spans() {
        let keys = TextKeys::start("hello", s(1))
            .with_word_at(s(3), "goodbye")
            .with_word_at(s(5), "hello");
        assert_eq!(
            keys.words,
            ["hello", "goodbye"],
            "a shared word is listed once"
        );
        assert_eq!(values(&keys), [0.0, 1.0, 0.0]);
        for (lt, word) in [
            (0.0, "hello"),
            (1.0, "hello"),
            (2.0, "hello"),
            (3.0, "goodbye"),
            (4.9, "goodbye"),
            (5.0, "hello"),
            (9.0, "hello"),
        ] {
            assert_eq!(keys.word_at(lt), word, "at {lt}");
            assert_eq!(
                keys.blend_at(lt),
                None,
                "a held span does not fade, at {lt}"
            );
        }

        // The first key leaves linear: from "hello" at 1 to "goodbye" at 3.
        let mut fading = keys.clone();
        if let Animation::Keyframed(k) = &mut fading.index.animation {
            k[0].interp_out = SideInterp::Linear;
            k[1].interp_in = SideInterp::Linear;
        }
        let blend = |lt: f64| fading.blend_at(lt).map(|(w, x)| (w, (x * 1000.0).round()));
        assert_eq!(fading.word_at(1.5), "hello");
        assert_eq!(blend(1.5), Some(("goodbye", 250.0)));
        assert_eq!(
            fading.word_at(2.0),
            "goodbye",
            "the middle shows the word it fades to"
        );
        assert_eq!(blend(2.0), Some(("hello", 500.0)));
        assert_eq!(blend(2.5), Some(("hello", 250.0)));
        assert_eq!(fading.blend_at(1.0), None, "a key shows its own word alone");
        assert_eq!(fading.blend_at(4.0), None, "the next span still holds");

        // Retyping a key rewrites only that key, and the list stays compact.
        let retyped = keys.clone().with_word_at(s(1), "hi");
        assert_eq!(retyped.words, ["hello", "goodbye", "hi"]);
        assert_eq!(values(&retyped), [2.0, 1.0, 0.0]);
        assert_eq!(
            retyped.word_at(5.0),
            "hello",
            "the other key keeps its word"
        );
        let retyped = retyped.with_word_at(s(5), "goodbye");
        assert_eq!(retyped.words, ["goodbye", "hi"], "an unused word goes");
        assert_eq!(values(&retyped), [1.0, 0.0, 0.0]);
        assert_eq!(retyped.stop(4.0), "goodbye");
    }
}
