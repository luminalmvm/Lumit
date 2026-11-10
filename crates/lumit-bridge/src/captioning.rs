//! Caption layers from the bridge's side: making one from a layer's speech or
//! a subtitle file, what the Captions panel lists, and every edit it makes.
//!
//! # In plain terms
//!
//! The panel works in comp time, the way the Timeline reads. A caption made
//! from a layer keeps its words in that layer's footage time, so every edit
//! here turns the panel's moments into the caption's own clock first, and
//! every caption it lists is placed back on the comp's clock through the
//! source layer's clips. Each edit is one op and one undo step.
//!
//! # Thread role and contract
//!
//! Called on whichever thread the bridge call runs on. Reads a snapshot,
//! commits through the store; no lock is held across anything slow.

use std::sync::Arc;

use lumit_core::{
    captions::{
        cues::{self, Cue, CueWord, Limits, SpeakerLabels},
        show, srt, vtt, Captions, CaptionsEdit,
    },
    model::{Composition, Document, Layer, LayerKind, TextDocument},
    ops::Op,
    time::Rational,
};
use uuid::Uuid;

use crate::{
    api::{
        captions::{
            BridgeCaptionFormat, BridgeCaptionLimits, BridgeCaptionOptions, BridgeCaptions,
            BridgeCue, BridgeCueWord, BridgeSpeakerLabels,
        },
        composition::CompositionReference,
        layer::LayerReference,
        transcript::BridgeSpeaker,
        BridgeError,
    },
    speech::{bridge_time, time_of},
};

pub(crate) mod edit;
pub(crate) mod files;
pub(crate) mod make;

/// How long a caption typed in by hand stays up, in seconds.
const TYPED_SECONDS: i64 = 2;

// ---- Limits ----------------------------------------------------------------

fn seconds_of_frames(comp: &Composition, frames: u32) -> Rational {
    let (num, den) = (comp.frame_rate.num(), comp.frame_rate.den());
    Rational::new(i64::from(frames) * i64::from(den), i64::from(num.max(1)))
        .unwrap_or(Rational::ZERO)
}

fn frames_of_seconds(comp: &Composition, seconds: Rational) -> u32 {
    let frames = seconds.to_f64() * comp.frame_rate.fps();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = frames.round().max(0.0) as u32;
    frames
}

fn millis(seconds: Rational) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ms = (seconds.to_f64() * 1000.0).round().max(0.0) as u32;
    ms
}

fn of_millis(ms: u32) -> Rational {
    Rational::new(i64::from(ms), 1000).unwrap_or(Rational::ZERO)
}

pub(crate) fn limits_in(comp: &Composition, limits: &BridgeCaptionLimits) -> Limits {
    Limits {
        chars_per_line: limits.chars_per_line.clamp(1, 200),
        lines: limits.lines.clamp(1, 4),
        words: limits.words,
        min_duration: of_millis(limits.min_duration_ms),
        max_duration: of_millis(limits.max_duration_ms.max(limits.min_duration_ms).max(100)),
        gap: seconds_of_frames(comp, limits.gap_frames),
        reading_speed: limits.reading_speed,
        split_on_speaker: limits.split_on_speaker,
        speaker_labels: match limits.speaker_labels {
            BridgeSpeakerLabels::Off => SpeakerLabels::Off,
            BridgeSpeakerLabels::Names => SpeakerLabels::Names,
            BridgeSpeakerLabels::Dashes => SpeakerLabels::Dashes,
        },
    }
}

pub(crate) fn limits_out(comp: &Composition, limits: &Limits) -> BridgeCaptionLimits {
    BridgeCaptionLimits {
        chars_per_line: limits.chars_per_line,
        lines: limits.lines,
        words: limits.words,
        min_duration_ms: millis(limits.min_duration),
        max_duration_ms: millis(limits.max_duration),
        gap_frames: frames_of_seconds(comp, limits.gap),
        reading_speed: limits.reading_speed,
        split_on_speaker: limits.split_on_speaker,
        speaker_labels: match limits.speaker_labels {
            SpeakerLabels::Off => BridgeSpeakerLabels::Off,
            SpeakerLabels::Names => BridgeSpeakerLabels::Names,
            SpeakerLabels::Dashes => BridgeSpeakerLabels::Dashes,
        },
    }
}

/// The usual limits for `language`, for a dialog to start from.
pub(crate) fn default_limits(comp: &Composition, language: &str) -> BridgeCaptionLimits {
    limits_out(comp, &Limits::for_language(language))
}

// ---- Reading a caption layer -----------------------------------------------

/// The layer, its composition and its captions, out of one snapshot.
struct Held {
    doc: Arc<Document>,
    comp: Composition,
    layer: Layer,
    captions: Arc<Captions>,
}

fn held(layer: &LayerReference) -> Result<Held, BridgeError> {
    let doc = layer.document()?;
    let comp = doc
        .comp(layer.comp_id)
        .ok_or(BridgeError::InvalidComp)?
        .clone();
    let found = comp
        .layers
        .iter()
        .find(|l| l.id == layer.layer_id)
        .cloned()
        .ok_or(BridgeError::InvalidLayer)?;
    let LayerKind::Text { document } = &found.kind else {
        return Err(BridgeError::NotCaptions);
    };
    let captions = document.captions.clone().ok_or(BridgeError::NotCaptions)?;
    Ok(Held {
        doc,
        comp,
        layer: found,
        captions,
    })
}

fn source_of<'c>(comp: &'c Composition, captions: &Captions) -> Option<&'c Layer> {
    captions
        .source
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
}

pub(crate) fn view(layer: &LayerReference) -> Result<Option<BridgeCaptions>, BridgeError> {
    let doc = layer.document()?;
    let comp = doc.comp(layer.comp_id).ok_or(BridgeError::InvalidComp)?;
    let found = comp
        .layers
        .iter()
        .find(|l| l.id == layer.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;
    let LayerKind::Text { document } = &found.kind else {
        return Ok(None);
    };
    let Some(captions) = document.captions.as_ref() else {
        return Ok(None);
    };
    let placed = show::first_shown(&doc, captions, comp, found);
    let source = source_of(comp, captions);
    let offset = found.start_offset.0;
    let mut cues: Vec<BridgeCue> = captions
        .cues
        .iter()
        .zip(&placed)
        .enumerate()
        .map(|(index, (cue, at))| {
            let to_comp = |t: Rational| -> Option<Rational> {
                match at {
                    // Linked words sit where their cue does, moved by how far
                    // into the cue they come.
                    Some((start, _)) if source.is_some() => {
                        Rational::from_f64_on_grid(start + (t.to_f64() - cue.start.to_f64()), 1000)
                            .ok()
                    }
                    _ => t.checked_add(offset).ok(),
                }
            };
            let grid = |t: f64| Rational::from_f64_on_grid(t, 1000).unwrap_or(Rational::ZERO);
            BridgeCue {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                start: at.map(|(s, _)| bridge_time(grid(s))),
                end: at.map(|(_, e)| bridge_time(grid(e))),
                text: cue.lines().join("\n"),
                words: cue
                    .words
                    .iter()
                    .map(|w| BridgeCueWord {
                        text: w.text.clone(),
                        start: to_comp(w.start).map(bridge_time),
                        end: to_comp(w.end).map(bridge_time),
                        emphasis: w.emphasis,
                    })
                    .collect(),
                breaks: cue
                    .breaks
                    .iter()
                    .map(|&b| u32::try_from(b).unwrap_or(u32::MAX))
                    .collect(),
                label: cue.label.clone(),
                speaker: cue
                    .words
                    .first()
                    .and_then(|w| w.speaker)
                    .map(|s| s.to_string()),
                #[allow(clippy::cast_possible_truncation)]
                reading_speed: cue.reading_speed() as f32,
                offset_x: cue.offset.map_or(0.0, |o| o[0]),
                offset_y: cue.offset.map_or(0.0, |o| o[1]),
            }
        })
        .collect();
    // In the order they show, and the ones out of the edit last.
    cues.sort_by(|a, b| {
        let key = |c: &BridgeCue| {
            c.start
                .map_or(f64::INFINITY, |t| t.num as f64 / t.den as f64)
        };
        key(a).total_cmp(&key(b)).then(a.index.cmp(&b.index))
    });
    Ok(Some(BridgeCaptions {
        language: captions.language.clone(),
        linked: source.is_some(),
        source: source.map(|l| l.id),
        source_name: source.map(|l| l.name.clone()),
        limits: limits_out(comp, &captions.limits),
        speakers: captions
            .speakers
            .iter()
            .map(|s| BridgeSpeaker {
                id: s.id.to_string(),
                name: s.name.clone(),
            })
            .collect(),
        cues,
    }))
}

pub(crate) fn caption_layers(
    comp: &CompositionReference,
) -> Result<Vec<LayerReference>, BridgeError> {
    let found = comp.composition()?;
    Ok(found
        .layers
        .iter()
        .filter(|l| matches!(&l.kind, LayerKind::Text { document } if document.captions.is_some()))
        .map(|l| LayerReference::new(comp.project, comp.id, l.id))
        .collect())
}

// ---- Turning panel moments into the caption's clock ------------------------

/// Where comp time `t` falls on a cue's own clock, and how fast that clock
/// runs against the comp's: its footage's source time for a linked cue, the
/// caption layer's time otherwise. `None` when a linked cue's footage is not
/// playing at `t`.
fn clock_at(held: &Held, item: Option<Uuid>, t: f64) -> Option<(Rational, f64)> {
    let (at, speed) = match (source_of(&held.comp, &held.captions), item) {
        (Some(source), Some(item)) => {
            let heard = show::heard_at(&held.doc, source, t, &|playing, _| playing == item)?;
            (heard.source, heard.speed)
        }
        _ => (t - held.layer.start_offset.0.to_f64(), 1.0),
    };
    Some((Rational::from_f64_on_grid(at.max(0.0), 1000).ok()?, speed))
}

fn commit_cues(
    layer: &LayerReference,
    at: usize,
    remove: usize,
    cues: Vec<Cue>,
) -> Result<(), BridgeError> {
    layer.commit(Op::EditCaptions {
        comp: layer.comp_id,
        layer: layer.layer_id,
        edit: Box::new(CaptionsEdit::Cues { at, remove, cues }),
    })
}

fn commit_all(layer: &LayerReference, captions: Captions) -> Result<(), BridgeError> {
    layer.commit(Op::EditCaptions {
        comp: layer.comp_id,
        layer: layer.layer_id,
        edit: Box::new(CaptionsEdit::Set(Some(Arc::new(captions)))),
    })
}

fn cue(held: &Held, index: u32) -> Result<Cue, BridgeError> {
    held.captions
        .cues
        .get(index as usize)
        .cloned()
        .ok_or(BridgeError::InvalidItem)
}
