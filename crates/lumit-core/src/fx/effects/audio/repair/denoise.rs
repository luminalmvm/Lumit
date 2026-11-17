//! Noise reduction: take a steady hiss, rumble or whine out from under the
//! sound.
//!
//! # In plain terms
//!
//! The sound is cut into 43 ms frames a quarter apart, and in each frame every
//! frequency is turned down by how much of it the noise floor explains. The
//! floor comes from one of two places. Learn listens to a stretch of noise on
//! its own and keeps what it heard in the instance ([`learn_profile`]); with
//! nothing learned, or with Adaptive on, the effect works its own floor out
//! from the whole of its input before the first block
//! ([`super::profile::adaptive_floor`]).
//!
//! Threshold moves the floor up or down before the gain is worked out, so a
//! few dB more treats more of the sound as noise. Reduction is the deepest
//! any frequency is turned down, and Tonal reduction goes further on steady
//! peaks in the floor, a whine or a buzz, which a broadband floor would leave
//! standing. Smoothing trades the watery twinkle of a fast gain for a slower,
//! softer one ([`SpectralGain`]).
//!
//! Noise only plays what was taken away. The effect runs a frame less one
//! sample late, which the chain compensates, and adds no tail.

use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;
use realfft::num_complex::Complex;

use super::profile::{
    adaptive_floor, encode, floor_of_blob, learned_points, tonal_excess, Framing, TONAL_DB,
};
use super::{on, row};
use crate::fx::audio_chain::{AudioProcessor, Denormals, AUDIO_CHANNELS};
use crate::fx::effects::audio::dsp::spectral::SpectralGain;
use crate::fx::effects::audio::dsp::stft::{Fft, Framer};
use crate::fx::{
    params::ParamId, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};

const THRESHOLD_DB: f64 = 0.0;
const REDUCTION_DB: f64 = 12.0;
const SMOOTHING: f64 = 50.0;
const TONAL_REDUCTION_DB: f64 = 0.0;

/// Noise reduction's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_denoise",
    label = "Noise reduction",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioDenoise {
    /// Learn the noise: from the work area where it narrows the layer down,
    /// or else from the quietest stretch near the playhead. The bridge reads
    /// the sound, calls [`learn_profile`] and keeps the answer as the
    /// instance's state.
    #[action(label = "Learn")]
    pub learn: (),
    /// Work the floor out from the whole input instead of a learned profile.
    /// On until something is learned.
    #[toggle(default = true, label = "Adaptive")]
    pub adaptive: bool,
    /// How far the floor is raised before the gain is worked out, in dB.
    #[bounded(min = -10.0, max = 20.0, default = THRESHOLD_DB, unit = Raw)]
    pub threshold: f32,
    /// The deepest any frequency is turned down, in dB.
    #[bounded(min = 0.0, max = 40.0, default = REDUCTION_DB, unit = Raw)]
    pub reduction: f32,
    /// How slowly and how widely the gain moves, which is what keeps the
    /// residue from twinkling.
    #[bounded(min = 0.0, max = 100.0, default = SMOOTHING, unit = Percent)]
    pub smoothing: f32,
    /// How much further steady peaks in the floor are turned down, in dB.
    #[bounded(
        min = 0.0,
        max = 40.0,
        default = TONAL_REDUCTION_DB,
        unit = Raw,
        label = "Tonal reduction"
    )]
    pub tonal: f32,
    /// Play only what was taken away.
    #[toggle(default = false, label = "Noise only")]
    pub noise_only: bool,
}

/// Learn a noise profile from `interleaved` stereo at `rate`: the mean power
/// at each frequency over the whole of it, as the blob Noise reduction keeps
/// in its instance's plugin_state (about a kilobyte; see
/// [`super::profile`] for the layout).
///
/// The profile is a density on a fixed frequency grid, so it can be learned
/// at one rate and played at another. Digital silence is skipped; a run with
/// no sound in it at all learns a floor so low nothing is removed.
#[must_use]
pub fn learn_profile(interleaved: &[f32], rate: u32) -> Vec<u8> {
    encode(&learned_points(interleaved, AUDIO_CHANNELS, rate))
}

/// Noise reduction's behaviour.
pub struct AudioDenoiseDef;

impl EffectDef for AudioDenoiseDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDenoise as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn open_audio(
        &self,
        state: Option<Vec<u8>>,
        values: &[(ParamId, f64)],
        rate: u32,
        _offline: bool,
    ) -> Option<Arc<dyn AudioProcessor>> {
        Some(Arc::new(Denoise::new(state.as_deref(), values, rate)))
    }
}

/// The floor and the gain rule one channel uses.
pub(super) struct Channel {
    /// Each bin's expected noise power, or empty for "not known", which
    /// passes the sound through.
    pub floor: Vec<f32>,
    /// How far each bin of the floor stands over its neighbours, in dB.
    pub excess: Vec<f32>,
    /// The lowest gain per bin, from Reduction and Tonal reduction.
    pub lows: Vec<f32>,
    pub gain: SpectralGain,
}

impl Channel {
    pub(super) fn new(framing: &Framing) -> Self {
        Self {
            floor: Vec::new(),
            excess: Vec::new(),
            lows: vec![1.0; framing.bins],
            gain: SpectralGain::new(framing.bins, framing.bin_hz, framing.hop_s()),
        }
    }

    /// Take a floor in each bin's expected power.
    pub(super) fn set_floor(&mut self, floor: &[f64]) {
        self.excess = tonal_excess(floor);
        self.floor = floor.iter().map(|&p| p as f32).collect();
    }

    /// The lowest gain per bin: Reduction everywhere, and Tonal reduction on
    /// top where the floor has a steady peak.
    pub(super) fn set_lows(&mut self, reduction_db: f64, tonal_db: f64) {
        for (k, low) in self.lows.iter_mut().enumerate() {
            let excess = f64::from(self.excess.get(k).copied().unwrap_or(0.0));
            let extra = if excess > TONAL_DB {
                excess.min(tonal_db)
            } else {
                0.0
            };
            *low = 10f64.powf(-(reduction_db + extra) / 20.0) as f32;
        }
    }
}

/// One open Noise reduction.
struct Denoise {
    state: Mutex<State>,
    latency: u32,
    /// A learned profile is in use, so the look has nothing to learn and
    /// the rack need not be baked whole.
    learned: bool,
}

struct State {
    framing: Framing,
    framer: Framer,
    fft: Fft,
    spectrum: Vec<Complex<f32>>,
    power: Vec<f32>,
    noise: Vec<f32>,
    channels: Vec<Channel>,
    /// The rows the lows were last worked out for.
    lows_for: (f64, f64),
}

impl Denoise {
    fn new(state: Option<&[u8]>, values: &[(ParamId, f64)], rate: u32) -> Self {
        let framing = Framing::at(rate);
        let framer = Framer::sqrt_hann(framing.n, AUDIO_CHANNELS);
        let latency = framer.latency() as u32;
        let mut channels: Vec<Channel> = (0..AUDIO_CHANNELS)
            .map(|_| Channel::new(&framing))
            .collect();
        let adaptive = on(values, AudioDenoise::ADAPTIVE, true);
        let profile = state.and_then(|blob| floor_of_blob(blob, &framing));
        let learned = !adaptive && profile.is_some();
        if let (true, Some(floor)) = (learned, &profile) {
            for channel in &mut channels {
                channel.set_floor(floor);
            }
        }
        let fft = Fft::new(framing.n);
        Self {
            state: Mutex::new(State {
                spectrum: fft.spectrum(),
                power: vec![0.0; framing.bins],
                noise: vec![0.0; framing.bins],
                fft,
                framer,
                channels,
                lows_for: (f64::NAN, f64::NAN),
                framing,
            }),
            latency,
            learned,
        }
    }
}

impl AudioProcessor for Denoise {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let mut held = self.state.lock();
        let State {
            framer,
            fft,
            spectrum,
            power,
            noise,
            channels,
            lows_for,
            ..
        } = &mut *held;

        let threshold = row(values, AudioDenoise::THRESHOLD, THRESHOLD_DB).clamp(-10.0, 20.0);
        let reduction = row(values, AudioDenoise::REDUCTION, REDUCTION_DB).clamp(0.0, 40.0);
        let smoothing = row(values, AudioDenoise::SMOOTHING, SMOOTHING);
        let tonal = row(values, AudioDenoise::TONAL, TONAL_REDUCTION_DB).clamp(0.0, 40.0);
        let noise_only = on(values, AudioDenoise::NOISE_ONLY, false);
        if *lows_for != (reduction, tonal) {
            *lows_for = (reduction, tonal);
            for channel in channels.iter_mut() {
                channel.set_lows(reduction, tonal);
            }
        }
        let raise = 10f64.powf(threshold / 10.0) as f32;

        framer.process(input, output, |_, frames| {
            for (frame, channel) in frames.iter_mut().zip(channels.iter_mut()) {
                if channel.floor.is_empty() {
                    if noise_only {
                        frame.fill(0.0);
                    }
                    continue;
                }
                channel.gain.set_smoothing(smoothing);
                fft.forward(frame, spectrum);
                for (p, x) in power.iter_mut().zip(spectrum.iter()) {
                    *p = x.norm_sqr();
                }
                for (n, f) in noise.iter_mut().zip(&channel.floor) {
                    *n = f * raise;
                }
                let gains = channel.gain.gains(power, noise, &channel.lows);
                for (x, &g) in spectrum.iter_mut().zip(gains) {
                    *x *= if noise_only { 1.0 - g } else { g };
                }
                fft.inverse(spectrum, frame);
            }
        });
        true
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn bakes_whole(&self) -> bool {
        !self.learned
    }

    fn look(&self, input: &[f32], _values: &[Vec<(ParamId, f64)>]) {
        if self.learned {
            return;
        }
        let mut held = self.state.lock();
        let framing = &held.framing;
        let floors: Vec<Option<Vec<f64>>> = {
            use rayon::prelude::*;
            (0..AUDIO_CHANNELS)
                .into_par_iter()
                .map(|c| {
                    let _flush = Denormals::on();
                    adaptive_floor(input, AUDIO_CHANNELS, c, framing)
                })
                .collect()
        };
        for (channel, floor) in held.channels.iter_mut().zip(floors) {
            if let Some(floor) = floor {
                channel.set_floor(&floor);
            }
        }
        held.lows_for = (f64::NAN, f64::NAN);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;

    /// White noise from a fixed seed, so every run hears the same hiss.
    pub(crate) fn hiss(frames: usize, amp: f32, seed: u64) -> Vec<f32> {
        let mut s = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (0..frames * 2)
            .map(|_| {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((s >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn it_is_deterministic_and_the_same_at_any_rate() {
        let mut input = harness::tone(64, 440.0);
        for (x, n) in input.iter_mut().zip(hiss(64 * 512, 0.05, 3)) {
            *x += n;
        }
        harness::deterministic(&AudioDenoiseDef, &[], &input);
        harness::same_seconds_at_any_rate(&AudioDenoiseDef, &[], 440.0, 1.0, 0.03);
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / x.len().max(1) as f64)
            .sqrt()
    }

    /// The level of a tone at `hz` in the left channel.
    fn level(x: &[f32], hz: f64) -> f64 {
        let step = std::f64::consts::TAU * hz / 48_000.0;
        let (mut re, mut im) = (0.0, 0.0);
        let frames = x.len() / 2;
        for n in 0..frames {
            re += f64::from(x[n * 2]) * (step * n as f64).cos();
            im += f64::from(x[n * 2]) * (step * n as f64).sin();
        }
        2.0 * re.hypot(im) / frames as f64
    }

    /// **Plan 3**: after Learn on two seconds of hiss, the hiss alone comes
    /// down by the Reduction and a tone over it stays within a dB.
    #[test]
    fn it_takes_the_hiss_down_and_keeps_the_tone() {
        let rate = 48_000;
        let frames = 512 * 400;
        let noise = hiss(frames, 0.02, 9);
        let profile = learn_profile(&noise[..96_000 * 2], rate);
        assert!(profile.len() < 2_048);
        let tone = harness::tone(400, 1_000.0);
        let input: Vec<f32> = noise.iter().zip(&tone).map(|(n, t)| n + t).collect();

        let values = vec![(AudioDenoise::ADAPTIVE, 0.0)];
        let p = AudioDenoiseDef
            .open_audio(Some(profile), &values, rate, false)
            .unwrap();
        p.look(&input, std::slice::from_ref(&values));
        let lat = p.latency() as usize;
        let tone_out = harness::run(&*p, &input, &values);
        let p = AudioDenoiseDef
            .open_audio(Some(learn_profile(&noise, rate)), &values, rate, false)
            .unwrap();
        let hiss_out = harness::run(&*p, &noise, &values);

        // From the first sample, and the first tenth of a second on its own:
        // the gain starts closed, so a clip does not open with a puff of its
        // noise.
        let lag = lat * 2;
        for to in [4_800 * 2, noise.len() - lag] {
            let down = 20.0 * (rms(&noise[..to]) / rms(&hiss_out[lag..to + lag])).log10();
            assert!(
                (9.5..14.0).contains(&down),
                "the first {to} samples of hiss came down {down} dB, not about 12"
            );
        }
        let tone_in = level(&tone[..96_000 * 2], 1_000.0);
        let tone_kept = level(&tone_out[lag..lag + 96_000 * 2], 1_000.0);
        let change = 20.0 * (tone_kept / tone_in).log10();
        assert!(change.abs() < 1.0, "the tone moved {change} dB");
    }

    /// A profile means the same noise at every rate: learned at 44.1 or
    /// 48 kHz and read back at either, each bin's floor is the power the noise
    /// really has there.
    #[test]
    fn a_profile_learned_at_one_rate_reads_back_at_another() {
        for learn_rate in [44_100, 48_000] {
            let noise = hiss(learn_rate as usize * 2, 0.1, 5);
            let blob = learn_profile(&noise, learn_rate);
            for play_rate in [44_100, 48_000] {
                let framing = Framing::at(play_rate);
                let floor = floor_of_blob(&blob, &framing).unwrap();
                // White noise uniform in +-0.1 has a variance of 0.01 / 3, and
                // each bin's expected power is that times the window energy.
                let energy: f64 = framing.window.iter().map(|w| f64::from(w * w)).sum();
                let want = 0.01 / 3.0 * energy;
                for k in [40, 200, 600] {
                    let db = 10.0 * (floor[k] / want).log10();
                    assert!(
                        db.abs() < 1.0,
                        "{learn_rate} to {play_rate}: bin {k} is {db} dB out"
                    );
                }
            }
        }
    }
}
