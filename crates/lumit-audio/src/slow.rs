//! The mix played slower than it was made, with its pitch kept: the Viewer's
//! playback speed.
//!
//! The same grains as [`crate::speed::keep_pitch`], 40 ms long, half
//! overlapped under a Hann window and each nudged by up to 10 ms to line up
//! with the last. That one works on a whole slice at once. This one lays a
//! grain each time the sound card has used up the last, reading the live mix
//! around where it is wanted, so nothing is baked and an edit is still heard
//! on the next grain.
//!
//! The playhead stays in the mix's own frames, so the clock reads comp time
//! and only runs slower.

use crate::{meter, mix::MixPlan, nanos, AudioEngine, Scrub, Shared};
use std::sync::atomic::Ordering;

/// The slowest the sound is played at: an eighth of its speed.
const SLOWEST: u32 = 8;

impl AudioEngine {
    /// Play `slow` times slower than the mix, its pitch kept. One is the
    /// mix's own speed. Heard from the next buffer, and the clock follows.
    pub fn set_slow(&self, slow: u32) {
        self.shared
            .slow
            .store(slow.clamp(1, SLOWEST), Ordering::Relaxed);
    }
}

/// What the callback keeps between buffers while the sound is slowed. Every
/// buffer is made when the stream opens, so the callback never allocates.
pub(crate) struct Stretch {
    /// The Hann window over one grain.
    window: Vec<f32>,
    /// Half a grain: how far apart grains are laid in what is heard.
    hop: usize,
    /// How far a grain may be moved to line up with the last.
    seek: usize,
    /// The mix around where the next grain is wanted: `seek` frames either
    /// side of it and a grain on, stereo.
    span: Vec<f32>,
    /// The last grain's second half, windowed, for the next to be laid over.
    tail: Vec<f32>,
    /// The same half as it was mixed, in mono: what the next grain is
    /// matched against.
    carry: Vec<f32>,
    /// The half grain being handed to the card, and how much of it has gone.
    ready: Vec<f32>,
    given: usize,
    /// Where in the mix this run began, and how many frames have been heard
    /// since. Counted rather than added up, so the clock never drifts.
    base: usize,
    heard: u64,
    slow: u32,
    /// Where the last buffer left the playhead. A playhead found anywhere
    /// else has been moved, and the grains start again from there.
    left_at: Option<usize>,
    /// Whether `tail` and `carry` hold a grain to carry on from.
    joined: bool,
}

impl Stretch {
    /// Sized for a stream at `rate`, with the grain [`crate::speed::keep_pitch`]
    /// uses.
    pub(crate) fn new(rate: u32) -> Self {
        let grain = ((rate as usize / 25) & !1).max(64);
        let hop = grain / 2;
        let seek = (rate as usize / 100).max(1);
        let window = (0..grain)
            .map(|j| {
                let x = std::f32::consts::TAU * j as f32 / grain as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        Self {
            window,
            hop,
            seek,
            span: vec![0.0; (2 * seek + grain) * 2],
            tail: vec![0.0; hop * 2],
            carry: vec![0.0; hop],
            ready: vec![0.0; hop * 2],
            given: hop,
            base: 0,
            heard: 0,
            slow: 1,
            left_at: None,
            joined: false,
        }
    }

    /// The playing callback while the sound is slowed: what [`crate::fill`]
    /// does, with the mix read through grains. False, and nothing touched,
    /// when the sound is stopped or at its own speed.
    pub(crate) fn fill(
        &mut self,
        shared: &Shared,
        scrub: &mut Scrub,
        out: &mut [f32],
        channels: usize,
    ) -> bool {
        let slow = shared.slow.load(Ordering::Relaxed);
        if slow <= 1 || channels == 0 || !shared.playing.load(Ordering::Relaxed) {
            self.left_at = None;
            return false;
        }
        out.fill(0.0);
        shared.scrub.store(0, Ordering::Relaxed);
        *scrub = Scrub::default();
        let Some(guard) = shared.plan.try_read() else {
            return true;
        };
        let Some(plan) = guard.as_ref() else {
            shared.meters.silence();
            return true;
        };
        let began = shared.playhead.load(Ordering::Relaxed);
        if self.left_at != Some(began) || self.slow != slow {
            self.base = began;
            self.heard = 0;
            self.slow = slow;
            self.given = self.hop;
            self.joined = false;
            self.tail.fill(0.0);
        }
        let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
        let mut missed = 0usize;
        for frame in out.chunks_exact_mut(channels) {
            let at = self.at();
            if at >= plan.total_frames {
                shared.playing.store(false, Ordering::Relaxed);
                break;
            }
            if self.given >= self.hop {
                missed += self.lay(plan, at, &mut acc);
            }
            let i = self.given * 2;
            for (sample, s) in frame.iter_mut().zip(&self.ready[i..i + 2]) {
                *sample = *s;
            }
            self.given += 1;
            self.heard += 1;
        }
        let playhead = self.at();
        self.left_at = Some(playhead);
        // As `fill` does: a seek that landed meanwhile is not written over.
        if shared
            .playhead
            .compare_exchange(began, playhead, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            shared.step.store(playhead - began, Ordering::Relaxed);
            shared.stepped_at.store(nanos(), Ordering::Relaxed);
        }
        shared.meters.publish(&acc);
        if missed > 0 {
            shared.starved.fetch_add(missed as u64, Ordering::Relaxed);
        }
        if shared.muted.load(Ordering::Relaxed) {
            out.fill(0.0);
        }
        true
    }

    /// The mix frame being heard now.
    fn at(&self) -> usize {
        self.base + (self.heard / u64::from(self.slow.max(1))) as usize
    }

    /// Lay the next grain, wanted at mix frame `want`, over the last one's
    /// tail, and make the next half grain ready. What the mix missed comes
    /// back.
    fn lay(
        &mut self,
        plan: &MixPlan,
        want: usize,
        acc: &mut [meter::MeterAcc; meter::SLOTS],
    ) -> usize {
        let lo = want.saturating_sub(self.seek);
        let missed = plan.mix_into(lo, &mut self.span, Some(acc));
        let hop = self.hop;
        let span = &self.span;
        let mono = |i: usize| span[i * 2] + span[i * 2 + 1];
        // Where the grain starts, as an offset into `span`: `want` itself
        // unless another start near it clearly sounds more like what the
        // last grain would have carried on into.
        let mut best = want - lo;
        if self.joined {
            let carry = &self.carry;
            let score = |d: usize| {
                let (mut dot, mut a, mut b) = (0.0f32, 0.0f32, 0.0f32);
                for j in (0..hop).step_by(4) {
                    let (x, y) = (carry[j], mono(d + j));
                    dot += x * y;
                    a += x * x;
                    b += y * y;
                }
                if a > 0.0 && b > 0.0 {
                    dot / (a * b).sqrt()
                } else {
                    0.0
                }
            };
            let mut best_score = score(best) + 1e-3;
            for d in (0..=want - lo + self.seek).step_by(2) {
                let s = score(d);
                if s > best_score {
                    best_score = s;
                    best = d;
                }
            }
        }
        for j in 0..hop {
            let (w, k) = (self.window[j], (best + j) * 2);
            self.ready[j * 2] = self.tail[j * 2] + span[k] * w;
            self.ready[j * 2 + 1] = self.tail[j * 2 + 1] + span[k + 1] * w;
        }
        for j in 0..hop {
            let (w, k) = (self.window[hop + j], best + hop + j);
            self.tail[j * 2] = span[k * 2] * w;
            self.tail[j * 2 + 1] = span[k * 2 + 1] * w;
            self.carry[j] = mono(k);
        }
        self.joined = true;
        self.given = 0;
        missed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_media::audio::AudioBuffer;
    use std::sync::Arc;

    /// Half and quarter speed through the callback: the clock moves one mix
    /// frame for every two or four heard, a tone keeps its pitch, and the
    /// joins between grains and between buffers do not click.
    #[test]
    fn slowed_sound_keeps_its_pitch_and_slows_the_clock() {
        let (rate, hz) = (48_000u32, 440.0f32);
        let samples: Vec<f32> = (0..rate as usize * 2)
            .flat_map(|n| {
                let v = (std::f32::consts::TAU * hz * n as f32 / rate as f32).sin() * 0.5;
                [v, v]
            })
            .collect();
        for slow in [2, 4] {
            let buffer = AudioBuffer {
                rate,
                samples: samples.clone(),
            };
            let shared = Shared::new(Some(crate::plan_of(Arc::new(buffer))));
            shared.playing.store(true, Ordering::Relaxed);
            shared.slow.store(slow, Ordering::Relaxed);
            let (mut stretch, mut scrub) = (Stretch::new(rate), Scrub::default());
            let mut out = vec![0.0f32; 480 * 2];
            let mut left = Vec::new();
            for _ in 0..100 {
                assert!(stretch.fill(&shared, &mut scrub, &mut out, 2));
                left.extend(out.chunks_exact(2).map(|f| f[0]));
            }
            let moved = shared.playhead.load(Ordering::Relaxed);
            assert_eq!(moved, rate as usize / slow as usize);

            // Past the first grain, which rises from nothing.
            let left = &left[2_000..];
            let rising = left
                .windows(2)
                .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
                .count();
            let pitch = rising as f32 * rate as f32 / left.len() as f32;
            assert!((pitch - hz).abs() < 5.0, "heard {pitch} Hz at 1/{slow}");
            // The tone's own steepest step is about 0.029.
            let step = left
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max);
            assert!(step < 0.06, "a step of {step} at 1/{slow}");
        }
    }
}
