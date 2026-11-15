//! The two halves of Whisper, open and ready: the encoder that reads a
//! window's spectrogram, and the decoder that writes one word piece at a
//! time from it.
//!
//! The decoder carries a cache from step to step: its own keys and values,
//! which grow by one row a step, and the encoder's, which are made on the
//! first step and never change. For the largest Whisper the encoder's half is
//! about sixty megabytes, so neither half ever leaves the runtime: each step
//! hands its cache straight to the next as [`Held`] tensors and only the next
//! piece's scores come back. When the words are all written, one more pass
//! over the whole sequence fetches the attention the timing heads paid to the
//! sound.
//!
//! The encoder runs on the graphics card where there is one, and the decoder
//! always runs on the processor. DirectML builds its work for each new tensor
//! shape, and a cache that grows by a row a step is a new shape every step:
//! measured, a step there cost most of a second, against tens of milliseconds
//! on the processor.
//!
//! # Thread role and contract
//!
//! A [`Whisper`] owns both sessions and is never shared: every run is an FFI
//! call that may hold the GPU (14-ENGINEERING-RULES §1.3). It lives on the
//! thread that runs the transcription. A single run is not interruptible;
//! cancellation happens between windows, in the caller's own loop.

use std::path::Path;

use super::{config::Settings, mel::Mel, tokens::Tokens};
use crate::{
    error::MlError,
    manifest,
    session::{Feed, Held, Place, Prefer, Session},
};

/// An open Whisper.
#[derive(Debug)]
pub struct Whisper {
    encoder: Session,
    decoder: Session,
    names: manifest::Whisper,
    /// What the pack says about itself.
    pub settings: Settings,
    /// Its word pieces.
    pub tokens: Tokens,
    mel: Mel,
}

/// The decoder's cache between steps.
#[derive(Debug)]
pub struct Cache {
    /// Key then value, per layer, one row per piece so far.
    own: Vec<Held>,
    /// Key then value, per layer, made once from the encoder's output.
    encoder: Vec<Held>,
}

impl Whisper {
    /// Open both halves from a pack's folder.
    ///
    /// # Errors
    ///
    /// [`MlError::PackUnreadable`] when a file the manifest names will not
    /// read, and whatever [`Session::open`] answers for a graph that will not
    /// open.
    pub fn open(dir: &Path, names: &manifest::Whisper, prefer: Prefer) -> Result<Self, MlError> {
        let read = |file: &str| {
            std::fs::read_to_string(dir.join(file)).map_err(|_| MlError::PackUnreadable)
        };
        let tokens = Tokens::parse(&read(&names.tokenizer)?)?;
        let settings = Settings::parse(&read(&names.config)?, &read(&names.generation)?, &tokens)?;
        let encoder = Session::open(dir, &names.encoder, prefer)?;
        let decoder = Session::open(dir, &names.decoder, Prefer::Cpu)?;
        Ok(Whisper {
            mel: Mel::new(settings.bands),
            encoder,
            decoder,
            names: names.clone(),
            settings,
            tokens,
        })
    }

    /// Which provider took the encoder, which is where the time goes.
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.encoder.provider()
    }

    /// Read up to thirty seconds of samples into what the decoder attends
    /// to.
    ///
    /// # Errors
    ///
    /// Whatever the run refuses.
    pub fn encode(&mut self, samples: &[f32]) -> Result<Held, MlError> {
        let features = self.mel.log_mel(samples);
        let shape = [1, self.settings.bands, super::mel::FRAMES];
        let mut out = self.encoder.bound(
            &[(self.names.features.as_str(), Feed::F32(&shape, &features))],
            &[(self.names.hidden.as_str(), Place::Host)],
        )?;
        out.pop().ok_or(MlError::ShapeMismatch)
    }

    /// The first step: the whole prefix at once with no cache. Hands back
    /// the cache it made and the scores at each of `rows`.
    ///
    /// # Errors
    ///
    /// Whatever the run refuses, and [`MlError::ShapeMismatch`] for a row the
    /// prefix does not have.
    pub fn begin(
        &mut self,
        hidden: &Held,
        ids: &[u32],
        rows: &[usize],
    ) -> Result<(Cache, Vec<Vec<f32>>), MlError> {
        let mut wanted = vec![self.names.logits.clone()];
        for layer in 0..self.settings.layers {
            for part in [
                "decoder.key",
                "decoder.value",
                "encoder.key",
                "encoder.value",
            ] {
                wanted.push(format!("{}.{layer}.{part}", self.names.present));
            }
        }
        let outputs: Vec<(&str, Place)> = wanted
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let place = if at == 0 { Place::Host } else { Place::Device };
                (name.as_str(), place)
            })
            .collect();
        let mut out = self.run_whole(hidden, ids, &outputs)?.into_iter();
        let logits = out.next().ok_or(MlError::ShapeMismatch)?;
        let (shape, scores) = logits.read()?;
        let width = shape.last().copied().unwrap_or(0);
        let rows = rows
            .iter()
            .map(|row| {
                scores
                    .get(row * width..(row + 1) * width)
                    .map(<[f32]>::to_vec)
                    .ok_or(MlError::ShapeMismatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut cache = Cache {
            own: Vec::with_capacity(2 * self.settings.layers),
            encoder: Vec::with_capacity(2 * self.settings.layers),
        };
        for (at, held) in out.enumerate() {
            if at % 4 < 2 {
                cache.own.push(held);
            } else {
                cache.encoder.push(held);
            }
        }
        Ok((cache, rows))
    }

    /// One more piece through the cache. Hands back the scores for the piece
    /// after it.
    ///
    /// # Errors
    ///
    /// Whatever the run refuses.
    pub fn step(&mut self, hidden: &Held, cache: &mut Cache, id: u32) -> Result<Vec<f32>, MlError> {
        let ids = [i64::from(id)];
        let flag = [true];
        let mut names = Vec::with_capacity(4 * self.settings.layers);
        for layer in 0..self.settings.layers {
            for part in [
                "decoder.key",
                "decoder.value",
                "encoder.key",
                "encoder.value",
            ] {
                names.push(format!("{}.{layer}.{part}", self.names.past));
            }
        }
        let mut inputs: Vec<(&str, Feed<'_>)> = vec![
            (self.names.input_ids.as_str(), Feed::I64(&[1, 1], &ids)),
            (self.names.encoder_states.as_str(), Feed::Held(hidden)),
            (self.names.use_cache.as_str(), Feed::Bool(&[1], &flag)),
        ];
        for (at, name) in names.iter().enumerate() {
            let layer = at / 4;
            let held = match at % 4 {
                0 => cache.own.get(2 * layer),
                1 => cache.own.get(2 * layer + 1),
                2 => cache.encoder.get(2 * layer),
                _ => cache.encoder.get(2 * layer + 1),
            }
            .ok_or(MlError::ShapeMismatch)?;
            inputs.push((name.as_str(), Feed::Held(held)));
        }
        let mut wanted = vec![self.names.logits.clone()];
        for layer in 0..self.settings.layers {
            for part in ["decoder.key", "decoder.value"] {
                wanted.push(format!("{}.{layer}.{part}", self.names.present));
            }
        }
        let outputs: Vec<(&str, Place)> = wanted
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let place = if at == 0 { Place::Host } else { Place::Device };
                (name.as_str(), place)
            })
            .collect();
        let mut out = self.decoder.bound(&inputs, &outputs)?.into_iter();
        let scores = out.next().ok_or(MlError::ShapeMismatch)?.read()?.1.to_vec();
        cache.own = out.collect();
        Ok(scores)
    }

    /// Where each timing head looked while reading `ids` whole: one matrix
    /// per head, a row per piece and a column per encoder position.
    ///
    /// # Errors
    ///
    /// Whatever the run refuses.
    pub fn attend(&mut self, hidden: &Held, ids: &[u32]) -> Result<Vec<Vec<f32>>, MlError> {
        let mut layers: Vec<usize> = self.settings.alignment.iter().map(|(l, _)| *l).collect();
        layers.sort_unstable();
        layers.dedup();
        let wanted: Vec<String> = layers
            .iter()
            .map(|layer| format!("{}.{layer}", self.names.cross_attentions))
            .collect();
        let outputs: Vec<(&str, Place)> = wanted
            .iter()
            .map(|name| (name.as_str(), Place::Host))
            .collect();
        let out = self.run_whole(hidden, ids, &outputs)?;
        let rows = ids.len();
        let mut matrices = Vec::with_capacity(self.settings.alignment.len());
        for (layer, head) in &self.settings.alignment {
            let at = layers
                .iter()
                .position(|l| l == layer)
                .ok_or(MlError::ShapeMismatch)?;
            let (shape, data) = out.get(at).ok_or(MlError::ShapeMismatch)?.read()?;
            let columns = shape.last().copied().unwrap_or(0);
            let size = rows * columns;
            matrices.push(
                data.get(head * size..(head + 1) * size)
                    .ok_or(MlError::ShapeMismatch)?
                    .to_vec(),
            );
        }
        Ok(matrices)
    }

    /// One run of the decoder over a whole sequence with an empty cache.
    fn run_whole(
        &mut self,
        hidden: &Held,
        ids: &[u32],
        outputs: &[(&str, Place)],
    ) -> Result<Vec<Held>, MlError> {
        let ids: Vec<i64> = ids.iter().map(|id| i64::from(*id)).collect();
        let id_shape = [1, ids.len()];
        let flag = [false];
        let empty_shape = [1, self.settings.heads, 0, self.settings.head_width];
        let mut names = Vec::with_capacity(4 * self.settings.layers);
        for layer in 0..self.settings.layers {
            for part in [
                "decoder.key",
                "decoder.value",
                "encoder.key",
                "encoder.value",
            ] {
                names.push(format!("{}.{layer}.{part}", self.names.past));
            }
        }
        let mut inputs: Vec<(&str, Feed<'_>)> = vec![
            (self.names.input_ids.as_str(), Feed::I64(&id_shape, &ids)),
            (self.names.encoder_states.as_str(), Feed::Held(hidden)),
            (self.names.use_cache.as_str(), Feed::Bool(&[1], &flag)),
        ];
        for name in &names {
            inputs.push((name.as_str(), Feed::F32(&empty_shape, &[])));
        }
        self.decoder.bound(&inputs, outputs)
    }
}
