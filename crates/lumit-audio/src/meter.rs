//! What the mix is doing, as numbers a panel can draw (docs/09-AUDIO.md §3.1).
//!
//! # In plain terms
//!
//! A mixing desk has a row of bouncing bars beside each fader. They are not
//! part of making the sound: they are how a person sees that a layer is too
//! loud, or that a track everybody swore was playing is in fact silent. This
//! module is the numbers behind those bars: for every node of the mix, and
//! for the master, the loudest sample of the last buffer (**peak**) and the
//! average energy of it (**RMS**) after the fader, how far a duck or the
//! limiter pulled it down (**gain reduction**), plus a sticky flag saying it
//! went over (**clip**). The master also carries its loudness, which
//! [`crate::loudness`] measures.
//!
//! **The realtime callback is the only writer**, and it is sacred: no locks,
//! no allocation, no waiting. So the meters are a fixed bank of plain atomics
//! that the callback *overwrites* once per buffer, never a queue, a ring the
//! reader must drain, or anything that can fill up. The panel loads whatever
//! is there whenever it repaints, and if it misses a buffer it has missed ten
//! milliseconds of a bar that is about to move again. That is what a meter
//! is: the newest reading wins, and there is no backlog to fall behind on.
//!
//! **Peak hold is the panel's**, not this module's: the little line that
//! rests above the bar for a few seconds is a drawing decision, and keeping
//! it here would mean the engine owning a stopwatch for a UI affordance.
//!
//! **Clip is sticky and is reset by hand.** A meter that cleared its own clip
//! light after a moment tells you an overload happened only if you were
//! looking; the whole point of the light is that it is still on when you come
//! back to the desk. A node's lights at full scale; the master's when the
//! limiter takes off more than 0.1 dB.

use crate::plan::MAX_NODES;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// The slot the master is metered into: the last one, past every node's.
/// Node `i` meters into slot `i`.
pub const MASTER: usize = MAX_NODES;

/// The number of slots a [`Meters`] holds: every node, plus the master.
pub const SLOTS: usize = MAX_NODES + 1;

/// A buffer's running numbers for every slot, kept by the mix.
pub type MeterAccs = [MeterAcc; SLOTS];

/// One slot's meter, as the panel reads it. Linear sample amplitudes, not
/// decibels: the mix works in amplitudes, and a bar that wants dB converts
/// once where it is drawn rather than the engine guessing which scale the
/// drawing wanted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterReading {
    /// Loudest absolute sample of the last buffer, left and right.
    pub peak: [f32; 2],
    /// Root-mean-square of the last buffer, left and right.
    pub rms: [f32; 2],
    /// The most the last buffer was pulled down, in dB (positive): a node's
    /// duck, the master's limiter.
    pub gr: f32,
    /// It has gone over since this was last reset. Sticky
    /// ([`Meters::reset_clip`]).
    pub clipped: bool,
}

/// One buffer's running numbers for one slot, accumulated by the mix and
/// published in one go. Plain floats, so the per-sample work is two compares
/// and two multiply-adds and no atomic traffic at all.
#[derive(Clone, Copy, Debug)]
pub struct MeterAcc {
    peak: [f32; 2],
    sum_sq: [f64; 2],
    frames: u64,
    /// The smallest gain applied in the buffer.
    lowest: f32,
    clipped: bool,
}

impl Default for MeterAcc {
    fn default() -> Self {
        Self {
            peak: [0.0; 2],
            sum_sq: [0.0; 2],
            frames: 0,
            lowest: 1.0,
            clipped: false,
        }
    }
}

impl MeterAcc {
    /// Fold one stereo frame in. A sample at or over `ceiling` lights the
    /// clip flag.
    #[inline]
    pub fn add(&mut self, l: f32, r: f32, ceiling: f32) {
        let (al, ar) = (l.abs(), r.abs());
        self.peak[0] = self.peak[0].max(al);
        self.peak[1] = self.peak[1].max(ar);
        self.sum_sq[0] += f64::from(l) * f64::from(l);
        self.sum_sq[1] += f64::from(r) * f64::from(r);
        self.frames += 1;
        self.clipped |= al >= ceiling || ar >= ceiling;
    }

    /// A gain of `gain` (linear) was applied: the deepest is the reading's
    /// gain reduction.
    #[inline]
    pub fn reduce(&mut self, gain: f32) {
        self.lowest = self.lowest.min(gain);
    }

    /// Light the clip flag.
    #[inline]
    pub fn clip(&mut self) {
        self.clipped = true;
    }

    /// This buffer's reading: peaks as they stand, RMS over the frames seen.
    #[must_use]
    fn reading(&self) -> MeterReading {
        let rms = |sum: f64| {
            if self.frames == 0 {
                0.0
            } else {
                (sum / self.frames as f64).sqrt() as f32
            }
        };
        let gr = if self.lowest >= 1.0 {
            0.0
        } else if self.lowest > 0.0 {
            -20.0 * self.lowest.log10()
        } else {
            f32::INFINITY
        };
        MeterReading {
            peak: self.peak,
            rms: [rms(self.sum_sq[0]), rms(self.sum_sq[1])],
            gr,
            clipped: self.clipped,
        }
    }
}

/// One slot's published numbers as raw f32 bits plus the clip flag. Relaxed
/// loads and stores only, because there is exactly one writer (the audio
/// callback) and readers who want the newest value rather than a consistent
/// set.
#[derive(Default)]
struct Slot {
    peak_l: AtomicU32,
    peak_r: AtomicU32,
    rms_l: AtomicU32,
    rms_r: AtomicU32,
    gr: AtomicU32,
    clipped: AtomicBool,
}

impl Slot {
    fn store(&self, r: MeterReading) {
        self.peak_l.store(r.peak[0].to_bits(), Ordering::Relaxed);
        self.peak_r.store(r.peak[1].to_bits(), Ordering::Relaxed);
        self.rms_l.store(r.rms[0].to_bits(), Ordering::Relaxed);
        self.rms_r.store(r.rms[1].to_bits(), Ordering::Relaxed);
        self.gr.store(r.gr.to_bits(), Ordering::Relaxed);
        if r.clipped {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }

    fn load(&self) -> MeterReading {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        MeterReading {
            peak: [f(&self.peak_l), f(&self.peak_r)],
            rms: [f(&self.rms_l), f(&self.rms_r)],
            gr: f(&self.gr),
            clipped: self.clipped.load(Ordering::Relaxed),
        }
    }
}

/// The master's loudness ([`crate::loudness`]): momentary, short-term and
/// integrated in LUFS, minus infinity while there is nothing to read; the
/// loudness range in LU; and the loudest true peak since the measure began,
/// as an amplitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessReading {
    pub momentary: f32,
    pub short_term: f32,
    pub integrated: f32,
    pub range: f32,
    pub true_peak: f32,
}

/// The meter bank shared between the audio callback and whoever draws it.
pub struct Meters {
    slots: Box<[Slot]>,
    momentary: AtomicU32,
    short_term: AtomicU32,
    integrated: AtomicU32,
    range: AtomicU32,
    true_peak: AtomicU32,
    /// Asked for by the desk, taken by the callback, which then starts the
    /// loudness measure afresh.
    loudness_reset: AtomicBool,
}

impl Default for Meters {
    fn default() -> Self {
        let silent = f32::NEG_INFINITY.to_bits();
        Self {
            slots: (0..SLOTS).map(|_| Slot::default()).collect(),
            momentary: AtomicU32::new(silent),
            short_term: AtomicU32::new(silent),
            integrated: AtomicU32::new(silent),
            range: AtomicU32::new(0),
            true_peak: AtomicU32::new(0),
            loudness_reset: AtomicBool::new(false),
        }
    }
}

impl Meters {
    /// Publish a callback's worth of accumulated numbers for the first
    /// `nodes` nodes and the master. Called once per buffer from the realtime
    /// callback: one set of relaxed stores per slot, no allocation, nothing
    /// that can block.
    pub fn publish(&self, acc: &MeterAccs, nodes: usize) {
        for (slot, a) in self.slots.iter().zip(acc.iter()).take(nodes.min(MASTER)) {
            slot.store(a.reading());
        }
        if let (Some(slot), Some(a)) = (self.slots.get(MASTER), acc.get(MASTER)) {
            slot.store(a.reading());
        }
    }

    /// Drop every bar to silence, keeping the clip flags. What a paused
    /// transport publishes, so the meters fall rather than freezing at
    /// whatever was playing when the button was pressed.
    pub fn silence(&self) {
        for slot in self.slots.iter() {
            slot.store(MeterReading::default());
        }
    }

    /// One slot's newest reading; [`MASTER`] for the output. An index past
    /// the bank reads as silence rather than refusing.
    #[must_use]
    pub fn read(&self, slot: usize) -> MeterReading {
        self.slots.get(slot).map(Slot::load).unwrap_or_default()
    }

    /// Put every clip light out: the desk's "I have seen it" button.
    pub fn reset_clip(&self) {
        for slot in self.slots.iter() {
            slot.clipped.store(false, Ordering::Relaxed);
        }
    }

    /// The master's loudness as the callback last published it.
    #[must_use]
    pub fn loudness(&self) -> LoudnessReading {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        LoudnessReading {
            momentary: f(&self.momentary),
            short_term: f(&self.short_term),
            integrated: f(&self.integrated),
            range: f(&self.range),
            true_peak: f(&self.true_peak),
        }
    }

    /// Start the integrated loudness, the range and the true peak afresh,
    /// from the next buffer the callback fills.
    pub fn reset_loudness(&self) {
        self.loudness_reset.store(true, Ordering::Relaxed);
    }

    pub(crate) fn take_loudness_reset(&self) -> bool {
        self.loudness_reset.swap(false, Ordering::Relaxed)
    }

    pub(crate) fn publish_loudness(&self, r: LoudnessReading) {
        let put = |a: &AtomicU32, v: f32| a.store(v.to_bits(), Ordering::Relaxed);
        put(&self.momentary, r.momentary);
        put(&self.short_term, r.short_term);
        put(&self.integrated, r.integrated);
        put(&self.range, r.range);
        put(&self.true_peak, r.true_peak);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A sample under full scale is not a clip, and the flag is sticky
    /// across buffers until it is reset by hand: the light stays on so it
    /// can be seen by someone who was not watching when it happened.
    #[test]
    fn the_clip_light_stays_on_until_it_is_put_out() {
        let meters = Meters::default();
        let mut quiet = [MeterAcc::default(); SLOTS];
        quiet[0].add(0.3, 0.3, 1.0);
        meters.publish(&quiet, 1);
        assert!(!meters.read(0).clipped);

        let mut hot = [MeterAcc::default(); SLOTS];
        hot[0].add(1.0, 0.1, 1.0);
        meters.publish(&hot, 1);
        assert!(meters.read(0).clipped);

        // Quiet buffers afterwards do not clear it; silence does not either.
        meters.publish(&quiet, 1);
        meters.silence();
        assert!(meters.read(0).clipped, "only a reset puts the light out");
        meters.reset_clip();
        assert!(!meters.read(0).clipped);
    }
}
