//! Caption layers: made from a layer's speech or a subtitle file, listed and
//! edited in the Captions panel, and written out as subtitle files.
//!
//! # In plain terms
//!
//! A caption layer is a Text layer that shows timed captions
//! ([`lumit_core::captions`]). Everything here works in comp time, the way the
//! panel and the Timeline read; the engine turns it into each caption's own
//! clock ([`crate::captioning`]). Every edit is one undo step.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::{
    assets::BridgeTextDocument, composition::CompositionReference, effect::BridgeRational,
    footage::FootageReference, layer::LayerReference, transcript::BridgeSpeaker, BridgeError,
};

/// How speakers show in the captions.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSpeakerLabels {
    Off,
    /// The speaker's name in front, when it changes.
    Names,
    /// A dash in front of each speaker's line, when two share a caption.
    Dashes,
}

/// What a caption may hold, and how long it stays.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeCaptionLimits {
    pub chars_per_line: u32,
    pub lines: u32,
    /// The most words a caption holds; nought is no limit, one is a word at a
    /// time.
    pub words: u32,
    pub min_duration_ms: u32,
    pub max_duration_ms: u32,
    /// Frames of the comp left empty between two captions.
    pub gap_frames: u32,
    /// Characters a second a caption stays readable at; nought leaves it out.
    pub reading_speed: u32,
    pub split_on_speaker: bool,
    pub speaker_labels: BridgeSpeakerLabels,
}

/// How a caption layer is made from a layer's speech.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCaptionOptions {
    pub limits: BridgeCaptionLimits,
    /// The new layer's name.
    pub name: String,
    /// Keep "um" and "uh" in the captions.
    pub fillers: bool,
}

/// A caption layer, as the Captions panel lists it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCaptions {
    pub language: String,
    /// Made from a layer, and following it.
    pub linked: bool,
    /// That layer, while it is in the composition.
    pub source: Option<Uuid>,
    /// That layer's name.
    pub source_name: Option<String>,
    pub limits: BridgeCaptionLimits,
    pub speakers: Vec<BridgeSpeaker>,
    /// In the order they show; captions whose speech is not in the edit come
    /// last.
    pub cues: Vec<BridgeCue>,
}

/// One caption.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCue {
    /// Its place in the layer's own list, which every edit names it by.
    pub index: u32,
    /// Where it first shows in comp time, or `None` when its speech is not in
    /// the edit.
    pub start: Option<BridgeRational>,
    pub end: Option<BridgeRational>,
    /// As it reads, its lines on lines of their own.
    pub text: String,
    pub words: Vec<BridgeCueWord>,
    /// The words that start a new line.
    pub breaks: Vec<u32>,
    pub label: Option<String>,
    /// Who says it, as a speaker id.
    pub speaker: Option<String>,
    /// Characters a second it asks of a reader.
    pub reading_speed: f32,
    /// How far it is moved from where the layer puts it, in the layer's own
    /// pixels.
    pub offset_x: f64,
    pub offset_y: f64,
}

/// One word of a caption, in comp time.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCueWord {
    pub text: String,
    pub start: Option<BridgeRational>,
    pub end: Option<BridgeRational>,
    pub emphasis: bool,
}

/// A subtitle file format.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCaptionFormat {
    Srt,
    Vtt,
    /// Timed Text, the XML file broadcasters ask for.
    Ttml,
    /// The words alone, one caption a line.
    Txt,
}

impl CompositionReference {
    /// A caption layer showing what `source` says, at the top of the stack,
    /// styled as `look`. The source's footage must be transcribed first. One
    /// undo step.
    #[frb(sync)]
    pub fn create_captions(
        &self,
        source: LayerReference,
        options: BridgeCaptionOptions,
        look: BridgeTextDocument,
    ) -> Result<LayerReference, BridgeError> {
        crate::captioning::make::create(self, source.layer_id, options, look)
    }

    /// A caption layer from an `.srt` or `.vtt` file, styled as `look`. One
    /// undo step.
    #[frb(sync)]
    pub fn import_captions(
        &self,
        path: String,
        name: String,
        look: BridgeTextDocument,
    ) -> Result<LayerReference, BridgeError> {
        crate::captioning::make::import(self, path, name, look)
    }

    /// The usual caption limits for `language`, for a dialog to start from.
    #[frb(sync)]
    pub fn caption_limits_for(&self, language: String) -> Result<BridgeCaptionLimits, BridgeError> {
        Ok(crate::captioning::default_limits(
            &self.composition()?,
            &language,
        ))
    }

    /// The composition's caption layers, top first.
    #[frb(sync)]
    pub fn caption_layers(&self) -> Result<Vec<LayerReference>, BridgeError> {
        crate::captioning::caption_layers(self)
    }
}

impl LayerReference {
    /// The footage this layer plays sound from, down through any Precomp
    /// layers, each once: what has to be heard to caption it.
    #[frb(sync)]
    pub fn heard_items(&self) -> Result<Vec<FootageReference>, BridgeError> {
        crate::captioning::make::heard_items(self)
    }

    /// This layer's captions, or `None` when it is not a caption layer.
    #[frb(sync)]
    pub fn captions(&self) -> Result<Option<BridgeCaptions>, BridgeError> {
        crate::captioning::view(self)
    }

    /// Give caption `cue` new text. Words that stay keep their times.
    #[frb(sync)]
    pub fn set_cue_text(&self, cue: u32, text: String) -> Result<(), BridgeError> {
        crate::captioning::edit::set_text(self, cue, text)
    }

    /// Show caption `cue` from `start` to `end`, in comp time.
    #[frb(sync)]
    pub fn set_cue_span(
        &self,
        cue: u32,
        start: BridgeRational,
        end: BridgeRational,
    ) -> Result<(), BridgeError> {
        crate::captioning::edit::set_span(self, cue, start, end)
    }

    /// Cut caption `cue` in two before its word `at_word`.
    #[frb(sync)]
    pub fn split_cue(&self, cue: u32, at_word: u32) -> Result<(), BridgeError> {
        crate::captioning::edit::split(self, cue, at_word)
    }

    /// Cut the caption showing at comp time `at` in two there.
    #[frb(sync)]
    pub fn split_cue_at(&self, at: BridgeRational) -> Result<(), BridgeError> {
        crate::captioning::edit::split_at(self, at)
    }

    /// Join `count` captions from `first` into one.
    #[frb(sync)]
    pub fn merge_cues(&self, first: u32, count: u32) -> Result<(), BridgeError> {
        crate::captioning::edit::merge(self, first, count)
    }

    /// A new caption saying `text` at comp time `at`, and its index.
    #[frb(sync)]
    pub fn add_cue(&self, at: BridgeRational, text: String) -> Result<u32, BridgeError> {
        crate::captioning::edit::add(self, at, text)
    }

    /// Delete these captions.
    #[frb(sync)]
    pub fn remove_cues(&self, cues: Vec<u32>) -> Result<(), BridgeError> {
        crate::captioning::edit::remove(self, cues)
    }

    /// Where caption `cue`'s lines break, by the words that start them.
    #[frb(sync)]
    pub fn set_cue_breaks(&self, cue: u32, breaks: Vec<u32>) -> Result<(), BridgeError> {
        crate::captioning::edit::set_breaks(self, cue, breaks)
    }

    /// Move caption `cue` this far from where the layer puts it, in comp
    /// pixels; nought both ways puts it back.
    #[frb(sync)]
    pub fn set_cue_offset(&self, cue: u32, x: f64, y: f64) -> Result<(), BridgeError> {
        crate::captioning::edit::set_offset(self, cue, x, y)
    }

    /// Raise `cues` to the top of the picture, or put them back down. One
    /// undo step.
    #[frb(sync)]
    pub fn raise_cues(&self, cues: Vec<u32>, raised: bool) -> Result<(), BridgeError> {
        crate::captioning::edit::raise(self, cues, raised)
    }

    /// Pick a word out for emphasis, or put it back.
    #[frb(sync)]
    pub fn set_word_emphasis(&self, cue: u32, word: u32, on: bool) -> Result<(), BridgeError> {
        crate::captioning::edit::set_emphasis(self, cue, word, on)
    }

    /// New limits; with `rebuild`, every caption is cut again from its words.
    #[frb(sync)]
    pub fn set_caption_limits(
        &self,
        limits: BridgeCaptionLimits,
        rebuild: bool,
    ) -> Result<(), BridgeError> {
        crate::captioning::edit::set_limits(self, limits, rebuild)
    }

    /// The language the captions are in, for the files written from them.
    #[frb(sync)]
    pub fn set_caption_language(&self, language: String) -> Result<(), BridgeError> {
        crate::captioning::edit::set_language(self, language)
    }

    /// Swap every `find` for `with` across the captions, and say how many.
    #[frb(sync)]
    pub fn replace_in_captions(
        &self,
        find: String,
        with: String,
        match_case: bool,
        whole_word: bool,
    ) -> Result<u32, BridgeError> {
        crate::captioning::edit::replace(self, find, with, match_case, whole_word)
    }

    /// Write the captions this layer shows to `path` as `format`, on the
    /// comp's clock.
    pub fn export_captions(
        &self,
        format: BridgeCaptionFormat,
        path: String,
    ) -> Result<(), BridgeError> {
        crate::captioning::files::export(self, format, path)
    }
}
