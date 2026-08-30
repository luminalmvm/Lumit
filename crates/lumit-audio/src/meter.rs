//! What the mix is doing, as numbers a panel can draw (docs/09-AUDIO.md §3.1).
//!
//! # In plain terms
//!
//! A mixing desk has a row of bouncing bars beside each fader. They are not
//! part of making the sound — they are how a person sees that a layer is too
//! loud, or that a track everybody swore was playing is in fact silent. This
//! module is the numbers behind those bars: for every strip in the mix, and
//! for the master, the loudest sample of the last buffer (**peak**) and the
//! average energy of it (**RMS**), plus a sticky flag saying the limiter has
//! had to hold something back (**clip**).
//!
//! **The realtime callback is the only writer**, and it is sacred: no locks,
//! no allocation, no waiting. So the meters are a small fixed bank of plain
//! atomics that the callback *overwrites* once per buffer — never a queue, a
//! ring the reader must drain, or anything that can fill up. The panel loads
//! whatever is there whenever it repaints, and if it misses a buffer it has
//! missed ten milliseconds of a bar that is about to move again. That is what
//! a meter is: the newest reading wins, and there is no backlog to fall
//! behind on.
//!
//! **Peak hold is the panel's**, not this module's — the little line that
//! rests above the bar for a few seconds is a drawing decision, and keeping
//! it here would mean the engine owning a stopwatch for a UI affordance.
//!
//! **Clip is sticky and is reset by hand.** A meter that cleared its own clip
//! light after a moment tells you an overload happened only if you were
//! looking; the whole point of the light is that it is still on when you come
//! back to the desk.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// How many strips carry meters. Past this many sounding strips the extras
/// are simply unmetered — they still play, they just have no bar.
///
// ponytail: a fixed bank, because the callback cannot allocate and a
// comp with more than 32 sounding strips is not the case the mixer is for.
// If one ever is, the bank becomes an `Arc<[Slot]>` sized when the plan is
// built and swapped with it — the writer/reader shape here does not change.
pub const MAX_STRIPS: usize = 32;

/// The slot the summed, master-faded, limited output is metered into — the
/// last one, past every strip.
pub const MASTER: usize = MAX_STRIPS;

/// The number of slots a [`Meters`] holds: every strip, plus the master.
pub const SLOTS: usize = MAX_STRIPS + 1;

/// One strip's meter, as the panel reads it. Linear sample amplitudes, not
/// decibels: the mix works in amplitudes, and a bar that wants dB converts
/// once where it is drawn rather than the engine guessing which scale the
/// drawing wanted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterReading {
    /// Loudest absolute sample of the last buffer, left and right.
    pub peak: [f32; 2],
    /// Root-mean-square of the last buffer, left and right.
    pub rms: [f32; 2],
    /// Something has reached the master ceiling since this was last reset —
    /// the limiter is holding sound back. Sticky ([`Meters::reset_clip`]).
    pub clipped: bool,
}

/// One buffer's running numbers for one strip, accumulated on the callback's
/// own stack and published in one go. Plain floats, so the per-sample work is
/// two compares and two multiply-adds and no atomic traffic at all.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeterAcc {
    peak: [f32; 2],
    sum_sq: [f64; 2],
    frames: u64,
    clipped: bool,
}

impl MeterAcc {
    /// Fold one stereo frame in. `ceiling` is the master limiter's, so a
    /// sample that has reached it lights the clip flag.
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
        MeterReading {
            peak: self.peak,
            rms: [rms(self.sum_sq[0]), rms(self.sum_sq[1])],
            clipped: self.clipped,
        }
    }
}

/// One slot's published numbers. Four amplitudes as raw f32 bits plus the
/// clip flag — relaxed loads and stores only, because there is exactly one
/// writer (the audio callback) and readers who want the newest value rather
/// than a consistent set.
#[derive(Default)]
struct Slot {
    peak_l: AtomicU32,
    peak_r: AtomicU32,
    rms_l: AtomicU32,
    rms_r: AtomicU32,
    clipped: AtomicBool,
}

impl Slot {
    fn store(&self, r: MeterReading) {
        self.peak_l.store(r.peak[0].to_bits(), Ordering::Relaxed);
        self.peak_r.store(r.peak[1].to_bits(), Ordering::Relaxed);
        self.rms_l.store(r.rms[0].to_bits(), Ordering::Relaxed);
        self.rms_r.store(r.rms[1].to_bits(), Ordering::Relaxed);
        if r.clipped {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }

    fn load(&self) -> MeterReading {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        MeterReading {
            peak: [f(&self.peak_l), f(&self.peak_r)],
            rms: [f(&self.rms_l), f(&self.rms_r)],
            clipped: self.clipped.load(Ordering::Relaxed),
        }
    }
}

/// The meter bank shared between the audio callback and whoever draws it.
pub struct Meters {
    slots: [Slot; SLOTS],
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            // `[Slot; SLOTS]` cannot derive Default (arrays only do up to 32
            // and Slot is not Copy), so the array is built from a closure.
            slots: std::array::from_fn(|_| Slot::default()),
        }
    }
}

impl Meters {
    /// Publish a callback's worth of accumulated numbers. Called once per
    /// buffer from the realtime callback: [`SLOTS`] × five relaxed stores,
    /// no allocation, nothing that can block.
    pub fn publish(&self, acc: &[MeterAcc; SLOTS]) {
        for (slot, a) in self.slots.iter().zip(acc.iter()) {
            slot.store(a.reading());
        }
    }

    /// Drop every bar to silence, keeping the clip flags. What a paused
    /// transport publishes, so the meters fall rather than freezing at
    /// whatever was playing when the button was pressed.
    pub fn silence(&self) {
        for slot in &self.slots {
            slot.store(MeterReading::default());
        }
    }

    /// One slot's newest reading; [`MASTER`] for the output. An index past
    /// the bank reads as silence rather than refusing.
    #[must_use]
    pub fn read(&self, slot: usize) -> MeterReading {
        self.slots.get(slot).map(Slot::load).unwrap_or_default()
    }

    /// Put every clip light out — the desk's "I have seen it" button.
    pub fn reset_clip(&self) {
        for slot in &self.slots {
            slot.clipped.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The accumulator's arithmetic: peak is the loudest absolute sample,
    /// RMS is the root mean square over the frames actually seen, and an
    /// empty buffer reads as silence rather than dividing by nothing.
    #[test]
    fn the_accumulator_is_peak_and_root_mean_square() {
        let mut acc = MeterAcc::default();
        assert_eq!(acc.reading(), MeterReading::default());

        // Two frames: L swings ±0.5 (RMS 0.5), R is 0 then 1 (RMS 1/√2).
        acc.add(0.5, 0.0, 1.0);
        acc.add(-0.5, 1.0, 1.0);
        let r = acc.reading();
        assert!((r.peak[0] - 0.5).abs() < 1e-6, "peak is the absolute value");
        assert!((r.peak[1] - 1.0).abs() < 1e-6);
        assert!((r.rms[0] - 0.5).abs() < 1e-6);
        assert!((r.rms[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(r.clipped, "a sample at the ceiling is a clip");
    }

    /// A sample under the ceiling is not a clip, and the flag is sticky
    /// across buffers until it is reset by hand — the light stays on so it
    /// can be seen by someone who was not watching when it happened.
    #[test]
    fn the_clip_light_stays_on_until_it_is_put_out() {
        let meters = Meters::default();
        let mut quiet = [MeterAcc::default(); SLOTS];
        quiet[0].add(0.3, 0.3, 0.966);
        meters.publish(&quiet);
        assert!(!meters.read(0).clipped);

        let mut hot = [MeterAcc::default(); SLOTS];
        hot[0].add(0.99, 0.1, 0.966);
        meters.publish(&hot);
        assert!(meters.read(0).clipped);

        // Quiet buffers afterwards do not clear it; silence does not either.
        meters.publish(&quiet);
        meters.silence();
        assert!(meters.read(0).clipped, "only a reset puts the light out");
        meters.reset_clip();
        assert!(!meters.read(0).clipped);
    }

    /// Publishing overwrites rather than accumulating (the newest buffer is
    /// the reading), silence drops the bars, the master has its own slot,
    /// and a slot past the bank reads as silence rather than panicking.
    #[test]
    fn the_newest_buffer_is_the_reading_and_the_master_has_its_own_slot() {
        let meters = Meters::default();
        let mut acc = [MeterAcc::default(); SLOTS];
        acc[2].add(0.8, 0.8, 1.0);
        acc[MASTER].add(0.4, 0.4, 1.0);
        meters.publish(&acc);
        assert!((meters.read(2).peak[0] - 0.8).abs() < 1e-6);
        assert!((meters.read(MASTER).peak[0] - 0.4).abs() < 1e-6);
        assert_eq!(meters.read(1), MeterReading::default(), "an unused strip");

        // A quieter buffer replaces the loud one — meters fall, they do not
        // remember (the hold above the bar is the panel's, not this bank's).
        let mut quieter = [MeterAcc::default(); SLOTS];
        quieter[2].add(0.1, 0.1, 1.0);
        meters.publish(&quieter);
        assert!((meters.read(2).peak[0] - 0.1).abs() < 1e-6);

        meters.silence();
        assert_eq!(meters.read(2).peak, [0.0, 0.0]);
        assert_eq!(meters.read(SLOTS + 5), MeterReading::default());
    }
}
