//! The jog: a scrub that plays at the speed of the drag, its pitch following
//! the speed as tape's does, backwards when the drag goes back.
//!
//! Each pointer move hands the callback a target. The callback keeps its own
//! position in the mix, in fractional frames, and a speed that eases toward
//! whatever would reach the target in one buffer. It mixes the stretch the
//! buffer crosses and reads it at that speed. A drag that comes to rest goes
//! quiet, and so does letting go.
//!
//! [`Jog::fill`] runs on the audio callback, under its rules: no allocation,
//! no locks, no waiting. The mix is read into a buffer the callback's
//! [`MixWork`] made when the stream opened. The clock does not move.

use crate::block::{DrySource, MixWork, SCRATCH_FRAMES};
use crate::plan::MixPlan;
use crate::{put, AudioEngine, Shared, SCRUB_FADE_FRAMES};
use std::sync::atomic::Ordering;

/// The stored target when no jog is asked for.
pub(crate) const NO_JOG: u64 = u64::MAX;

/// The jog's buffer, in frames: one chunk read at the fastest speed, and the
/// frames either side its last one is read between.
pub(crate) const JOG_FRAMES: usize = FASTEST as usize * SCRATCH_FRAMES + 3;

/// The fastest the jog plays, either way, as a multiple of the mix's speed.
const FASTEST: f64 = 4.0;

/// How long the speed takes to follow the drag, in seconds.
const EASE_S: f64 = 0.03;

/// Slower than this, for [`QUIET_S`], and the jog goes quiet.
const RESTING: f64 = 0.02;
const QUIET_S: f64 = 0.06;

impl AudioEngine {
    /// Play the mix toward `target_seconds` at the speed of the drag: called
    /// on every pointer move while scrubbing. The pitch follows the speed, up
    /// to four times either way. The clock does not move, and nothing is
    /// asked during playback.
    pub fn jog(&self, target_seconds: f64) {
        if self.is_playing() {
            return;
        }
        let frames = target_seconds.max(0.0) * f64::from(self.device_rate);
        self.shared.jog.store(frames.to_bits(), Ordering::Relaxed);
        self.wake();
    }

    /// The drag is over: the jog fades out.
    pub fn jog_end(&self) {
        self.shared.jog.store(NO_JOG, Ordering::Relaxed);
    }
}

impl Shared {
    /// Where the jog is asked to go, in frames, or None.
    pub(crate) fn jog_target(&self) -> Option<f64> {
        let bits = self.jog.load(Ordering::Relaxed);
        (bits != NO_JOG).then(|| f64::from_bits(bits))
    }
}

/// The jog as the callback keeps it between buffers.
#[derive(Debug, Default)]
pub(crate) struct Jog {
    /// Under way: a target has been seen and the sound has not yet faded
    /// out after it went.
    active: bool,
    /// Where in the mix it is, in frames.
    position: f64,
    /// Mix frames per frame heard: one is the mix's own speed.
    speed: f64,
    /// How many frames it has been slower than [`RESTING`].
    still: usize,
    /// How loud it is now, rising and falling over [`SCRUB_FADE_FRAMES`].
    level: f32,
}

impl Jog {
    /// Whether there is anything for [`Self::fill`] to do.
    pub(crate) fn wanted(&self, target: Option<f64>) -> bool {
        self.active || target.is_some()
    }

    /// Add the jog's sound for the buffer into `out` (`channels` to a frame),
    /// heading for `target`. Answers how many frames of some clip were due
    /// and not in memory.
    pub(crate) fn fill(
        &mut self,
        plan: &MixPlan,
        target: Option<f64>,
        work: &mut MixWork,
        out: &mut [f32],
        channels: usize,
    ) -> usize {
        let total = plan.total_frames as f64;
        let rate = f64::from(work.rate().max(1));
        let quiet_after = (QUIET_S * rate) as usize;
        // Gone quiet at rest after sounding is as good as over: the next
        // target, a drag or a shuttle after one that stopped, starts afresh
        // where it is. A fresh start rests at `usize::MAX` until it moves.
        let over = self.level <= 0.0 && (quiet_after..usize::MAX).contains(&self.still);
        match target {
            Some(t) if !self.active || over => {
                // A new drag starts where it is asked to, at rest and quiet.
                *self = Self {
                    active: true,
                    position: t.clamp(0.0, total),
                    still: usize::MAX,
                    ..Self::default()
                };
            }
            None if !self.active => return 0,
            _ => {}
        }
        if channels == 0 {
            return 0;
        }
        let mut buffer = work.lend_jog();
        let mut missed = 0;
        for chunk in out.chunks_mut(SCRATCH_FRAMES * channels) {
            let frames = (chunk.len() / channels) as f64;
            if let Some(t) = target {
                let wanted = (t - self.position) / frames.max(1.0);
                let ease = 1.0 - (-frames / (EASE_S * rate)).exp();
                self.speed = (self.speed + (wanted - self.speed) * ease).clamp(-FASTEST, FASTEST);
            }
            if self.speed.abs() < RESTING {
                self.still = self.still.saturating_add(chunk.len() / channels);
            } else {
                self.still = 0;
            }
            let sounding = target.is_some() && self.still < quiet_after;
            if sounding || self.level > 0.0 {
                missed += self.sound(plan, work, &mut buffer, chunk, channels, sounding);
            }
            self.position = (self.position + frames * self.speed).clamp(0.0, total);
        }
        work.return_jog(buffer);
        if target.is_none() && self.level <= 0.0 {
            *self = Self::default();
        }
        missed
    }

    /// Mix the stretch `chunk` crosses into `buffer` and add it into `chunk`
    /// read at the speed, rising toward full level while `sounding` and
    /// falling toward nothing otherwise.
    fn sound(
        &mut self,
        plan: &MixPlan,
        work: &mut MixWork,
        buffer: &mut [f32],
        chunk: &mut [f32],
        channels: usize,
        sounding: bool,
    ) -> usize {
        let span = (chunk.len() / channels) as f64 * self.speed;
        let first = self.position.min(self.position + span).floor().max(0.0) as usize;
        let count = (span.abs().ceil() as usize + 2)
            .min(plan.total_frames.saturating_sub(first))
            .min(buffer.len() / 2);
        let Some(mixed) = buffer.get_mut(..count * 2) else {
            return 0;
        };
        let missed = plan.mix_block(first, mixed, work, &mut DrySource, None, None);
        let step = 1.0 / SCRUB_FADE_FRAMES as f32;
        for (k, frame) in chunk.chunks_exact_mut(channels).enumerate() {
            self.level = if sounding {
                (self.level + step).min(1.0)
            } else {
                (self.level - step).max(0.0)
            };
            let x = self.position + k as f64 * self.speed - first as f64;
            if x < 0.0 {
                continue;
            }
            let i = x as usize;
            let f = (x - i as f64) as f32;
            let Some(a) = mixed.get(i * 2..i * 2 + 2) else {
                continue;
            };
            let b = mixed.get(i * 2 + 2..i * 2 + 4).unwrap_or(a);
            put(
                frame,
                a[0] + (b[0] - a[0]) * f,
                a[1] + (b[1] - a[1]) * f,
                Some(self.level),
            );
        }
        missed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::{block::MixWork, fill, plan_of, Scrub, Shared};
    use lumit_media::audio::AudioBuffer;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// A drag moving at twice the mix's speed plays a tone an octave up,
    /// with the clock left where it is, and a drag that comes to rest goes
    /// quiet.
    #[test]
    fn a_jog_at_twice_speed_plays_an_octave_up_and_rests_quiet() {
        let (rate, hz) = (48_000u32, 300.0f32);
        let samples: Vec<f32> = (0..rate as usize * 4)
            .flat_map(|n| {
                let v = (std::f32::consts::TAU * hz * n as f32 / rate as f32).sin() * 0.5;
                [v, v]
            })
            .collect();
        let shared = Shared::new(Some(plan_of(Arc::new(AudioBuffer { rate, samples }))));
        let (mut scrub, mut work) = (Scrub::default(), MixWork::new(rate));
        let mut out = vec![0.0f32; 480 * 2];
        let mut target = rate as f64;
        let mut left = Vec::new();
        for n in 0..60 {
            shared.jog.store(target.to_bits(), Ordering::Relaxed);
            fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
            target += 960.0;
            if n >= 30 {
                left.extend(out.chunks_exact(2).map(|f| f[0]));
            }
        }
        let rising = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        let pitch = rising as f32 * rate as f32 / left.len() as f32;
        assert!((pitch - 2.0 * hz).abs() < 0.03 * hz, "heard {pitch} Hz");
        assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);

        // The target stops moving: the jog slows, rests and goes quiet.
        for _ in 0..80 {
            fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
        }
        for _ in 0..5 {
            fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
            assert!(out.iter().all(|s| *s == 0.0), "a resting jog is quiet");
        }
    }
}
