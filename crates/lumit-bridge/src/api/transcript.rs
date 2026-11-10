//! A footage item's transcript, and hearing one.
//!
//! # In plain terms
//!
//! Transcribing listens to a footage item's sound and writes down every word
//! with its time and its speaker. The words are what captions are made from
//! and what Cut's text editing cuts on. The listening happens on its own
//! thread, one job at a time, with progress anyone can watch and a cancel that
//! keeps what has finished ([`crate::speech`]).
//!
//! Every edit here is one op and one undo step. Times cross as the exact
//! fractions the document keeps.

use flutter_rust_bridge::frb;

use crate::{
    api::{
        effect::BridgeRational, footage::FootageReference, project::ProjectReference, BridgeError,
    },
    frb_generated::StreamSink,
};

/// One footage item's words.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTranscript {
    /// The language the words are in, such as `en`.
    pub language: String,
    /// Which of the file's audio streams was heard.
    pub stream: u32,
    /// The speech pack that heard it.
    pub model: String,
    pub speakers: Vec<BridgeSpeaker>,
    /// In order of time, never overlapping.
    pub words: Vec<BridgeWord>,
}

/// One word, in the footage item's own source time.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWord {
    pub text: String,
    pub start: BridgeRational,
    pub end: BridgeRational,
    /// 0 to 1. Exactly 0 is a sound heard between words and never written
    /// down, which comes in as "um".
    pub confidence: f32,
    /// A speaker's id, as [`BridgeSpeaker::id`].
    pub speaker: Option<String>,
    /// Starts a paragraph, or somebody else's turn.
    pub paragraph: bool,
}

/// Someone who speaks.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSpeaker {
    pub id: String,
    pub name: String,
}

/// A language the speech model can hear.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLanguage {
    /// Such as `de`.
    pub code: String,
    /// The language's own name for itself, such as "Deutsch".
    pub name: String,
}

/// What a transcription is asked to do.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTranscribeOptions {
    /// A language code, or empty to have it heard.
    pub language: String,
    /// Nought tells speakers apart by ear, one puts every word to one
    /// speaker, more puts them to that many.
    pub speakers: u32,
    /// Which audio stream of each file to hear, or `None` for its first.
    pub stream: Option<u32>,
    /// Write the words down in English, whatever was said.
    pub translate: bool,
    /// What a new speaker is called, with `{n}` for their number, such as
    /// "Speaker {n}". The names are the project's from then on.
    pub speaker_name: String,
    /// Names and words the speakers use, as typed, so they are heard and
    /// spelt right. Empty for none.
    #[frb(default = "")]
    pub vocabulary: String,
}

/// Where a transcription has got to.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTranscribeStage {
    /// Opening the speech model.
    Loading,
    /// Hearing a file.
    Listening,
    /// Telling the voices apart.
    Speakers,
    /// One file finished and landed.
    Done,
    /// One file could not be heard.
    Failed,
    /// The job was stopped.
    Cancelled,
}

/// One step of a transcription.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTranscribeEvent {
    /// The footage item's id.
    pub item: String,
    /// How far through the whole job, 0 to 1.
    pub fraction: f64,
    pub stage: BridgeTranscribeStage,
    /// Seconds of sound heard per second of work, once known.
    pub speed: f64,
    /// Seconds the job should take from here, once known.
    pub remaining: f64,
}

/// How a transcription ended.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeTranscribeOutcome {
    pub done: u32,
    pub failed: u32,
    pub cancelled: bool,
}

/// What is installed for hearing speech.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeSpeechReady {
    /// The model runtime.
    pub runtime: bool,
    /// A speech pack.
    pub pack: bool,
    /// The speech pack can tell voices apart.
    pub speakers: bool,
    /// The speech pack can write what was said in English.
    pub translates: bool,
}

impl FootageReference {
    /// This item's transcript, or `None` before it has one.
    #[frb(sync)]
    pub fn transcript(&self) -> Result<Option<BridgeTranscript>, BridgeError> {
        crate::speech::read(self.project, self.id)
    }

    /// Whether this item has a transcript, without copying its words.
    #[frb(sync)]
    pub fn has_transcript(&self) -> bool {
        crate::speech::has(self.project, self.id)
    }

    /// Give this item a whole transcript, replacing any it has. One undo step.
    #[frb(sync)]
    pub fn set_transcript(&self, transcript: BridgeTranscript) -> Result<(), BridgeError> {
        crate::speech::set(self.project, self.id, transcript)
    }

    /// Put `words` where the `remove` words from `at` were. One undo step.
    #[frb(sync)]
    pub fn set_transcript_words(
        &self,
        at: u32,
        remove: u32,
        words: Vec<BridgeWord>,
    ) -> Result<(), BridgeError> {
        crate::speech::splice(self.project, self.id, at, remove, words)
    }

    /// Correct one word's text, keeping its time. One undo step.
    #[frb(sync)]
    pub fn set_word_text(&self, index: u32, text: String) -> Result<(), BridgeError> {
        crate::speech::retext(self.project, self.id, index, text)
    }

    /// Put the words from `from` up to `to` to `speaker`, or to nobody. One
    /// undo step.
    #[frb(sync)]
    pub fn set_words_speaker(
        &self,
        from: u32,
        to: u32,
        speaker: Option<String>,
    ) -> Result<(), BridgeError> {
        crate::speech::reassign(self.project, self.id, from, to, speaker)
    }

    /// A new speaker on this transcript, and their id. One undo step.
    #[frb(sync)]
    pub fn add_speaker(&self, name: String) -> Result<String, BridgeError> {
        crate::speech::add_speaker(self.project, self.id, name)
    }

    /// Take this item's transcript away. One undo step.
    #[frb(sync)]
    pub fn remove_transcript(&self) -> Result<(), BridgeError> {
        crate::speech::remove(self.project, self.id)
    }
}

impl ProjectReference {
    /// Rename a speaker in every transcript and caption layer that has them.
    /// One undo step.
    #[frb(sync)]
    pub fn rename_speaker(&self, speaker: String, name: String) -> Result<(), BridgeError> {
        crate::speech::rename_speaker(self.id, &speaker, name)
    }

    /// Hear `items` and give each its transcript, as each one finishes.
    ///
    /// Returns when the job is over. One job runs at a time across Lumit, and
    /// a second call while one runs is refused. Cancelling keeps every item
    /// already finished.
    pub fn transcribe(
        &self,
        items: Vec<FootageReference>,
        options: BridgeTranscribeOptions,
        events: Option<StreamSink<BridgeTranscribeEvent>>,
    ) -> Result<BridgeTranscribeOutcome, BridgeError> {
        crate::speech::transcribe(self.id, items, options, events)
    }

    /// Stop this project's transcription, if one is running.
    #[frb(sync)]
    pub fn cancel_transcription(&self) {
        crate::speech::cancel(self.id);
    }

    /// Where this project's transcription has got to, or `None` while none
    /// runs.
    #[frb(sync)]
    pub fn transcription_progress(&self) -> Option<BridgeTranscribeEvent> {
        crate::speech::progress(self.id)
    }

    /// Every step of every transcription of this project, whoever started it,
    /// until the project closes.
    pub fn watch_transcription(
        &self,
        events: StreamSink<BridgeTranscribeEvent>,
    ) -> Result<(), BridgeError> {
        crate::speech::watch(self.id, events);
        Ok(())
    }
}

/// The languages the speech model can hear, in its own order.
#[frb(sync)]
pub fn transcription_languages() -> Vec<BridgeLanguage> {
    crate::speech::languages()
}

/// What is installed for hearing speech.
#[frb(sync)]
pub fn speech_ready() -> BridgeSpeechReady {
    crate::speech::ready()
}
