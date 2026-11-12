//! Voice isolation: the speech kept, everything else turned down.
//!
//! # In plain terms
//!
//! A trained model listens to the whole of the sound the effect is given and
//! says which part of it is somebody speaking. This file holds the rows and
//! what is done with that answer a block at a time: the voice and what is
//! left over each get a level of their own, and Wet sets how much of that is
//! heard against the untouched sound. Background only plays what the model
//! calls background, to hear what is being taken away.
//!
//! The model is not here. This crate knows nothing about addons or model
//! runtimes, so whoever opens the chain and has one hands it in as a
//! [`VoiceModel`] through [`open`]. The definition's own `open_audio` has
//! none to hand, and a processor without one passes its sound through
//! untouched, which is also what happens on a machine without the Voice
//! addon: the badge names [`PACK`] and the sound is not changed.
//!
//! # Thread role and contract
//!
//! The bake's thread. [`AudioProcessor::look`] asks the model once, before the
//! first block, with the link's whole input; the blocks then only read the
//! answer, so they allocate nothing and are bit-identical however the run is
//! split. The answer is a function of the input alone, which is what makes a
//! second bake the same as the first.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::dsp::db::gain_of_db;
use super::dsp::smoother::Smoother;
use super::modulation::{frame, put, row};
use crate::fx::{
    audio_chain::AudioProcessor, audio_chain::AUDIO_CHANNELS, params::ParamId, registry::EffectDef,
    registry::EffectMetadata, schema::EffectSchema,
};

/// How long a moved row takes to arrive, the same ramp Gain uses.
const SMOOTH_MS: f64 = 5.0;

/// The bottom of the Background row, where it means off rather than very
/// quiet: the leftover is taken out altogether.
pub const BACKGROUND_OFF_DB: f64 = -60.0;

/// The addon that finds the voice, by its catalogue id, for the badge on a
/// machine that has not got it.
pub const PACK: &str = "deepfilternet3";

/// The effect's `match_name`, so the code that opens chains can find it.
pub const MATCH_NAME: &str = <AudioVoice as EffectMetadata>::SCHEMA.match_name;

/// The Voice isolation effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_voice",
    label = "Voice isolation",
    version = 1,
    category = Audio,
    cost = Heavy,
    roi = Exact,
    // Sound, so no matte and no picture: the Audio family's rule.
    matte = false,
)]
pub struct AudioVoice {
    /// The voice the model found, in dB.
    #[bounded(min = -24.0, max = 12.0, default = 0.0, unit = Raw)]
    pub voice: f32,
    /// Everything else, in dB. The bottom of the travel takes it out
    /// altogether.
    #[bounded(min = -60.0, max = 0.0, default = -60.0, unit = Raw)]
    pub background: f32,
    /// How much of the result is heard against the untouched sound. A little
    /// of the original left in sounds less processed than none.
    #[bounded(min = 0.0, max = 100.0, default = 80.0, unit = Percent)]
    pub wet: f32,
    /// Play only what the model calls background, to hear what is taken away.
    #[toggle(default = false)]
    pub background_only: bool,
}

/// The voice the model found in each channel of a link's input, at the bake's
/// own rate and lined up with it sample for sample.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Voices {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

/// What asking a [`VoiceModel`] came to.
#[derive(Debug, Clone)]
pub enum Heard {
    /// The voice in each channel.
    Voice(Arc<Voices>),
    /// Nothing on this machine can find one: no model runtime, or no Voice
    /// addon. The sound goes through untouched and the badge says what to
    /// install.
    Missing,
    /// The model would not run, with its own sentence for the badge.
    Failed(String),
    /// The bake was stopped part way, so nothing it made will be heard.
    Stopped,
}

/// The model, as the code that opens the chain hands it in.
pub trait VoiceModel: Send + Sync {
    /// The voice in `input`, interleaved stereo at `rate`, the whole of a
    /// link's input from its first sample.
    fn voices(&self, input: &[f32], rate: u32) -> Heard;
}

/// The Voice isolation effect's behaviour.
pub struct AudioVoiceDef;

impl EffectDef for AudioVoiceDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioVoice as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    /// Without a model: the sound goes through untouched. The chain opens
    /// this effect through [`open`] instead, with one.
    fn open_audio(
        &self,
        _state: Option<Vec<u8>>,
        values: &[(ParamId, f64)],
        rate: u32,
        _offline: bool,
    ) -> Option<Arc<dyn AudioProcessor>> {
        Some(open(values, rate, None))
    }
}

/// One open Voice isolation, finding the voice with `model`.
#[must_use]
pub fn open(
    values: &[(ParamId, f64)],
    rate: u32,
    model: Option<Arc<dyn VoiceModel>>,
) -> Arc<dyn AudioProcessor> {
    let rate = rate.max(1);
    let per_second = f64::from(rate);
    let levels = Levels::of(values);
    Arc::new(VoiceAudio {
        model,
        rate,
        state: Mutex::new(VoiceState {
            heard: None,
            voice: Smoother::new(SMOOTH_MS, per_second, levels.voice),
            background: Smoother::new(SMOOTH_MS, per_second, levels.background),
            wet: Smoother::new(SMOOTH_MS, per_second, levels.wet),
        }),
    })
}

/// What the rows come to as multipliers.
struct Levels {
    voice: f64,
    background: f64,
    wet: f64,
    background_only: bool,
}

impl Levels {
    fn of(values: &[(ParamId, f64)]) -> Self {
        let background = row(values, AudioVoice::BACKGROUND, BACKGROUND_OFF_DB);
        Levels {
            voice: gain_of_db(row(values, AudioVoice::VOICE, 0.0)),
            background: if background <= BACKGROUND_OFF_DB {
                0.0
            } else {
                gain_of_db(background)
            },
            wet: (row(values, AudioVoice::WET, 80.0) / 100.0).clamp(0.0, 1.0),
            background_only: row(values, AudioVoice::BACKGROUND_ONLY, 0.0) >= 0.5,
        }
    }
}

/// What the blocks read: the model's answer and the three levels on their way
/// to what the rows ask.
struct VoiceState {
    heard: Option<Heard>,
    voice: Smoother,
    background: Smoother,
    wet: Smoother,
}

/// One open Voice isolation.
struct VoiceAudio {
    model: Option<Arc<dyn VoiceModel>>,
    rate: u32,
    /// Behind a mutex because `look` and `process` take `&self`; uncontended
    /// by construction, since one instance is driven by one bake.
    state: Mutex<VoiceState>,
}

impl AudioProcessor for VoiceAudio {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        steady: i64,
    ) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let voices = match &state.heard {
            Some(Heard::Voice(voices)) => Arc::clone(voices),
            // The block ships dry and the bake files the sentence.
            Some(Heard::Failed(_)) => return false,
            // No model to ask, or none installed: untouched, and no badge
            // from here, since the missing addon is not a failure.
            _ => {
                output.copy_from_slice(input);
                return true;
            }
        };
        let levels = Levels::of(values);
        state.voice.set_target(levels.voice);
        state.background.set_target(levels.background);
        state.wet.set_target(levels.wet);
        let first = usize::try_from(steady).unwrap_or(0);
        for (j, (i, o)) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
            .enumerate()
        {
            let (l, r) = frame(i);
            let at = first + j;
            let heard = (
                voices.left.get(at).copied().unwrap_or(0.0),
                voices.right.get(at).copied().unwrap_or(0.0),
            );
            let (voice, background, wet) = (
                state.voice.step() as f32,
                state.background.step() as f32,
                state.wet.step() as f32,
            );
            let mixed = |x: f32, v: f32| {
                if levels.background_only {
                    x - v
                } else {
                    let result = voice * v + background * (x - v);
                    x + wet * (result - x)
                }
            };
            put(o, mixed(l, heard.0), mixed(r, heard.1));
        }
        true
    }

    fn bakes_whole(&self) -> bool {
        true
    }

    fn look(&self, input: &[f32], _values: &[Vec<(ParamId, f64)>]) {
        let Some(model) = &self.model else {
            return;
        };
        // Asked with no lock held: the model may take minutes on a long
        // sound, and the blocks are not running yet anyway.
        let heard = model.voices(input, self.rate);
        if let Ok(mut state) = self.state.lock() {
            state.heard = Some(heard);
        }
    }

    fn last_error(&self) -> Option<String> {
        match self.state.lock().ok()?.heard.as_ref()? {
            Heard::Failed(why) => Some(why.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::modulation::harness;
    use super::*;
    use crate::fx::audio_chain::{run_chain, ChainLink, AUDIO_BLOCK_FRAMES};

    /// A stand-in for the model with a known answer: the left channel's voice
    /// is half of it, the right channel's a quarter. Counts how often it was
    /// asked.
    #[derive(Default)]
    struct Halves(Mutex<usize>);

    impl VoiceModel for Halves {
        fn voices(&self, input: &[f32], _rate: u32) -> Heard {
            *self.0.lock().unwrap() += 1;
            let (left, right) = input
                .chunks_exact(AUDIO_CHANNELS)
                .map(|f| (f[0] * 0.5, f[1] * 0.25))
                .unzip();
            Heard::Voice(Arc::new(Voices { left, right }))
        }
    }

    /// One bake of `input` through the effect with `model`, the way the
    /// chain runs it: the look first, then the blocks.
    fn bake(model: Option<Arc<dyn VoiceModel>>, over: &[(&str, f64)], input: &[f32]) -> Vec<f32> {
        let mut values = harness::values(&AudioVoiceDef, over);
        // The switch goes to the bake as nought or one, which the harness
        // leaves out with every other row that is not a number.
        if let Some((_, on)) = over.iter().find(|(id, _)| *id == "background_only") {
            values.push((AudioVoice::BACKGROUND_ONLY, *on));
        }
        let processor = open(&values, harness::RATE, model);
        run_chain(
            &[ChainLink {
                processor,
                values: vec![values],
            }],
            input,
        )
        .samples
    }

    /// **Without a model the sound goes through untouched, and with one the
    /// voice and the background each take their own level, the same twice.**
    /// A machine without the Voice addon must not hear its sound change, and
    /// a bake has to be a function of its input or the preview and the export
    /// part company.
    #[test]
    fn voice_isolation_mixes_what_it_heard_and_passes_through_without_a_model() {
        let input = harness::tone(8, 220.0);
        harness::deterministic(&AudioVoiceDef, &[], &input);
        assert_eq!(bake(None, &[], &input), input, "untouched without a model");

        let model = Arc::new(Halves::default());
        let all = [("wet", 100.0), ("voice", 0.0), ("background", -60.0)];
        let once = bake(Some(model.clone()), &all, &input);
        assert_eq!(
            bake(Some(model.clone()), &all, &input),
            once,
            "the same twice"
        );
        assert_eq!(*model.0.lock().unwrap(), 2, "asked once a bake");
        // Background off and the voice at unity: the voice alone.
        for (got, x) in once.chunks_exact(2).zip(input.chunks_exact(2)) {
            assert!((got[0] - x[0] * 0.5).abs() < 1e-6 && (got[1] - x[1] * 0.25).abs() < 1e-6);
        }

        // The defaults keep a fifth of the original under the voice.
        let default = bake(Some(model.clone()), &[], &input);
        let tail = AUDIO_BLOCK_FRAMES * AUDIO_CHANNELS * 4;
        for (got, x) in default[tail..]
            .chunks_exact(2)
            .zip(input[tail..].chunks_exact(2))
        {
            let want = x[0] + 0.8 * (x[0] * 0.5 - x[0]);
            assert!((got[0] - want).abs() < 1e-5, "{} against {want}", got[0]);
        }

        // Background only is the input less the voice, whatever the rows say.
        let rest = bake(
            Some(model),
            &[("background_only", 1.0), ("voice", 6.0)],
            &input,
        );
        for (got, x) in rest.chunks_exact(2).zip(input.chunks_exact(2)) {
            assert!((got[0] - x[0] * 0.5).abs() < 1e-6 && (got[1] - x[1] * 0.75).abs() < 1e-6);
        }
    }
}
