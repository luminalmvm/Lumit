//! Hearing speech: the transcription job, and the transcript edits behind
//! [`crate::api::transcript`].
//!
//! # In plain terms
//!
//! A job hears a list of footage items one after another. Each file's sound is
//! decoded a block at a time at the speech model's rate and fed to the model,
//! and the words it hears land on the item as one op as soon as the file is
//! done, so stopping keeps every file already finished. What the model heard
//! is also kept in the cache beside the file's frame index, named by the file,
//! the stream, the pack and the options, so hearing the same file again costs
//! nothing.
//!
//! Voices are told apart across the whole job, so the same person keeps the
//! same speaker in every file of it.
//!
//! # Thread role and contract
//!
//! The job runs on its own thread, `lumit-speech`, one at a time across
//! Lumit; the call that started it waits for it, as a beat detection waits
//! for its worker. No project lock is held while a file is decoded or the model
//! runs: what to hear is read off a snapshot first, and each result is
//! committed through the store afterwards. Edits commit on the caller's
//! thread.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use lumit_core::{
    model::{Document, ProjectItem},
    ops::Op,
    time::{Rational, SourceTime},
    transcript::{Speaker, Transcript, TranscriptEdit, Word},
};
use uuid::Uuid;

use crate::{
    api::{
        effect::BridgeRational,
        footage::FootageReference,
        state::PROJECTS,
        transcript::{
            BridgeLanguage, BridgeSpeaker, BridgeSpeechReady, BridgeTranscribeEvent,
            BridgeTranscribeOptions, BridgeTranscribeOutcome, BridgeTranscribeStage,
            BridgeTranscript, BridgeWord,
        },
        BridgeError,
    },
    frb_generated::StreamSink,
};

/// The rate the speech model hears at, and so the grid word times land on.
const RATE: u32 = 16_000;

/// How many frames of sound are decoded and fed at a time: two seconds.
#[cfg(feature = "media")]
const BLOCK: usize = 1 << 15;

/// The fewest seconds between two progress reports.
const REPORT_EVERY: f64 = 0.2;

// ---- Reading and editing -------------------------------------------------

fn document(project: Uuid) -> Result<Arc<Document>, BridgeError> {
    let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
    let state = projects.get(&project).ok_or(BridgeError::InvalidProject)?;
    let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
    Ok(state.store.snapshot())
}

fn commit(project: Uuid, op: Op) -> Result<(), BridgeError> {
    let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
    let state = projects.get(&project).ok_or(BridgeError::InvalidProject)?;
    let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
    state.store.commit(op).map_err(BridgeError::OpError)?;
    Ok(())
}

fn transcript_of(doc: &Document, item: Uuid) -> Result<Arc<Transcript>, BridgeError> {
    doc.transcripts
        .get(&item)
        .cloned()
        .ok_or(BridgeError::NoTranscript)
}

pub(crate) fn bridge_time(at: Rational) -> BridgeRational {
    BridgeRational {
        num: at.num(),
        den: at.den(),
    }
}

pub(crate) fn time_of(at: BridgeRational) -> Result<Rational, BridgeError> {
    if at.den <= 0 {
        return Err(BridgeError::InvalidTime);
    }
    Rational::new(at.num, at.den).map_err(|_| BridgeError::InvalidTime)
}

fn speaker_id(id: &str) -> Result<Uuid, BridgeError> {
    Uuid::parse_str(id).map_err(|_| BridgeError::InvalidItem)
}

fn word_out(word: &Word) -> BridgeWord {
    BridgeWord {
        text: word.text.clone(),
        start: bridge_time(word.start.0),
        end: bridge_time(word.end.0),
        confidence: word.confidence,
        speaker: word.speaker.map(|s| s.to_string()),
        paragraph: word.paragraph,
    }
}

fn word_in(word: BridgeWord) -> Result<Word, BridgeError> {
    let mut out = Word::new(
        word.text,
        SourceTime(time_of(word.start)?),
        SourceTime(time_of(word.end)?),
        word.confidence,
    );
    out.speaker = word.speaker.as_deref().map(speaker_id).transpose()?;
    out.paragraph = word.paragraph;
    Ok(out)
}

pub(crate) fn read(project: Uuid, item: Uuid) -> Result<Option<BridgeTranscript>, BridgeError> {
    let doc = document(project)?;
    Ok(doc.transcripts.get(&item).map(|t| BridgeTranscript {
        language: t.language.clone(),
        stream: t.stream,
        model: t.model.clone(),
        speakers: t
            .speakers
            .iter()
            .map(|s| BridgeSpeaker {
                id: s.id.to_string(),
                name: s.name.clone(),
            })
            .collect(),
        words: t.words.iter().map(word_out).collect(),
    }))
}

pub(crate) fn has(project: Uuid, item: Uuid) -> bool {
    document(project).is_ok_and(|doc| doc.transcripts.contains_key(&item))
}

pub(crate) fn set(
    project: Uuid,
    item: Uuid,
    transcript: BridgeTranscript,
) -> Result<(), BridgeError> {
    let speakers = transcript
        .speakers
        .into_iter()
        .map(|s| {
            Ok(Speaker {
                id: speaker_id(&s.id)?,
                name: s.name,
            })
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    let words = transcript
        .words
        .into_iter()
        .map(word_in)
        .collect::<Result<Vec<_>, _>>()?;
    let landed = Transcript {
        language: transcript.language,
        stream: transcript.stream,
        words,
        speakers,
        model: transcript.model,
        extra: serde_json::Map::new(),
    };
    commit(
        project,
        Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Set(Some(Arc::new(landed)))),
        },
    )
}

pub(crate) fn splice(
    project: Uuid,
    item: Uuid,
    at: u32,
    remove: u32,
    words: Vec<BridgeWord>,
) -> Result<(), BridgeError> {
    let words = words
        .into_iter()
        .map(word_in)
        .collect::<Result<Vec<_>, _>>()?;
    commit(
        project,
        Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Words {
                at: at as usize,
                remove: remove as usize,
                words,
            }),
        },
    )
}

pub(crate) fn retext(
    project: Uuid,
    item: Uuid,
    index: u32,
    text: String,
) -> Result<(), BridgeError> {
    let doc = document(project)?;
    let transcript = transcript_of(&doc, item)?;
    let mut word = transcript
        .words
        .get(index as usize)
        .cloned()
        .ok_or(BridgeError::InvalidItem)?;
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    word.text = text;
    // Corrected by hand, so no longer a sound nobody wrote down.
    if word.confidence == 0.0 {
        word.confidence = 1.0;
    }
    commit(
        project,
        Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Words {
                at: index as usize,
                remove: 1,
                words: vec![word],
            }),
        },
    )
}

pub(crate) fn reassign(
    project: Uuid,
    item: Uuid,
    from: u32,
    to: u32,
    speaker: Option<String>,
) -> Result<(), BridgeError> {
    let doc = document(project)?;
    let transcript = transcript_of(&doc, item)?;
    let speaker = speaker.as_deref().map(speaker_id).transpose()?;
    let (from, to) = (from as usize, (to as usize).min(transcript.words.len()));
    if from >= to {
        return Ok(());
    }
    let mut words = transcript.words[from..to].to_vec();
    for (i, word) in words.iter_mut().enumerate() {
        word.speaker = speaker;
        // A run given to someone else starts their turn.
        if i == 0 {
            word.paragraph = true;
        }
    }
    let mut edits = vec![words];
    // The word after the run is somebody's turn again.
    let mut after = transcript.words.get(to).cloned();
    if let Some(next) = after.as_mut() {
        next.paragraph = next.paragraph || next.speaker != speaker;
        edits[0].push(next.clone());
    }
    let count = edits[0].len();
    commit(
        project,
        Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Words {
                at: from,
                remove: count,
                words: edits.remove(0),
            }),
        },
    )
}

pub(crate) fn add_speaker(project: Uuid, item: Uuid, name: String) -> Result<String, BridgeError> {
    let doc = document(project)?;
    let transcript = transcript_of(&doc, item)?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let id = Uuid::now_v7();
    let mut speakers = transcript.speakers.clone();
    speakers.push(Speaker { id, name });
    commit(
        project,
        Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Speakers(speakers)),
        },
    )?;
    Ok(id.to_string())
}

pub(crate) fn remove(project: Uuid, item: Uuid) -> Result<(), BridgeError> {
    commit(
        project,
        Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Set(None)),
        },
    )
}

pub(crate) fn rename_speaker(
    project: Uuid,
    speaker: &str,
    name: String,
) -> Result<(), BridgeError> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let speaker = speaker_id(speaker)?;
    let doc = document(project)?;
    let mut ops = lumit_core::transcript::rename_speaker(&doc, speaker, &name);
    ops.extend(crate::captioning::edit::renamed(&doc, speaker, &name));
    if ops.is_empty() {
        return Ok(());
    }
    commit(project, Op::Batch { ops })
}

pub(crate) fn languages() -> Vec<BridgeLanguage> {
    lumit_ml::speech::languages()
        .iter()
        .map(|(code, name)| BridgeLanguage {
            code: (*code).to_owned(),
            name: (*name).to_owned(),
        })
        .collect()
}

pub(crate) fn ready() -> BridgeSpeechReady {
    let runtime = !matches!(
        lumit_ml::runtime::status(),
        lumit_ml::runtime::RuntimeStatus::Missing
    );
    // With two packs in, English may come from the one that translates.
    let whisper = |translate: bool| {
        lumit_ml::speech::pick(translate)
            .and_then(|p| p.manifest.model)
            .and_then(|m| match m.arch {
                lumit_ml::manifest::Arch::Whisper(w) => Some(w),
                _ => None,
            })
    };
    let first = whisper(false);
    BridgeSpeechReady {
        runtime,
        pack: first.is_some(),
        speakers: first.as_ref().is_some_and(|w| w.voices.is_some()),
        translates: whisper(true).is_some_and(|w| w.translates),
    }
}

// ---- The job ---------------------------------------------------------------

/// The one job that may run.
struct Running {
    project: Uuid,
    cancel: Arc<AtomicBool>,
    latest: Option<BridgeTranscribeEvent>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Everyone watching a project's transcriptions. A watcher whose far end has
/// gone is dropped the next time something is said.
static WATCHERS: Mutex<Vec<(Uuid, StreamSink<BridgeTranscribeEvent>)>> = Mutex::new(Vec::new());

pub(crate) fn watch(project: Uuid, events: StreamSink<BridgeTranscribeEvent>) {
    if let Ok(mut watchers) = WATCHERS.lock() {
        watchers.push((project, events));
    }
}

pub(crate) fn cancel(project: Uuid) {
    if let Ok(running) = RUNNING.lock() {
        if let Some(job) = running.as_ref().filter(|j| j.project == project) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub(crate) fn progress(project: Uuid) -> Option<BridgeTranscribeEvent> {
    RUNNING
        .lock()
        .ok()?
        .as_ref()
        .filter(|j| j.project == project)
        .and_then(|j| j.latest.clone())
}

/// A project is closing: stop its job and let its watchers go.
pub(crate) fn forget(project: Uuid) {
    cancel(project);
    if let Ok(mut watchers) = WATCHERS.lock() {
        watchers.retain(|(p, _)| *p != project);
    }
}

/// Say `event` to the job's own listener and to everyone watching.
fn tell(
    project: Uuid,
    own: Option<&StreamSink<BridgeTranscribeEvent>>,
    event: BridgeTranscribeEvent,
) {
    if let Ok(mut running) = RUNNING.lock() {
        if let Some(job) = running.as_mut() {
            job.latest = Some(event.clone());
        }
    }
    if let Some(own) = own {
        let _ = own.add(event.clone());
    }
    if let Ok(mut watchers) = WATCHERS.lock() {
        watchers.retain(|(p, sink)| *p != project || sink.add(event.clone()).is_ok());
    }
}

/// Lets the slot go however the job ends.
struct Release;

impl Drop for Release {
    fn drop(&mut self) {
        if let Ok(mut running) = RUNNING.lock() {
            *running = None;
        }
    }
}

pub(crate) fn transcribe(
    project: Uuid,
    items: Vec<FootageReference>,
    options: BridgeTranscribeOptions,
    events: Option<StreamSink<BridgeTranscribeEvent>>,
) -> Result<BridgeTranscribeOutcome, BridgeError> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut running = RUNNING.lock().map_err(|_| BridgeError::WriteFailed)?;
        if running.is_some() {
            return Err(BridgeError::TranscriptionBusy);
        }
        *running = Some(Running {
            project,
            cancel: Arc::clone(&cancel),
            latest: None,
        });
    }
    let release = Release;
    let ids: Vec<Uuid> = items.iter().map(FootageReference::id).collect();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("lumit-speech".into())
        .spawn(move || {
            let mut say = |event: BridgeTranscribeEvent| tell(project, events.as_ref(), event);
            let outcome = job::run(project, &ids, &options, &cancel, &mut say);
            let _ = send.send(outcome);
        });
    if spawned.is_err() {
        return Err(BridgeError::TranscriptionBusy);
    }
    let outcome = receive.recv().map_err(|_| BridgeError::ReadFailed)?;
    drop(release);
    outcome
}

mod job;

mod voices;
