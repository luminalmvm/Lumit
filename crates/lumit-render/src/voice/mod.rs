//! Voice isolation's model half: what finds the voice for the
//! `audio_voice` effect, and what the interface asks about the Voice addon.
//!
//! # In plain terms
//!
//! The effect's rows and its mix live in `lumit-core`, which knows nothing
//! about addons. When a chain is opened, [`open_audio`] builds the effect with this
//! side's model behind it. The model hears the whole of the link's input once,
//! before the first block, at 48 kHz whatever the bake's rate, and its answer
//! is kept: the same sound baked again, at the same rate with the same pack,
//! is handed the same answer without the model running at all. Moving a row
//! bakes again and costs only the mix.
//!
//! Without the runtime or the pack the effect passes its sound through and
//! [`addon_missing`] names what to install, the way Depth and Remove
//! background do. Installing moves the addons folder's generation, so the
//! next bake finds the pack without a restart.
//!
//! [`voices`] is the one call into the model, so it could move to an analysis
//! step and a sidecar without the effect noticing.
//!
//! # Thread role and contract
//!
//! The bake's thread: an export's, or the mixer's plan worker. The model is
//! opened, run and dropped inside one call and never shared
//! (14-ENGINEERING-RULES §1.3). It runs on the processor with a fixed thread
//! count, so its answer is bit-stable and a cached one is the answer a fresh
//! run would give (§3). An export's cancel stops it between minutes of sound.
//! The kept answers are a byte-budgeted store under the governor.

mod resample;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use lumit_budget::Tier;
use lumit_cache::{ByteLru, ByteSized};
use lumit_core::fx::audio_chain::{AudioProcessor, AUDIO_CHANNELS};
use lumit_core::fx::effects::audio::voice::{self as effect, Heard, VoiceModel, Voices};
use lumit_core::fx::params::ParamId;
use lumit_core::fx::registry::EffectDef;
use lumit_core::model::EffectInstance;
use lumit_ml::{error::MlError, manifest::Task, voice::Voice};

/// The most bytes of kept answers. Thirty minutes of stereo at 48 kHz is
/// 691 MB, and that is the longest sound worth keeping whole: anything longer
/// runs again.
const KEPT_BYTES: usize = 768 << 20;

/// The longest input the model is run on, in interleaved samples: an hour of
/// stereo at 48 kHz, 1.4 GB. The bake holds about four copies of its sound
/// at once (the caller's, the chain's two and the answer), and an hour
/// measured 5.3 GB at its peak, about what a 16 GB machine can spare beside
/// everything else. Longer, and the effect passes its sound through and says
/// why.
const MOST_SAMPLES: usize = 3600 * 48_000 * AUDIO_CHANNELS;

/// What the badge says beneath an input longer than [`MOST_SAMPLES`].
pub const TOO_LONG: &str = "Too long for Voice isolation. Split the layer.";

/// One kept answer.
struct Kept(Arc<Voices>);

impl ByteSized for Kept {
    fn byte_size(&self) -> usize {
        (self.0.left.len() + self.0.right.len()) * std::mem::size_of::<f32>()
    }
}

/// Answers by the blake3 of the input, the rate and the pack. Evicts by the
/// governor's pressure and its own budget.
static KEPT: Mutex<Option<ByteLru<[u8; 32], Kept>>> = Mutex::new(None);

/// Count kept answers against the governor's `ledger` from here on. The
/// answers already kept stay: every renderer built calls this, an export's
/// among them, and the preview should not have to run the model again
/// because an export happened.
pub(crate) fn account_against(ledger: &Arc<lumit_budget::Ledger>) {
    if let Ok(mut slot) = KEPT.lock() {
        slot.get_or_insert_with(|| ByteLru::new(KEPT_BYTES))
            .account_against(Arc::clone(ledger), Tier::Ram);
    }
}

/// The model, as the effect is handed it.
struct Model;

impl VoiceModel for Model {
    fn voices(&self, input: &[f32], rate: u32) -> Heard {
        voices(input, rate)
    }
}

/// What a chain opens for `instance`: Voice isolation with the model behind
/// it, in place of the definition's own `open_audio`, which has none, and
/// anything else as its definition opens it.
pub(crate) fn open_audio(
    instance: &EffectInstance,
    def: &dyn EffectDef,
    first: &[(ParamId, f64)],
    rate: u32,
    offline: bool,
) -> Option<Arc<dyn AudioProcessor>> {
    if instance.effect.match_name == effect::MATCH_NAME {
        Some(effect::open(first, rate, Some(Arc::new(Model))))
    } else {
        def.open_audio(instance.plugin_state_bytes(), first, rate, offline)
    }
}

/// What the installed Voice addon is, or `None` when there is none or no
/// runtime to run it. Read off the installed-packs snapshot, so it is cheap
/// enough to ask on every rebuild; it changes whenever the addons folder does.
#[must_use]
pub fn installed() -> Option<[u8; 32]> {
    lumit_ml::store::installed_identity(Task::Voice)
}

/// [`installed`] folded to a word, for a mix's signature. Voice isolation
/// passes its sound through until the addon arrives, so a mix holding one
/// sounds different after an install though nothing in its jobs has moved.
#[must_use]
pub fn stamp() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    installed().hash(&mut h);
    h.finish()
}

/// What a Voice isolation instance is missing, as the badge's detail slot
/// wants it, or `None` when the machine can run its model or `fx` is some
/// other effect. The runtime by its id, else the pack by its catalogue id.
#[must_use]
pub fn addon_missing(fx: &EffectInstance) -> Option<String> {
    if fx.effect.match_name != effect::MATCH_NAME {
        return None;
    }
    if matches!(
        lumit_ml::runtime::status(),
        lumit_ml::runtime::RuntimeStatus::Missing
    ) {
        return Some(lumit_ml::runtime::RUNTIME_ID.to_owned());
    }
    installed().is_none().then(|| effect::PACK.to_owned())
}

/// The voice in `input`, interleaved stereo at `rate`: the one call into the
/// model, kept by what it was asked.
fn voices(input: &[f32], rate: u32) -> Heard {
    let Some(pack) = installed() else {
        return Heard::Missing;
    };
    if input.len() > MOST_SAMPLES {
        return Heard::Failed(TOO_LONG.to_owned());
    }
    let key = key(input, rate, &pack);
    let found = KEPT.lock().ok().and_then(|mut slot| {
        let kept = slot.get_or_insert_with(|| ByteLru::new(KEPT_BYTES));
        kept.get(&key).map(|k| Arc::clone(&k.0))
    });
    if let Some(found) = found {
        return Heard::Voice(found);
    }
    let stop = crate::model::stop_flag().unwrap_or_default();
    match run(input, rate, &stop) {
        Ok(made) => {
            let made = Arc::new(made);
            if let Ok(mut slot) = KEPT.lock() {
                let kept = slot.get_or_insert_with(|| ByteLru::new(KEPT_BYTES));
                if kept
                    .pressure()
                    .is_some_and(lumit_budget::Pressure::should_trim)
                {
                    kept.evict_cold_half();
                }
                kept.insert(key, Kept(Arc::clone(&made)));
            }
            Heard::Voice(made)
        }
        Err(MlError::Cancelled) => Heard::Stopped,
        Err(MlError::RuntimeMissing | MlError::PackMissing(_)) => Heard::Missing,
        Err(other) => Heard::Failed(other.to_string()),
    }
}

/// What an answer is filed under: the samples themselves, the rate they are
/// at and the pack that heard them.
fn key(input: &[f32], rate: u32, pack: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"lumit-voice-1");
    hasher.update(pack);
    hasher.update(&rate.to_le_bytes());
    let mut bytes = [0u8; 1 << 16];
    for chunk in input.chunks(bytes.len() / 4) {
        let mut used = 0;
        for (slot, x) in bytes.chunks_exact_mut(4).zip(chunk) {
            slot.copy_from_slice(&x.to_le_bytes());
            used += 4;
        }
        hasher.update(bytes.get(..used).unwrap_or(&[]));
    }
    *hasher.finalize().as_bytes()
}

/// Open the pack and run it over both channels at the model's rate, then
/// bring the answer back to `rate` at the input's own length.
///
/// One copy of the sound is held here at a time: the model writes its answer
/// over the channels it is handed, and on the way back each channel at the
/// model's rate is let go as soon as its copy at `rate` is made.
fn run(input: &[f32], rate: u32, stop: &AtomicBool) -> Result<Voices, MlError> {
    let frames = input.len() / AUDIO_CHANNELS;
    let model_rate = lumit_ml::voice::RATE;
    let at_model = |channel: usize| -> Vec<f32> {
        let side: Vec<f32> = input
            .chunks_exact(AUDIO_CHANNELS)
            .map(|f| f.get(channel).copied().unwrap_or(0.0))
            .collect();
        if rate == model_rate {
            return side;
        }
        let len = (frames as u64 * u64::from(model_rate)).div_ceil(u64::from(rate.max(1)));
        resample::resample(&side, rate, model_rate, len as usize)
    };
    let mut heard = Voice::open()?.isolate(vec![at_model(0), at_model(1)], stop)?;
    let back = |channel: Option<Vec<f32>>| -> Vec<f32> {
        let mut channel = channel.unwrap_or_default();
        if rate == model_rate {
            channel.resize(frames, 0.0);
            channel
        } else {
            resample::resample(&channel, model_rate, rate, frames)
        }
    };
    let right = back(heard.pop());
    let left = back(heard.pop());
    Ok(Voices { left, right })
}
