//! Reverb (docs/impl/audio-effects.md §4): Freeverb, at whatever rate the bake
//! runs at.
//!
//! # In plain terms
//!
//! A room is thousands of reflections arriving too close together to count.
//! Freeverb makes them with eight combs a channel, each a delay fed back
//! through a one pole so its repeats get duller as they get quieter, and four
//! all-passes after them to smear what is left until no single repeat can be
//! picked out. The eight lengths are chosen not to share factors, which is
//! what stops the repeats piling up on each other and ringing on one note.
//!
//! Two things are worth knowing before changing anything here.
//!
//! **The tunings are in samples at 44.1 kHz** and are scaled by the rate the
//! bake opens at, or the room shrinks by a third at 96 kHz
//! (docs/impl/audio-effects.md §8).
//!
//! **The gain staging is folded.** Freeverb's own fixed input gain is 0.015
//! and its wet knob runs to three; here the two are multiplied into one
//! constant so that Wet is an ordinary per cent and a hundred of it is the
//! sound Freeverb calls fully wet.

use std::sync::{Arc, Mutex};

use super::super::dsp::biquad::{Biquad, Coeffs};
use super::super::dsp::db::{db_of_gain, SILENCE_FLOOR_DB};
use super::super::dsp::delay_line::DelayLine;
use super::super::dsp::smoother::Smoother;
use super::{aim, held, value_of};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId};
use lumit_fx_macros::Effect;

/// The rate Freeverb's tunings are counted in samples at.
const TUNED_AT: f64 = 44_100.0;

/// The eight comb lengths, in samples at [`TUNED_AT`].
const COMBS: [f64; 8] = [
    1_116.0, 1_188.0, 1_277.0, 1_356.0, 1_422.0, 1_491.0, 1_557.0, 1_617.0,
];

/// The four all-pass lengths, in samples at [`TUNED_AT`].
const ALL_PASSES: [f64; 4] = [556.0, 441.0, 341.0, 225.0];

/// What the right channel adds to every length, so the two banks are not the
/// same room heard twice.
const SPREAD: f64 = 23.0;

/// The all-passes' fixed feedback. Freeverb's, and not a row: at anything but
/// a half they stop being all-pass and start colouring.
const ALL_PASS_FEEDBACK: f32 = 0.5;

/// Freeverb's fixed input gain times its wet scale, in one number, so Wet can
/// be a plain per cent (see the module's own words).
const INPUT_GAIN: f32 = 0.045;

/// Room size to comb feedback: Freeverb's own scale and offset, so the row's
/// nought is a small hard room and its hundred is a hall.
const ROOM_SCALE: f64 = 0.28;
const ROOM_OFFSET: f64 = 0.7;

/// Damping to the one pole in each comb's feedback. Freeverb's scale: a whole
/// row of damping still lets some top end round the loop.
const DAMPING_SCALE: f64 = 0.4;

/// The longest pre-delay the row offers, in milliseconds.
const MAX_PRE_DELAY_MS: f64 = 200.0;

/// How long a reverb's tail may be, in seconds.
///
/// ponytail: a flat ceiling rather than a measured decay. A hall at the top of
/// the Room size row takes twenty seconds to reach the silence floor and a
/// rate change stretches that; thirty seconds is more than any of it and short
/// enough that a mistake costs a second of render rather than a minute.
const TAIL_CEILING_S: f64 = 30.0;

/// How fast the gains follow their rows.
const GLIDE_MS: f64 = 20.0;

/// A room around the sound: eight combs and four all-passes a channel.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_reverb",
    label = "Reverb",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound in, sound out: no picture to dissolve, and so no matte row
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioReverb {
    /// How long the room holds on to the sound.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub room_size: f32,
    /// How much top end the walls take out of each reflection.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub damping: f32,
    /// How far apart the two channels' rooms are heard. At nought the tail is
    /// mono.
    #[bounded(min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub width: f32,
    /// The gap between the sound and the room answering it, in milliseconds.
    /// This is what keeps a voice in front of its own reverb.
    #[bounded(min = 0.0, max = 200.0, default = 20.0, unit = Raw, label = "Pre-delay")]
    pub pre_delay: f32,
    /// A low pass over the tail alone, for a room with soft walls.
    #[bounded(
        min = 500.0,
        max = 20_000.0,
        default = 12_000.0,
        log = true,
        unit = Raw
    )]
    pub high_cut: f32,
    /// How much of the sound itself reaches the output.
    #[bounded(min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub dry: f32,
    /// How much of the room reaches it.
    #[bounded(min = 0.0, max = 100.0, default = 30.0, unit = Percent)]
    pub wet: f32,
}

/// The reverb's behaviour: sound, and no picture at all.
pub struct AudioReverbDef;

impl EffectDef for AudioReverbDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioReverb as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn open_audio(
        &self,
        _state: Option<Vec<u8>>,
        values: &[(ParamId, f64)],
        rate: u32,
        _offline: bool,
    ) -> Option<Arc<dyn AudioProcessor>> {
        // A bake at no rate has no sound in it, and every length below would
        // scale to nothing.
        if rate == 0 {
            return None;
        }
        Some(Arc::new(AudioReverbProcessor::new(values, f64::from(rate))))
    }
}

/// One damped comb: a ring whose feedback passes a one pole.
struct Comb {
    line: DelayLine,
    /// The read that makes the loop exactly `len` frames long, given that the
    /// line has not been pushed this frame yet.
    read: f64,
    /// The one pole's memory.
    store: f32,
}

impl Comb {
    fn new(len: f64) -> Self {
        let len = len.max(2.0);
        Self {
            line: DelayLine::new(len as usize + 1),
            read: len - 1.0,
            store: 0.0,
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let out = self.line.read_linear(self.read);
        self.store = out * (1.0 - damping) + self.store * damping;
        self.line.push(input + self.store * feedback);
        out
    }
}

/// One all-pass: everything comes back, only later and out of order.
struct AllPass {
    line: DelayLine,
    read: f64,
}

impl AllPass {
    fn new(len: f64) -> Self {
        let len = len.max(2.0);
        Self {
            line: DelayLine::new(len as usize + 1),
            read: len - 1.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let held = self.line.read_linear(self.read);
        self.line.push(input + held * ALL_PASS_FEEDBACK);
        held - input
    }
}

/// One channel's room.
struct Bank {
    combs: [Comb; 8],
    all_passes: [AllPass; 4],
}

impl Bank {
    /// The bank for channel `channel`, its lengths scaled from 44.1 kHz to
    /// `rate` and the right one spread away from the left.
    fn new(rate: f64, channel: usize) -> Self {
        let scale = rate / TUNED_AT;
        let spread = if channel == 0 { 0.0 } else { SPREAD };
        Self {
            combs: std::array::from_fn(|n| {
                Comb::new((COMBS.get(n).copied().unwrap_or(1_116.0) + spread) * scale)
            }),
            all_passes: std::array::from_fn(|n| {
                AllPass::new((ALL_PASSES.get(n).copied().unwrap_or(556.0) + spread) * scale)
            }),
        }
    }

    /// One frame of room, out of one frame of sound.
    fn process(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        // The combs sum, which is what makes the early reflections; the
        // all-passes then run in series, which is what stops any one of them
        // being heard on its own.
        let mut out = 0.0;
        for comb in &mut self.combs {
            out += comb.process(input, feedback, damping);
        }
        for all_pass in &mut self.all_passes {
            out = all_pass.process(out);
        }
        out
    }
}

/// The live reverb.
struct ReverbState {
    bank: [Bank; 2],
    pre: [DelayLine; 2],
    cut: [Biquad; 2],
    /// The comb feedback the Room size row asks for.
    feedback: Smoother,
    damping: Smoother,
    width: Smoother,
    /// The pre-delay, in frames.
    pre_frames: Smoother,
    dry: Smoother,
    wet: Smoother,
    /// What the high cut was last set for, so the coefficients are worked out
    /// only when the row has moved.
    cut_hz: f64,
    primed: bool,
}

/// One open reverb.
pub struct AudioReverbProcessor {
    rate: f64,
    tail: u32,
    state: Mutex<ReverbState>,
}

impl AudioReverbProcessor {
    /// Open one from the values the chain's first block holds.
    fn new(values: &[(ParamId, f64)], rate: f64) -> Self {
        let room = held(value_of(values, AudioReverb::ROOM_SIZE, 50.0), 0.0, 100.0) / 100.0;
        let pre = held(
            value_of(values, AudioReverb::PRE_DELAY, 20.0),
            0.0,
            MAX_PRE_DELAY_MS,
        );
        let longest = (COMBS.last().copied().unwrap_or(1_617.0) + SPREAD) * rate / TUNED_AT;
        // The whole of the Pre-delay row's travel, because a driver may sweep
        // it past what it opened at and a line cannot grow in `process`.
        let pre_line = (MAX_PRE_DELAY_MS * 0.001 * rate) as usize + 4;
        Self {
            rate,
            tail: tail_frames(feedback_of_room(room), longest, pre * 0.001 * rate, rate),
            state: Mutex::new(ReverbState {
                bank: std::array::from_fn(|channel| Bank::new(rate, channel)),
                pre: [DelayLine::new(pre_line), DelayLine::new(pre_line)],
                cut: [Biquad::default(), Biquad::default()],
                feedback: Smoother::new(GLIDE_MS, rate, feedback_of_room(room)),
                damping: Smoother::new(GLIDE_MS, rate, 0.0),
                width: Smoother::new(GLIDE_MS, rate, 1.0),
                pre_frames: Smoother::new(GLIDE_MS, rate, pre * 0.001 * rate),
                dry: Smoother::new(GLIDE_MS, rate, 1.0),
                wet: Smoother::new(GLIDE_MS, rate, 0.0),
                cut_hz: 0.0,
                primed: false,
            }),
        }
    }
}

/// The comb feedback a room of `room` (0 to 1) asks for.
fn feedback_of_room(room: f64) -> f64 {
    room * ROOM_SCALE + ROOM_OFFSET
}

/// Frames between the input stopping and the tail falling under the silence
/// floor.
fn tail_frames(feedback: f64, longest: f64, pre: f64, rate: f64) -> u32 {
    // The longest comb is the slowest to die, so it is the one the tail has
    // to outlast. Damping only makes the real decay quicker, which is the
    // safe side of this estimate to be wrong on.
    let per_pass = db_of_gain(feedback);
    let passes = if per_pass < 0.0 {
        SILENCE_FLOOR_DB / per_pass
    } else {
        1.0
    };
    (longest * passes + pre).min(TAIL_CEILING_S * rate) as u32
}

impl AudioProcessor for AudioReverbProcessor {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let Ok(mut guard) = self.state.lock() else {
            return false;
        };
        let state = &mut *guard;

        // This block's numbers, worked out once at its start.
        let room = held(value_of(values, AudioReverb::ROOM_SIZE, 50.0), 0.0, 100.0) / 100.0;
        let damping = held(value_of(values, AudioReverb::DAMPING, 50.0), 0.0, 100.0) / 100.0;
        let width = held(value_of(values, AudioReverb::WIDTH, 100.0), 0.0, 100.0) / 100.0;
        let pre = held(
            value_of(values, AudioReverb::PRE_DELAY, 20.0),
            0.0,
            MAX_PRE_DELAY_MS,
        );
        let cut = held(
            value_of(values, AudioReverb::HIGH_CUT, 12_000.0),
            500.0,
            20_000.0,
        );
        let dry = held(value_of(values, AudioReverb::DRY, 100.0), 0.0, 100.0) / 100.0;
        let wet = held(value_of(values, AudioReverb::WET, 30.0), 0.0, 100.0) / 100.0;

        aim(&mut state.feedback, feedback_of_room(room), state.primed);
        aim(&mut state.damping, damping * DAMPING_SCALE, state.primed);
        aim(&mut state.width, width, state.primed);
        aim(&mut state.pre_frames, pre * 0.001 * self.rate, state.primed);
        aim(&mut state.dry, dry, state.primed);
        aim(&mut state.wet, wet, state.primed);
        state.primed = true;

        if cut != state.cut_hz {
            // Butterworth, so the cut has no peak of its own to ring on.
            let coeffs = Coeffs::low_pass(cut, std::f64::consts::FRAC_1_SQRT_2, self.rate);
            for filter in &mut state.cut {
                filter.set(coeffs);
            }
            state.cut_hz = cut;
        }

        for (frame_in, frame_out) in input.chunks_exact(2).zip(output.chunks_exact_mut(2)) {
            let ([left_in, right_in], [left_out, right_out]) = (frame_in, frame_out) else {
                continue;
            };
            let feedback = state.feedback.step() as f32;
            let damping = state.damping.step() as f32;
            let width = state.width.step() as f32;
            let pre = state.pre_frames.step();
            let dry = state.dry.step() as f32;
            let wet = state.wet.step() as f32;

            // Pushed before it is read, so a pre-delay of nought is the sound
            // itself rather than a frame of it.
            state.pre[0].push(*left_in);
            state.pre[1].push(*right_in);
            let left_pre = state.pre[0].read_linear(pre);
            let right_pre = state.pre[1].read_linear(pre);

            // Both banks hear the same mono sum, as Freeverb's do; what makes
            // them two rooms is the spread in their lengths.
            let into = (left_pre + right_pre) * INPUT_GAIN;
            let left_room = state.cut[0].process(state.bank[0].process(into, feedback, damping));
            let right_room = state.cut[1].process(state.bank[1].process(into, feedback, damping));

            // Freeverb's width: at one each side hears its own room, at
            // nought both hear the sum of them.
            let near = wet * (width * 0.5 + 0.5);
            let far = wet * ((1.0 - width) * 0.5);
            *left_out = left_room * near + right_room * far + *left_in * dry;
            *right_out = right_room * near + left_room * far + *right_in * dry;
        }
        true
    }

    /// Nought: a room answers late, which is the effect rather than a delay to
    /// compensate for.
    fn latency(&self) -> u32 {
        0
    }

    fn tail(&self) -> u32 {
        self.tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;
    use crate::fx::effects::audio::space::{noise, peak, play, play_all};
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES};

    const RATE: u32 = 48_000;

    /// Plan 1's third clause, and §8's last trap: Freeverb's tunings are in
    /// samples at 44.1 kHz, so a room scaled wrongly shrinks at 96 kHz. The
    /// decay is compared as seconds of sound at all three rates.
    #[test]
    fn the_room_decays_over_the_same_seconds_at_any_bake_rate() {
        harness::same_seconds_at_any_rate(
            &AudioReverbDef,
            &[("dry", 0.0), ("wet", 100.0)],
            1_000.0,
            0.6,
            0.02,
        );
    }

    /// A full set of rows, so a test says only what it cares about. Damping
    /// and the high cut are open where a test measures a decay against the
    /// tail the effect promised, because both make the real decay quicker
    /// than the promise.
    ///
    /// Every test here listens to the room rather than to the sound that went
    /// in, so the dry side is shut and the wet side is fully open.
    fn rows(room: f64, damping: f64, width: f64) -> Vec<(ParamId, f64)> {
        vec![
            (AudioReverb::ROOM_SIZE, room),
            (AudioReverb::DAMPING, damping),
            (AudioReverb::WIDTH, width),
            (AudioReverb::PRE_DELAY, 20.0),
            (AudioReverb::HIGH_CUT, 20_000.0),
            (AudioReverb::DRY, 0.0),
            (AudioReverb::WET, 100.0),
        ]
    }

    fn open(values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        AudioReverbDef
            .open_audio(None, values, RATE, false)
            .unwrap_or_else(|| Arc::new(AudioReverbProcessor::new(values, f64::from(RATE))))
    }

    /// A block of noise, then silence for as long as the caller asks.
    fn burst(blocks: usize) -> Vec<f32> {
        let mut input = vec![0.0; blocks * AUDIO_BLOCK_SAMPLES];
        let sound = noise(AUDIO_BLOCK_FRAMES);
        if let Some(head) = input.get_mut(..sound.len()) {
            head.copy_from_slice(&sound);
        }
        input
    }

    #[test]
    fn the_same_input_twice_is_bit_identical() {
        let values = rows(60.0, 80.0, 100.0);
        let input = noise(AUDIO_BLOCK_FRAMES * 8);
        let first = play_all(open(&values).as_ref(), &input, &values);
        let second = play_all(open(&values).as_ref(), &input, &values);
        assert_eq!(first, second);
    }

    #[test]
    fn one_run_and_two_runs_split_at_a_block_edge_agree() {
        let values = rows(60.0, 80.0, 100.0);
        let input = noise(AUDIO_BLOCK_FRAMES * 8);
        let whole = play_all(open(&values).as_ref(), &input, &values);

        let carried = open(&values);
        let mut split = vec![0.0; input.len()];
        play(carried.as_ref(), &input, &values, 0..3, &mut split);
        play(carried.as_ref(), &input, &values, 3..8, &mut split);
        assert_eq!(whole, split);
    }

    /// Plan 3, the reverb's own: the room keeps sounding after the sound has
    /// stopped, and has stopped itself by the tail it reported.
    #[test]
    fn the_room_rings_on_and_is_quiet_by_the_end_of_its_tail() {
        // The smallest room, so the tail is a second rather than twenty.
        let values = rows(0.0, 100.0, 0.0);
        let processor = open(&values);
        let tail = processor.tail() as usize;
        assert!(
            (40_000..80_000).contains(&tail),
            "a small room's tail is about a second, not {tail} frames"
        );

        let blocks = (tail + AUDIO_BLOCK_FRAMES * 2).div_ceil(AUDIO_BLOCK_FRAMES);
        let input = burst(blocks);
        let out = play_all(processor.as_ref(), &input, &values);

        // Halfway through the tail the room is still answering.
        let middle = peak(
            out.get(tail..tail + AUDIO_BLOCK_SAMPLES)
                .unwrap_or_default(),
        );
        assert!(middle > 1e-4, "the room went quiet at once ({middle})");

        // By the end of it there is nothing left.
        let after = peak(out.get(tail * 2..).unwrap_or_default());
        assert!(after < 1e-5, "the tail was still sounding at {after}");
    }

    /// The other half of the same sanity: a bigger room holds the sound for
    /// longer, which is what Room size is for.
    #[test]
    fn a_bigger_room_holds_the_sound_for_longer() {
        let blocks = 96;
        let input = burst(blocks);
        let late = |room: f64| -> f32 {
            let values = rows(room, 100.0, 0.0);
            let out = play_all(open(&values).as_ref(), &input, &values);
            peak(
                out.get(blocks / 2 * AUDIO_BLOCK_SAMPLES..)
                    .unwrap_or_default(),
            )
        };
        let small = late(0.0);
        let large = late(100.0);
        assert!(large > small * 4.0, "small {small}, large {large}");
    }

    /// Width at nought is one room heard in both ears.
    #[test]
    fn no_width_is_a_mono_tail() {
        let values = rows(50.0, 100.0, 0.0);
        let input = burst(16);
        let out = play_all(open(&values).as_ref(), &input, &values);
        let spread = out.chunks_exact(2).fold(0.0f32, |top, frame| match frame {
            [left, right] => top.max((left - right).abs()),
            _ => top,
        });
        assert!(spread < 1e-6, "the two channels differed by {spread}");
    }
}
