//! Captions: a Text layer whose words change with the speech.
//!
//! # In plain terms
//!
//! A caption layer is an ordinary Text layer that also carries a list of
//! captions, each a few timed words. At every frame it shows the caption being
//! said, styled like any Text layer, and its animators can follow the words as
//! they are spoken.
//!
//! Captions made from a layer stay linked to it. Their times are kept in the
//! source time of the footage the words were heard in, and each frame carries
//! them through that layer's clips, trims and Retime ([`show`]). So when the
//! layer is cut, moved or slowed down, the captions follow, and words cut out
//! of the edit drop out of the captions too. Captions read from a subtitle
//! file, or typed, keep their own times on the caption layer's clock.
//!
//! # Thread role and contract
//!
//! Pure data and the edits on it. Edited through [`Op::EditCaptions`] on
//! whichever thread commits; read by the render workers from their snapshot.

pub mod ass;
pub mod cues;
mod files;
pub mod lrc;
pub mod show;
pub mod srt;
pub mod ttml;
pub mod vtt;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    model::{Document, Layer, LayerKind},
    ops::{Op, OpError},
};

use cues::{Cue, Limits};

/// A Text layer's captions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Captions {
    /// The layer they were made from, in the same composition. While it is
    /// there, the cues' times are its footage's source times. `None` keeps
    /// them on this layer's own clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Uuid>,
    /// In order of time for each footage item, never overlapping one another.
    pub cues: Vec<Cue>,
    /// The language the words are in, such as `en`, for the files written
    /// from them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// How captions are cut and broken into lines when they are made or
    /// broken again.
    #[serde(default)]
    pub limits: Limits,
    /// The people the words name, with the names their labels print.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<crate::transcript::Speaker>,
    /// Unknown fields from newer Lumit versions.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One change to a layer's captions, carried by [`Op::EditCaptions`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CaptionsEdit {
    /// Make the layer a caption layer, change everything about its captions,
    /// or turn it back into plain text with `None`.
    Set(Option<Arc<Captions>>),
    /// Put `cues` where the `remove` cues from `at` were. Typing, timing,
    /// splitting and joining all go through this, carrying whole cues so the
    /// inverse is exact.
    Cues {
        at: usize,
        remove: usize,
        cues: Vec<Cue>,
    },
}

impl CaptionsEdit {
    /// What the History list calls it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            CaptionsEdit::Set(None) => "Remove captions",
            CaptionsEdit::Set(Some(_)) | CaptionsEdit::Cues { .. } => "Edit captions",
        }
    }
}

/// Why a subtitle file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptionsError {
    /// The file stops making sense at this line, counted from one.
    #[error("line {line} is not part of a caption")]
    Unreadable { line: usize },
    /// The file is readable and holds no captions.
    #[error("there are no captions in the file")]
    Empty,
}

/// Apply `edit` to the captions of `layer` in `comp`, giving back the op that
/// undoes it.
///
/// # Errors
///
/// [`OpError::UnknownComp`] and [`OpError::UnknownLayer`] for what is not
/// there, [`OpError::KindNotConvertible`] for a layer that is not text,
/// [`OpError::BadIndex`] for a splice past the end or on a layer without
/// captions, and [`OpError::InvalidSpan`] when the result would hold a cue
/// that ends before it starts, an empty cue, or two cues of one footage item
/// out of order or overlapping.
pub(crate) fn apply(
    doc: &mut Document,
    comp: Uuid,
    layer: Uuid,
    edit: &CaptionsEdit,
) -> Result<Op, OpError> {
    let comp_ref = doc.comp_mut(comp).ok_or(OpError::UnknownComp)?;
    let layer_ref = comp_ref
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .ok_or(OpError::UnknownLayer)?;
    let LayerKind::Text { document } = &mut layer_ref.kind else {
        return Err(OpError::KindNotConvertible);
    };
    let undo = match edit {
        CaptionsEdit::Set(next) => {
            if let Some(next) = next {
                check(next)?;
            }
            CaptionsEdit::Set(std::mem::replace(&mut document.captions, next.clone()))
        }
        CaptionsEdit::Cues { at, remove, cues } => {
            let current = document.captions.as_mut().ok_or(OpError::BadIndex)?;
            let end = at.checked_add(*remove).ok_or(OpError::BadIndex)?;
            if end > current.cues.len() {
                return Err(OpError::BadIndex);
            }
            // Copied only when a snapshot still holds it, so the splices of
            // one batch share one copy. Taking cues out never breaks the
            // order, so only new ones are checked, and a refused splice is
            // put back.
            let next = Arc::make_mut(current);
            let removed: Vec<Cue> = next.cues.splice(*at..end, cues.iter().cloned()).collect();
            if !cues.is_empty() {
                if let Err(refused) = check(next) {
                    next.cues.splice(*at..*at + cues.len(), removed);
                    return Err(refused);
                }
            }
            CaptionsEdit::Cues {
                at: *at,
                remove: cues.len(),
                cues: removed,
            }
        }
    };
    Ok(Op::EditCaptions {
        comp,
        layer,
        edit: Box::new(undo),
    })
}

/// The edit that points `layer`'s captions at layer `to` when they were made
/// from one of `from`, in comp `comp`: what precomposing their layer needs.
pub(crate) fn repointed(comp: Uuid, layer: &Layer, from: &[Uuid], to: Uuid) -> Option<Op> {
    let LayerKind::Text { document } = &layer.kind else {
        return None;
    };
    let captions = document.captions.as_ref()?;
    captions.source.filter(|s| *s != to && from.contains(s))?;
    Some(Op::EditCaptions {
        comp,
        layer: layer.id,
        edit: Box::new(CaptionsEdit::Set(Some(Arc::new(Captions {
            source: Some(to),
            ..(**captions).clone()
        })))),
    })
}

/// Every cue has words and starts before it ends, its words in order and its
/// line breaks inside it, and the cues of one footage item run in order
/// without overlapping.
fn check(captions: &Captions) -> Result<(), OpError> {
    for cue in &captions.cues {
        let words_in_order = cue.words.windows(2).all(|w| w[0].start <= w[1].start)
            && cue.words.iter().all(|w| w.start <= w.end);
        let breaks_inside = cue.breaks.windows(2).all(|b| b[0] < b[1])
            && cue.breaks.iter().all(|&b| b > 0 && b < cue.words.len());
        if cue.words.is_empty() || cue.end < cue.start || !words_in_order || !breaks_inside {
            return Err(OpError::InvalidSpan);
        }
    }
    // An edit may play a file's later words first, so the list itself is in
    // no particular order of source time: each item's cues are sorted here.
    let mut spans: Vec<(Option<Uuid>, crate::time::Rational, crate::time::Rational)> = captions
        .cues
        .iter()
        .map(|c| (c.item, c.start, c.end))
        .collect();
    spans.sort();
    let overlapping = spans
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[1].1 < pair[0].2);
    if overlapping {
        return Err(OpError::InvalidSpan);
    }
    Ok(())
}

/// Cues in order with no two of one footage item overlapping: each is cut
/// short where the next of its item starts. What a subtitle file read in is
/// put through before it lands, since files in the wild overlap.
#[must_use]
pub fn tidy(mut cues: Vec<Cue>) -> Vec<Cue> {
    cues.retain(|c| !c.words.is_empty());
    cues.sort_by_key(|c| c.start);
    for i in 0..cues.len() {
        let next = cues[i + 1..]
            .iter()
            .find(|c| c.item == cues[i].item)
            .map(|c| c.start);
        if let Some(next) = next {
            let cue = &mut cues[i];
            cue.end = cue.end.min(next).max(cue.start);
            for word in &mut cue.words {
                word.end = word.end.min(cue.end).max(word.start.min(cue.end));
                word.start = word.start.min(word.end);
            }
        }
    }
    cues
}

/// The captions in a subtitle file, read by the format its name says:
/// WebVTT, Timed Text, Advanced SubStation, lyrics, and SubRip for anything
/// else.
///
/// # Errors
///
/// What the format's own reader answers.
pub fn read_file(name: &str, text: &str) -> Result<Vec<Cue>, CaptionsError> {
    let lower = name.to_lowercase();
    let ends = |ext: &str| lower.ends_with(ext);
    if ends(".vtt") {
        vtt::read(text)
    } else if ends(".ttml") || ends(".dfxp") || ends(".xml") {
        ttml::read(text)
    } else if ends(".ass") || ends(".ssa") {
        ass::read(text)
    } else if ends(".lrc") {
        lrc::read(text)
    } else {
        srt::read(text)
    }
}

/// [`read_file`] on a file's bytes, read as UTF-8 or UTF-16 by the mark at
/// its start, UTF-8 without one, and Windows-1252 when it is not UTF-8.
///
/// # Errors
///
/// What [`read_file`] answers.
pub fn read_bytes(name: &str, bytes: &[u8]) -> Result<Vec<Cue>, CaptionsError> {
    read_file(name, &files::text_of(bytes))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{
        captions::cues::CueWord,
        edits::{base_layer, new_comp_ops, CompSettings},
        model::{LinearColour, TextDocument},
        time::Rational,
    };

    fn s(ms: i64) -> Rational {
        Rational::new(ms, 1000).unwrap()
    }

    fn cue(text: &str, from: i64, to: i64) -> Cue {
        Cue {
            start: s(from),
            end: s(to),
            words: vec![CueWord {
                text: text.into(),
                start: s(from),
                end: s(to),
                speaker: None,
                emphasis: false,
            }],
            breaks: Vec::new(),
            label: None,
            dashes: false,
            item: None,
            offset: None,
            extra: serde_json::Map::new(),
        }
    }

    fn captions(cues: Vec<Cue>) -> Option<Arc<Captions>> {
        Some(Arc::new(Captions {
            source: None,
            cues,
            language: "en".into(),
            limits: Limits::default(),
            speakers: Vec::new(),
            extra: serde_json::Map::new(),
        }))
    }

    /// **Every caption edit undoes exactly, and a broken one is refused.**
    /// Captions land on a Text layer, one is retyped, and walking the inverses
    /// back leaves the document as it was; an edit that would leave a caption
    /// empty, backwards or overlapping the next changes nothing. Every typed
    /// correction is journalled as one of these, so an inverse that lost a
    /// caption would lose it from the project.
    #[test]
    fn every_caption_edit_undoes_exactly() {
        let mut doc = Document::new();
        let (comp, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            crate::ops::apply(&mut doc, op).unwrap();
        }
        let duration = doc.comp(comp).unwrap().duration.0;
        let document = TextDocument {
            text: String::new(),
            expression: None,
            size: 60.0,
            fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
            path: None,
            path_offset: crate::anim::Property::zero(),
            animators: Vec::new(),
            style: Default::default(),
            paragraph: Default::default(),
            captions: None,
            extra: serde_json::Map::new(),
        };
        let text = base_layer(
            "Captions".into(),
            LayerKind::Text { document },
            duration,
            Default::default(),
        );
        let null = base_layer("Null".into(), LayerKind::Null, duration, Default::default());
        let (layer, other) = (text.id, null.id);
        for (index, layer) in [text, null].into_iter().enumerate() {
            let add = Op::AddLayer {
                comp,
                index,
                layer: Box::new(layer),
            };
            crate::ops::apply(&mut doc, &add).unwrap();
        }
        let before = doc.clone();
        let edit = |doc: &mut Document, edit: CaptionsEdit| apply(doc, comp, layer, &edit);

        let mut undo = vec![
            edit(
                &mut doc,
                CaptionsEdit::Set(captions(vec![cue("One", 0, 1000), cue("Two", 1000, 2000)])),
            )
            .unwrap(),
            edit(
                &mut doc,
                CaptionsEdit::Cues {
                    at: 1,
                    remove: 1,
                    cues: vec![cue("Three", 1500, 2500)],
                },
            )
            .unwrap(),
        ];
        let shown = |doc: &Document| -> Vec<String> {
            let found = doc
                .comp(comp)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer)
                .unwrap();
            let LayerKind::Text { document } = &found.kind else {
                panic!()
            };
            document
                .captions
                .as_ref()
                .unwrap()
                .cues
                .iter()
                .map(Cue::text)
                .collect()
        };
        assert_eq!(shown(&doc), vec!["One", "Three"]);

        let edited = doc.clone();
        let mut backwards = cue("Back", 3000, 3500);
        backwards.end = s(2900);
        let mut empty = cue("", 3000, 3500);
        empty.words.clear();
        for refused in [
            CaptionsEdit::Cues {
                at: 3,
                remove: 0,
                cues: Vec::new(),
            },
            CaptionsEdit::Cues {
                at: 1,
                remove: 0,
                cues: vec![cue("Over", 500, 1200)],
            },
            CaptionsEdit::Cues {
                at: 2,
                remove: 0,
                cues: vec![backwards],
            },
            CaptionsEdit::Cues {
                at: 2,
                remove: 0,
                cues: vec![empty],
            },
            CaptionsEdit::Set(captions(vec![cue("A", 0, 2000), cue("B", 1000, 3000)])),
        ] {
            assert!(edit(&mut doc, refused).is_err());
        }
        assert_eq!(
            apply(&mut doc, comp, other, &CaptionsEdit::Set(None)),
            Err(OpError::KindNotConvertible)
        );
        assert_eq!(
            apply(&mut doc, comp, Uuid::now_v7(), &CaptionsEdit::Set(None)),
            Err(OpError::UnknownLayer)
        );
        assert_eq!(
            apply(&mut doc, Uuid::now_v7(), layer, &CaptionsEdit::Set(None)),
            Err(OpError::UnknownComp)
        );
        assert_eq!(doc, edited, "a refused edit changes nothing");

        undo.push(edit(&mut doc, CaptionsEdit::Set(None)).unwrap());
        assert_eq!(CaptionsEdit::Set(None).name(), "Remove captions");
        assert_eq!(
            CaptionsEdit::Cues {
                at: 0,
                remove: 0,
                cues: Vec::new()
            }
            .name(),
            "Edit captions"
        );
        for op in undo.iter().rev() {
            crate::ops::apply(&mut doc, op).unwrap();
        }
        assert_eq!(doc, before);

        // A file's overlapping captions are cut where the next starts, and an
        // empty one goes.
        let mut blank = cue("", 0, 100);
        blank.words.clear();
        let tidied = tidy(vec![cue("Late", 2000, 3000), cue("Early", 0, 2500), blank]);
        let spans: Vec<(Rational, Rational)> = tidied.iter().map(|c| (c.start, c.end)).collect();
        assert_eq!(spans, vec![(s(0), s(2000)), (s(2000), s(3000))]);
        assert_eq!(tidied[0].words[0].end, s(2000));
    }
}
