//! What a Whisper pack says about itself: its shape from `config.json`, and
//! its languages, tasks, special tokens and timing heads from
//! `generation_config.json`, joined with the tokenizer's own special tokens.
//!
//! # Thread role and contract
//!
//! Pure parsing on whichever thread opens the pack. No IO here; the caller
//! reads the files and hands their text over.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use super::tokens::Tokens;
use crate::error::MlError;

/// Everything the decoder needs to know about one Whisper.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Decoder layers, each with its own cache.
    pub layers: usize,
    /// Attention heads in each decoder layer.
    pub heads: usize,
    /// Mel bands the encoder reads.
    pub bands: usize,
    /// Numbers per head in each cached key and value.
    pub head_width: usize,
    /// The (layer, head) pairs whose attention follows the sound in time.
    pub alignment: Vec<(usize, usize)>,
    /// Language code to its token, in the model's own order.
    pub languages: Vec<(String, u32)>,
    /// The end of text, and the first id that is not a word piece.
    pub eot: u32,
    /// The start of a transcript.
    pub sot: u32,
    /// The start of a prompt, for the verbatim one.
    pub prev: u32,
    /// Transcribe and translate.
    pub transcribe: u32,
    /// Translate into English.
    pub translate: u32,
    /// The model's guess that a window holds no speech.
    pub no_speech: u32,
    /// Asked for no timestamp tokens.
    pub no_timestamps: u32,
    /// Never written anywhere.
    pub suppress: HashSet<u32>,
    /// Never written first.
    pub suppress_first: HashSet<u32>,
}

#[derive(Deserialize)]
struct Shape {
    decoder_layers: usize,
    decoder_attention_heads: usize,
    num_mel_bins: usize,
    d_model: usize,
}

#[derive(Deserialize)]
struct Generation {
    #[serde(default)]
    alignment_heads: Vec<(usize, usize)>,
    lang_to_id: BTreeMap<String, u32>,
    task_to_id: BTreeMap<String, u32>,
    no_timestamps_token_id: u32,
    decoder_start_token_id: u32,
    eos_token_id: u32,
    #[serde(default)]
    suppress_tokens: Vec<u32>,
    #[serde(default)]
    begin_suppress_tokens: Vec<u32>,
}

impl Settings {
    /// Join the two files' text and the tokenizer.
    ///
    /// # Errors
    ///
    /// [`MlError::PackUnreadable`] when either file is not what a Whisper
    /// export writes, or the tokenizer lacks a special token the decoder
    /// needs.
    pub fn parse(config: &str, generation: &str, tokens: &Tokens) -> Result<Self, MlError> {
        let shape: Shape = serde_json::from_str(config).map_err(|_| MlError::PackUnreadable)?;
        let generation: Generation =
            serde_json::from_str(generation).map_err(|_| MlError::PackUnreadable)?;
        let named = |text: &str| tokens.special(text).ok_or(MlError::PackUnreadable);
        let task = |name: &str| {
            generation
                .task_to_id
                .get(name)
                .copied()
                .ok_or(MlError::PackUnreadable)
        };
        let mut languages: Vec<(String, u32)> = generation
            .lang_to_id
            .iter()
            .map(|(token, id)| {
                (
                    token
                        .trim_start_matches("<|")
                        .trim_end_matches("|>")
                        .to_owned(),
                    *id,
                )
            })
            .collect();
        languages.sort_by_key(|(_, id)| *id);
        let no_speech = named("<|nospeech|>").or_else(|_| named("<|nocaptions|>"))?;
        let alignment = if generation.alignment_heads.is_empty() {
            // The fallback openai uses: every head of the later half.
            (shape.decoder_layers / 2..shape.decoder_layers)
                .flat_map(|layer| (0..shape.decoder_attention_heads).map(move |head| (layer, head)))
                .collect()
        } else {
            generation.alignment_heads
        };
        Ok(Settings {
            layers: shape.decoder_layers,
            heads: shape.decoder_attention_heads,
            bands: shape.num_mel_bins,
            head_width: shape.d_model / shape.decoder_attention_heads.max(1),
            alignment,
            languages,
            eot: generation.eos_token_id,
            sot: generation.decoder_start_token_id,
            prev: named("<|startofprev|>")?,
            transcribe: task("transcribe")?,
            translate: task("translate")?,
            no_speech,
            no_timestamps: generation.no_timestamps_token_id,
            suppress: generation.suppress_tokens.into_iter().collect(),
            suppress_first: generation.begin_suppress_tokens.into_iter().collect(),
        })
    }

    /// The token of a language, by code.
    #[must_use]
    pub fn language(&self, code: &str) -> Option<u32> {
        self.languages
            .iter()
            .find(|(known, _)| known == code)
            .map(|(_, id)| *id)
    }
}
