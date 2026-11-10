//! Editing a cut by its words: the Transcript panel's reading of what a
//! composition, or one footage item, says, and the edits it makes by text.
//!
//! The words and the stretches each edit takes come from
//! `lumit_core::sequence::spoken`, and every edit goes through the Cut
//! workspace's own [`Edit`], so a text edit is the timeline edit it stands
//! for: one `Batch`, one undo step, whole or not at all.
//!
//! Edits run on the UI thread. The reads that are not sync walk a snapshot
//! off it.

use lumit_core::captions::cues::spaced;
use lumit_core::interpretation::StartTimecode;
use lumit_core::model::Document;
use lumit_core::sequence::spoken::{self, fillers::Filler, Look, Token, TokenKind};
use lumit_core::time::FrameRate;

use super::*;
use crate::transcript_export::{self, Said};

mod edits;
mod script;
mod words;
mod written;

use written::{said_in, write_out};

/// What a token of the transcript is.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSpokenKind {
    /// A word.
    Word,
    /// A silence between two words of one clip.
    Pause,
    /// A stretch with no words: between clips, before the first word or
    /// after the last.
    Break,
    /// A word a delete or a lift took out, shown at its edit point. It takes
    /// no frames.
    Removed,
}

/// Why a word is a filler.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFiller {
    None,
    /// A sound and not a word: "um", "uh", or a sound heard and not written.
    Hesitation,
    /// A word or phrase said to fill, set off from the sentence.
    Phrase,
    /// A word or a pair said again straight after: the earlier one.
    Repeat,
    /// A sentence the speaker stopped and said again soon after: every word
    /// of the earlier attempt.
    Retake,
}

/// One token of a transcript.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSpoken {
    pub kind: BridgeSpokenKind,
    /// The word as shown, empty for a pause or a break.
    pub text: String,
    /// The stretch an edit of this token takes, in frames of the
    /// transcript's grid, the end exclusive.
    pub start_frame: i64,
    pub end_frame: i64,
    /// The first frame the word is heard on, and the first after it. A pause
    /// gives its whole silence, from the word before's end to the word
    /// after's start, the silence [`CompositionReference::cut_shorten_pauses`]
    /// measures, and a break gives its own stretch.
    pub heard_start_frame: i64,
    pub heard_end_frame: i64,
    /// Indices into the transcript's `items`, `rows`, `clips` and
    /// `speakers`, `u32::MAX` for none.
    pub item: u32,
    /// The word's place in its item's transcript.
    pub index: u32,
    pub row: u32,
    pub clip: u32,
    pub speaker: u32,
    pub confidence: f32,
    /// The word starts a paragraph or somebody's turn.
    pub paragraph: bool,
    /// Whether a space goes before this token's text on its line: not at a
    /// paragraph's start, not after a break, and not between words of a
    /// script written without spaces.
    pub space_before: bool,
    pub filler: BridgeFiller,
    /// Taking this token out would cut into a word beside it.
    pub tight: bool,
    /// An edit point lies between this token and the one before.
    pub edit_before: bool,
}

/// How the panel looks at the words.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeSpokenLook {
    /// The shortest silence shown as a pause or a break, in milliseconds.
    pub pause_ms: u32,
    /// Show the words a delete or a lift took out, at their edit points.
    pub show_removed: bool,
}

/// Someone who speaks in a transcript.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSpokenSpeaker {
    pub id: String,
    pub name: String,
}

/// What a composition, or a footage item, says, as the Transcript panel
/// shows it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCutTranscript {
    pub tokens: Vec<BridgeSpoken>,
    pub items: Vec<FootageReference>,
    pub rows: Vec<LayerReference>,
    /// Clip ids.
    pub clips: Vec<String>,
    /// In the order they first speak.
    pub speakers: Vec<BridgeSpokenSpeaker>,
    /// Footage heard in the composition that has no transcript.
    pub untranscribed: Vec<FootageReference>,
    /// The language most of the words are in.
    pub language: String,
    /// The rate the frames are counted at.
    pub fps_num: u32,
    pub fps_den: u32,
}

/// A stretch of frames, the end exclusive.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeFrameSpan {
    pub start_frame: i64,
    pub end_frame: i64,
}

/// A stretch of a footage item's source time, in seconds.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSourceSpan {
    pub start: BridgeRational,
    pub end: BridgeRational,
}

/// The kinds of file a transcript is written as.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTranscriptFormat {
    Text,
    Word,
    RichText,
    Markdown,
    Html,
    Csv,
    SubRip,
    WebVtt,
}

/// How a transcript is written to a file.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeTranscriptExport {
    pub format: BridgeTranscriptFormat,
    /// The speaker's name at each turn.
    pub speakers: bool,
    /// A timecode at each paragraph. A CSV file always has times.
    pub timecodes: bool,
    /// Keep the filler words.
    pub fillers: bool,
    /// The frames to write, the end exclusive.
    pub start_frame: i64,
    pub end_frame: i64,
    /// For subtitles: the most characters on a line, and lines on screen.
    pub chars_per_line: u32,
    pub lines: u32,
}

/// How putting a cut together from a script ended.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAssembled {
    pub result: BridgeCutResult,
    /// How many lines were found and placed.
    pub placed: u32,
    /// The lines nobody said, as written.
    pub missing: Vec<String>,
    /// The composition made for it, when none was given.
    pub comp: Option<CompositionReference>,
}

/// A script's lines and where each is said, ready to lay down.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeScriptFound {
    pub lines: Vec<BridgeScriptLine>,
    /// What a composition made for the cut takes: the size and rate of the
    /// first footage found.
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// A line of a script, and its takes, the best first.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeScriptLine {
    pub text: String,
    pub takes: Vec<BridgeWordRun>,
}

/// Removed words to put back: `from` to `to` of `item`, after `clip`.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRemovedRun {
    pub clip: String,
    pub item: FootageReference,
    pub from: u32,
    pub to: u32,
}

/// A correction to a transcript: `remove` words from `at` become `texts`.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeWordsEdit {
    pub item: FootageReference,
    pub at: u32,
    pub remove: u32,
    pub texts: Vec<String>,
}

/// The words `from` to `to`, both included, of one item's transcript.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeWordRun {
    pub item: FootageReference,
    pub from: u32,
    pub to: u32,
}

/// The engine's look from the panel's.
#[frb(ignore)]
fn look_of(look: BridgeSpokenLook) -> Result<Look, BridgeError> {
    Ok(Look {
        pause: Rational::new(i64::from(look.pause_ms), 1000)
            .map_err(|_| BridgeError::InvalidTime)?,
        show_removed: look.show_removed,
    })
}

/// The time of a frame on `rate`.
#[frb(ignore)]
fn time_at(rate: FrameRate, frame: i64) -> Result<Rational, BridgeError> {
    Ok(rate
        .time_of_frame(frame)
        .map_err(|_| BridgeError::InvalidTime)?
        .0)
}

/// The first whole frame at or after `t` on `rate`.
#[frb(ignore)]
fn frame_up(rate: FrameRate, t: Rational) -> i64 {
    let frame = rate.frame_at(CompTime(t));
    let on = rate.time_of_frame(frame).is_ok_and(|at| at.0 >= t);
    frame + i64::from(!on)
}

/// Frame spans as times on `rate`, in order and with any that touch made one,
/// none of them before frame nought.
#[frb(ignore)]
fn times_of(
    rate: FrameRate,
    spans: &[BridgeFrameSpan],
) -> Result<Vec<(Rational, Rational)>, BridgeError> {
    let mut out = Vec::with_capacity(spans.len());
    for span in spans {
        out.push((
            time_at(rate, span.start_frame.max(0))?,
            time_at(rate, span.end_frame.max(0))?,
        ));
    }
    Ok(spoken::merged(out))
}

/// The language most of the words in `tokens` are in.
#[frb(ignore)]
fn language_of(doc: &Document, tokens: &[Token]) -> String {
    let mut languages: Vec<(&str, usize)> = Vec::new();
    for token in tokens.iter().filter(|t| t.kind == TokenKind::Word) {
        let Some(transcript) = token.word.and_then(|w| doc.transcripts.get(&w.item)) else {
            continue;
        };
        match languages
            .iter_mut()
            .find(|(l, _)| *l == transcript.language)
        {
            Some((_, n)) => *n += 1,
            None => languages.push((&transcript.language, 1)),
        }
    }
    languages
        .into_iter()
        .rev()
        .max_by_key(|(_, n)| *n)
        .map(|(l, _)| l.to_owned())
        .unwrap_or_default()
}

/// The index of `value` in `list`, added at the end when it is new.
#[frb(ignore)]
fn slot<T: PartialEq>(list: &mut Vec<T>, value: T) -> u32 {
    let at = list.iter().position(|v| *v == value).unwrap_or_else(|| {
        list.push(value);
        list.len() - 1
    });
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// Tokens as the panel takes them. `comp` names the composition their rows
/// are layers of.
#[frb(ignore)]
fn transcript_of(
    doc: &Document,
    project: Uuid,
    comp: Option<Uuid>,
    tokens: &[Token],
    rate: FrameRate,
) -> BridgeCutTranscript {
    let mut items: Vec<Uuid> = Vec::new();
    let mut rows: Vec<Uuid> = Vec::new();
    let mut clips: Vec<Uuid> = Vec::new();
    let mut speakers: Vec<BridgeSpokenSpeaker> = Vec::new();
    let mut last_speaker: Option<Uuid> = None;
    let mut line: Option<String> = None;
    let mut out = Vec::with_capacity(tokens.len());
    let frame = |t: Rational| rate.frame_at(CompTime(t));
    for token in tokens {
        let word = token.word.and_then(|w| {
            let transcript = doc.transcripts.get(&w.item)?;
            Some((w, transcript, transcript.words.get(w.index)?))
        });
        // A pause is heard as its whole silence, the one `cut_shorten_pauses`
        // measures.
        let (start, end) = token.heard.unwrap_or((token.start, token.end));
        // A word's frames cover all of it and a pause's lie inside its
        // silence, so a pause never reads longer than it measures.
        let (heard_start_frame, heard_end_frame) = match token.kind {
            TokenKind::Pause => {
                let from = frame_up(rate, start);
                (from, frame(end).max(from))
            }
            _ => (frame(start), frame_up(rate, end)),
        };
        let mut spoken = BridgeSpoken {
            kind: match token.kind {
                TokenKind::Word => BridgeSpokenKind::Word,
                TokenKind::Pause => BridgeSpokenKind::Pause,
                TokenKind::Break => BridgeSpokenKind::Break,
                TokenKind::Removed => BridgeSpokenKind::Removed,
            },
            text: String::new(),
            start_frame: frame(token.start),
            end_frame: frame(token.end),
            heard_start_frame,
            heard_end_frame,
            item: u32::MAX,
            index: u32::MAX,
            row: token.row.map_or(u32::MAX, |r| slot(&mut rows, r)),
            clip: token.clip.map_or(u32::MAX, |c| slot(&mut clips, c)),
            speaker: u32::MAX,
            confidence: 1.0,
            paragraph: false,
            space_before: false,
            filler: match token.filler {
                None => BridgeFiller::None,
                Some(Filler::Hesitation) => BridgeFiller::Hesitation,
                Some(Filler::Phrase) => BridgeFiller::Phrase,
                Some(Filler::Repeat) => BridgeFiller::Repeat,
                Some(Filler::Retake) => BridgeFiller::Retake,
            },
            tight: token.tight,
            edit_before: token.edit_before,
        };
        if let Some((at, transcript, word)) = word {
            spoken.text = word.text.clone();
            spoken.item = slot(&mut items, at.item);
            spoken.index = u32::try_from(at.index).unwrap_or(u32::MAX);
            spoken.confidence = word.confidence;
            if let Some(id) = word.speaker {
                let name = transcript.speaker_name(id).unwrap_or_default().to_owned();
                spoken.speaker = slot(
                    &mut speakers,
                    BridgeSpokenSpeaker {
                        id: id.to_string(),
                        name,
                    },
                );
            }
            if token.kind == TokenKind::Word {
                let turn = word.speaker.is_some() && last_speaker != word.speaker;
                spoken.paragraph = word.paragraph || turn;
                last_speaker = word.speaker.or(last_speaker);
            }
        }
        // What the line has so far: the text of the last word on it, if any.
        spoken.space_before = match (&line, token.kind) {
            (_, TokenKind::Break) => false,
            (Some(before), _) if !spoken.paragraph => spaced(before, &spoken.text),
            _ => false,
        };
        line = match token.kind {
            TokenKind::Break => None,
            TokenKind::Pause => line,
            TokenKind::Word | TokenKind::Removed => Some(spoken.text.clone()),
        };
        out.push(spoken);
    }
    BridgeCutTranscript {
        tokens: out,
        items: items
            .into_iter()
            .map(|id| FootageReference::new(project, id))
            .collect(),
        rows: match comp {
            Some(comp) => rows
                .into_iter()
                .map(|id| LayerReference::new(project, comp, id))
                .collect(),
            None => Vec::new(),
        },
        clips: clips.iter().map(Uuid::to_string).collect(),
        speakers,
        untranscribed: Vec::new(),
        language: language_of(doc, tokens),
        fps_num: rate.num(),
        fps_den: rate.den(),
    }
}

impl CompositionReference {
    /// What this composition says, laid out for the Transcript panel: every
    /// heard word, with the pauses and breaks between them, tiled on this
    /// composition's frames. Not sync: it reads a snapshot off the UI
    /// thread.
    pub fn cut_transcript(
        &self,
        look: BridgeSpokenLook,
    ) -> Result<BridgeCutTranscript, BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id()).ok_or(BridgeError::InvalidComp)?;
        let tokens = spoken::tokens(&doc, comp, &look_of(look)?);
        let mut out = transcript_of(
            &doc,
            self.project_id(),
            Some(comp.id),
            &tokens,
            comp.frame_rate,
        );
        for item in spoken::sources(&doc, comp) {
            let footage = matches!(doc.item(item), Some(ProjectItem::Footage(_)));
            if !footage || doc.transcripts.contains_key(&item) {
                continue;
            }
            let item = FootageReference::new(self.project_id(), item);
            #[cfg(feature = "media")]
            let sounds = item.probe().is_some_and(|info| info.audio.is_some());
            #[cfg(not(feature = "media"))]
            let sounds = false;
            if sounds {
                out.untranscribed.push(item);
            }
        }
        Ok(out)
    }

    /// The words under `spans` as plain text, a blank line between
    /// paragraphs: Copy as text. Not sync: it lays out every word.
    pub fn cut_transcript_text(
        &self,
        look: BridgeSpokenLook,
        spans: Vec<BridgeFrameSpan>,
    ) -> Result<String, BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id()).ok_or(BridgeError::InvalidComp)?;
        let tokens = spoken::tokens(&doc, comp, &look_of(look)?);
        let words = said_in(&doc, &tokens, &times_of(comp.frame_rate, &spans)?, true);
        Ok(transcript_export::text(&words, false, None))
    }

    /// Write the words of the frames `how` names to `path`, in the format it
    /// names. Removed words are never written. Not sync: it writes a file.
    pub fn cut_transcript_export(
        &self,
        path: String,
        look: BridgeSpokenLook,
        how: BridgeTranscriptExport,
    ) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id()).ok_or(BridgeError::InvalidComp)?;
        let tokens = spoken::tokens(&doc, comp, &look_of(look)?);
        let rate = comp.frame_rate;
        let range = [(
            time_at(rate, how.start_frame)?,
            time_at(rate, how.end_frame)?,
        )];
        let words = said_in(&doc, &tokens, &range, how.fillers);
        let start = StartTimecode {
            frame: u64::try_from(rate.frame_at(comp.start_timecode())).unwrap_or(0),
            drop_frame: false,
        };
        write_out(
            &path,
            &comp.name,
            &language_of(&doc, &tokens),
            &words,
            &how,
            rate,
            start,
        )
    }

    /// Extract or lift the frames under `spans`: with `ripple` every unlocked
    /// layer closes up and other layers are carried as a Cut ripple carries
    /// them, without it the gaps stay. A transcribed Footage layer whose bar
    /// meets a span is made a Sequence layer first, in the same undo step. A sound clip's edge
    /// this makes gets a 2 ms fade where it has none, so the cut does not
    /// click.
    #[frb(sync)]
    pub fn cut_transcript_remove(
        &self,
        spans: Vec<BridgeFrameSpan>,
        ripple: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |mut edit| {
            let spans = times_of(edit.comp.frame_rate, &spans)?;
            edit.convert_heard(&spans, &[])?;
            edit.take_spans(&spans, ripple)?;
            edit.commit()
        })
    }

    /// Copy what every layer holds under `spans` for a paste, here or in the
    /// Cut timeline: the stretches one after another from nought. The
    /// document is not touched.
    #[frb(sync)]
    pub fn cut_transcript_copy(
        &self,
        spans: Vec<BridgeFrameSpan>,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |mut edit| {
            let spans = times_of(edit.comp.frame_rate, &spans)?;
            // A Footage layer's words are copied as the clip it would become.
            // Nothing is committed, so it stays a Footage layer.
            edit.convert_heard(&spans, &[])?;
            let held = edit.held_spans(&spans)?;
            let mut copied = COPIED.lock().map_err(|_| BridgeError::WriteFailed)?;
            *copied = Some((edit.of.project_id(), held));
            Ok(())
        })
    }

    /// Insert what was copied at `at_frame`, a word's edge: every unlocked
    /// layer is cut there and opened by the copy's length, and each piece
    /// lands on the layer it came from, counted among layers of its kind.
    /// `end_frame` is where the copy ends.
    #[frb(sync)]
    pub fn cut_transcript_paste(&self, at_frame: i64) -> Result<BridgeCutPaste, BridgeError> {
        let mut end_frame = at_frame;
        let result = cut(self, |mut edit| {
            let held = edit.copied()?;
            let at = edit.time(at_frame.max(0))?;
            edit.convert_heard(&[], &[at])?;
            edit.insert_held(held, at, &mut end_frame)
        })?;
        Ok(BridgeCutPaste {
            result,
            end_frame: if result == BridgeCutResult::Done {
                end_frame
            } else {
                at_frame
            },
        })
    }

    /// A drag of words: what lies under `spans` inserted at `to_frame`. With
    /// `copy` the spans stay where they are. Without, they are taken out and
    /// the rest closes up first. `Nothing` when `to_frame` is inside a span,
    /// or without `copy` on the edge of the only one.
    #[frb(sync)]
    pub fn cut_transcript_move(
        &self,
        spans: Vec<BridgeFrameSpan>,
        to_frame: i64,
        copy: bool,
    ) -> Result<BridgeCutPaste, BridgeError> {
        let mut end_frame = to_frame;
        let result = cut(self, |edit| {
            edit.move_words(&spans, to_frame, copy, &mut end_frame)
        })?;
        Ok(BridgeCutPaste {
            result,
            end_frame: if result == BridgeCutResult::Done {
                end_frame
            } else {
                to_frame
            },
        })
    }

    /// Put each run of removed words back after its clip, the clip before
    /// their edit point: a copy of it, and of the clips linked to it there,
    /// trimmed to the words, from halfway into the silence before the first
    /// to halfway into the silence after the last. Inserted with a ripple, or
    /// where they were in the gap a lift left when there is room. Every run
    /// in one undo step.
    #[frb(sync)]
    pub fn cut_transcript_restore(
        &self,
        runs: Vec<BridgeRemovedRun>,
    ) -> Result<BridgeCutResult, BridgeError> {
        let mut parsed = Vec::with_capacity(runs.len());
        for run in runs {
            let clip = Uuid::parse_str(&run.clip).map_err(|_| BridgeError::InvalidItem)?;
            parsed.push((clip, run.item.id(), run.from as usize, run.to as usize));
        }
        cut(self, |edit| edit.restore(&parsed))
    }

    /// Shorten every pause longer than `longer_than_ms` to `keep_ms`, or only
    /// the pauses whose stretches lie in `only`, by taking out the middle of
    /// each, latest first, in one undo step. With `ripple` the cut closes up.
    #[frb(sync)]
    pub fn cut_shorten_pauses(
        &self,
        look: BridgeSpokenLook,
        longer_than_ms: u32,
        keep_ms: u32,
        only: Option<Vec<BridgeFrameSpan>>,
        ripple: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        let look = look_of(look)?;
        cut(self, |mut edit| {
            let rate = edit.comp.frame_rate;
            let doc = edit.of.document()?;
            let tokens = spoken::tokens(&doc, &edit.comp, &look);
            let mut spans = spoken::shorten_pauses(
                &tokens,
                Rational::new(i64::from(longer_than_ms), 1000)?,
                Rational::new(i64::from(keep_ms), 1000)?,
                rate,
            );
            if let Some(only) = only {
                let only = times_of(rate, &only)?;
                spans.retain(|(a, b)| {
                    let mid = spoken::middle(*a, *b);
                    only.iter().any(|(x, y)| *x <= mid && mid < *y)
                });
            }
            if spans.is_empty() {
                return Err(BridgeCutResult::Nothing.into());
            }
            edit.convert_heard(&spans, &[])?;
            edit.take_spans(&spans, ripple)?;
            edit.commit()
        })
    }

    /// The rough cut from a source's words: each of `spans` of `footage` put
    /// down one after another from `at_frame`, as [`Self::cut_place`] puts
    /// one, in one undo step. `end_frame` is where the last ends.
    #[allow(clippy::too_many_arguments)]
    #[frb(sync)]
    pub fn cut_place_words(
        &self,
        target: Option<LayerReference>,
        footage: &FootageReference,
        spans: Vec<BridgeSourceSpan>,
        at_frame: i64,
        insert: bool,
        linked: bool,
        with_picture: bool,
        arrival: Option<BridgeStreamsArrival>,
    ) -> Result<BridgeCutPaste, BridgeError> {
        let arrival = arrival.unwrap_or(BridgeStreamsArrival::First);
        let mut end_frame = at_frame;
        let result = cut(self, |mut edit| {
            let at = edit.time(at_frame.max(0))?;
            let end = edit.place_spans(
                target.map(|l| l.id()),
                footage,
                &spans,
                at,
                insert,
                linked,
                with_picture,
                arrival,
            )?;
            end_frame = edit.frame_up(end)?;
            edit.commit()
        })?;
        Ok(BridgeCutPaste {
            result,
            end_frame: if result == BridgeCutResult::Done {
                end_frame
            } else {
                at_frame
            },
        })
    }

    /// Make `span` the work area, as one undo step.
    #[frb(sync)]
    pub fn cut_transcript_mark(&self, span: BridgeFrameSpan) -> Result<(), BridgeError> {
        let rate = self.composition()?.frame_rate;
        let (start, end) = (
            time_at(rate, span.start_frame)?,
            time_at(rate, span.end_frame)?,
        );
        if end <= start {
            return Err(BridgeError::InvalidTime);
        }
        self.commit(Op::SetWorkArea {
            comp: self.id(),
            work_area: Some((CompTime(start), CompTime(end))),
        })
    }
}

impl FootageReference {
    /// The rate a footage item's own words are counted at: its frames as
    /// interpreted. A file with no picture has no frames, so it is counted
    /// at 30 a second, fine enough to cut speech on.
    #[frb(ignore)]
    fn spoken_rate(&self, doc: &Document) -> Result<FrameRate, BridgeError> {
        let Some(ProjectItem::Footage(f)) = doc.item(self.id) else {
            return Err(BridgeError::InvalidItem);
        };
        let given = f
            .sequence
            .as_ref()
            .map(|s| s.frame_rate)
            .or(f.interpretation.frame_rate);
        #[cfg(feature = "media")]
        let given = given.or_else(|| {
            let video = self.probe()?.video.clone()?;
            FrameRate::new(
                u32::try_from(video.fps_num).ok()?,
                u32::try_from(video.fps_den).ok()?,
            )
            .ok()
        });
        Ok(given.unwrap_or(FrameRate::new(30, 1).map_err(|_| BridgeError::InvalidFrameRate)?))
    }

    /// The file behind this item as a probe reads it: found under the
    /// project's lock and read with none held. `None` when it will not probe.
    #[cfg(feature = "media")]
    #[frb(ignore)]
    fn probe(&self) -> Option<std::sync::Arc<lumit_media::probe::MediaProbe>> {
        let src = {
            let proj = self.project().ok()?;
            let state = proj.read().ok()?;
            let doc = state.store.snapshot();
            let Some(ProjectItem::Footage(f)) = doc.item(self.id) else {
                return None;
            };
            Self::resolve_source(&state, f)?
        };
        crate::probe::ensure_probed(&src)
    }

    /// What this footage item says, on its own clock and frames, laid out as
    /// [`CompositionReference::cut_transcript`] lays a composition's: a
    /// pause between any two words, and no rows, clips or removed words. An
    /// item with no transcript yet comes back in `untranscribed`. Not sync:
    /// it may read the file to learn its rate.
    pub fn source_transcript(
        &self,
        look: BridgeSpokenLook,
    ) -> Result<BridgeCutTranscript, BridgeError> {
        let doc = self.document()?;
        let rate = self.spoken_rate(&doc)?;
        let tokens = spoken::source_tokens(&doc, self.id, rate, &look_of(look)?);
        let mut out = transcript_of(&doc, self.project, None, &tokens, rate);
        if doc.transcripts.contains_key(&self.id) {
            out.items = vec![self.clone()];
        } else {
            out.untranscribed = vec![self.clone()];
        }
        Ok(out)
    }

    /// The words of this item under `spans`, on its own frames, as plain
    /// text.
    pub fn source_transcript_text(
        &self,
        look: BridgeSpokenLook,
        spans: Vec<BridgeFrameSpan>,
    ) -> Result<String, BridgeError> {
        let doc = self.document()?;
        let rate = self.spoken_rate(&doc)?;
        let tokens = spoken::source_tokens(&doc, self.id, rate, &look_of(look)?);
        let words = said_in(&doc, &tokens, &times_of(rate, &spans)?, true);
        Ok(transcript_export::text(&words, false, None))
    }

    /// Write this item's words to `path`, as
    /// [`CompositionReference::cut_transcript_export`] writes a
    /// composition's, its frames counted on its own clock.
    pub fn source_transcript_export(
        &self,
        path: String,
        look: BridgeSpokenLook,
        how: BridgeTranscriptExport,
    ) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let rate = self.spoken_rate(&doc)?;
        let tokens = spoken::source_tokens(&doc, self.id, rate, &look_of(look)?);
        let range = [(
            time_at(rate, how.start_frame)?,
            time_at(rate, how.end_frame)?,
        )];
        let words = said_in(&doc, &tokens, &range, how.fillers);
        let language = doc
            .transcripts
            .get(&self.id)
            .map(|t| t.language.clone())
            .unwrap_or_default();
        let name = match doc.item(self.id) {
            Some(ProjectItem::Footage(f)) => f.name.clone(),
            _ => String::new(),
        };
        // The file's own first timecode, as the source view reads it.
        #[cfg(feature = "media")]
        let start = self
            .media_info()?
            .map_or_else(StartTimecode::default, |info| StartTimecode {
                frame: info.start_frame,
                drop_frame: info.start_drop_frame,
            });
        #[cfg(not(feature = "media"))]
        let start = StartTimecode::default();
        write_out(&path, &name, &language, &words, &how, rate, start)
    }
}

impl ProjectReference {
    /// Correct words, across any number of items, as one undo step. Each
    /// edit puts `texts` where `remove` words from `at` were: the same number
    /// keep their times, speaker and paragraph, any other number is spread
    /// over the old words' stretch, and no texts takes the words out of the
    /// transcript. The media is never touched. Several edits to one item are
    /// read against the transcript as it stands, so they must not overlap.
    #[frb(sync)]
    pub fn correct_words(&self, edits: Vec<BridgeWordsEdit>) -> Result<(), BridgeError> {
        words::correct(self, edits)
    }

    /// Say who said each run of words, or nobody with `None`, as one undo
    /// step. A speaker another item's transcript knows is added to this
    /// one's list first.
    #[frb(sync)]
    pub fn set_spoken_speaker(
        &self,
        runs: Vec<BridgeWordRun>,
        speaker: Option<String>,
    ) -> Result<(), BridgeError> {
        words::set_speaker(self, runs, speaker)
    }

    /// Find `script`'s lines in `footage`'s words, for
    /// [`Self::assemble_from_script`] to lay down. Changes nothing. Not sync:
    /// it reads every word, and the first footage found to size a new
    /// composition by.
    pub fn find_script(
        &self,
        script: String,
        footage: Vec<FootageReference>,
    ) -> Result<BridgeScriptFound, BridgeError> {
        script::find(self, &script, &footage)
    }

    /// Put a cut together from what [`Self::find_script`] found: the best
    /// take of each line laid down in the script's order, with a little room
    /// either side of its words, as a picture clip and its linked sound. A
    /// line's other good takes go on its clips, so T steps through them.
    /// Into `into` at `at_frame` as an insert, or into a new composition
    /// called `name`. The lines nobody said come back in `missing`. One undo
    /// step, taken back whole when the cut is refused.
    #[frb(sync)]
    pub fn assemble_from_script(
        &self,
        found: BridgeScriptFound,
        into: Option<CompositionReference>,
        at_frame: i64,
        name: String,
    ) -> Result<BridgeAssembled, BridgeError> {
        script::assemble(self, found, into, at_frame, name)
    }
}

/// A script file's text: a plain text file as it is, and a Final Draft
/// `.fdx` as its dialogue, a paragraph to a line. Not sync: it reads a file.
pub fn read_script_file(path: String) -> Result<String, BridgeError> {
    script::read(&path)
}
