//! Fader overrides while a control is held (docs/09-AUDIO.md).
//!
//! A fader, a pan or a send level being dragged has to be heard while it
//! moves, not when it is let go. The bridge writes the value here as it
//! changes and the mix reads it in place of the automation.
//!
//! Written by the bridge on its own thread and read by whoever mixes: the
//! audio callback, and the rack thread and an export through their own
//! [`crate::block::MixWork`]. Plain atomics only, so the callback never waits
//! for a writer. A bank is made with its plan, sized for it, and handed the
//! state of the bank before it on a swap ([`LiveBank::seed_from`]), so a
//! fader held across an edit stays held.

use crate::plan::NONE;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};

/// The control is not held: the mix follows the fader's own value or keys.
pub const FOLLOW: u8 = 0;
/// The control is held: the mix uses the held gains.
pub const HOLD: u8 = 1;
/// The control was let go: the mix crossfades from the held gains (times
/// the keys, after a trim) back to the keys over the glide, then goes back
/// to [`FOLLOW`].
pub const GLIDE: u8 = 2;
/// The control is held as a trim: the mix uses the fader's own value or
/// keys times the held gains, so the automation keeps its shape.
pub const TRIM: u8 = 3;

/// One fader's override: a node's Volume and Pan, the master, or a send's
/// level (both sides the same).
#[derive(Default)]
pub struct LiveFader {
    /// The held `[left, right]` gains as f32 bits.
    gain: [AtomicU32; 2],
    /// [`FOLLOW`], [`HOLD`], [`GLIDE`] or [`TRIM`].
    mode: AtomicU8,
    /// Whether the held gains multiply the keys (a trim) rather than take
    /// their place. Kept through a release, so the glide starts from what
    /// was heard.
    times: AtomicBool,
    /// How long the glide back to the keys takes, in frames.
    glide_frames: AtomicU32,
    /// Moved on by every hold and every release, so the mix can tell a new
    /// glide from the one it is already in.
    epoch: AtomicU32,
}

impl LiveFader {
    /// Hold the control at `gains`. Called again on every move while it is
    /// dragged.
    pub fn hold(&self, gains: [f32; 2]) {
        self.set(gains, false, HOLD);
    }

    /// Hold the control as a trim: the keys times `gains`. Called again on
    /// every move while it is dragged. Let go the same way as a hold.
    pub fn trim(&self, gains: [f32; 2]) {
        self.set(gains, true, TRIM);
    }

    fn set(&self, gains: [f32; 2], times: bool, mode: u8) {
        self.gain[0].store(gains[0].to_bits(), Ordering::Relaxed);
        self.gain[1].store(gains[1].to_bits(), Ordering::Relaxed);
        self.times.store(times, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.mode.store(mode, Ordering::Release);
    }

    /// Let the control go: glide from the held gains back to the keys over
    /// `glide_frames`. Nought goes back at once.
    pub fn release(&self, glide_frames: u32) {
        if glide_frames == 0 {
            self.drop_hold();
            return;
        }
        self.glide_frames.store(glide_frames, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.mode.store(GLIDE, Ordering::Release);
    }

    /// Back to the keys at once, with no glide.
    pub fn drop_hold(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.mode.store(FOLLOW, Ordering::Release);
    }

    /// Whether the control is held now, as a hold or a trim (a glide is not
    /// held).
    #[must_use]
    pub fn held(&self) -> bool {
        matches!(self.mode(), HOLD | TRIM)
    }

    /// [`FOLLOW`], [`HOLD`], [`GLIDE`] or [`TRIM`].
    #[must_use]
    pub fn mode(&self) -> u8 {
        self.mode.load(Ordering::Acquire)
    }

    /// Whether the held gains multiply the keys: held as a trim, or gliding
    /// back from one.
    #[must_use]
    pub fn trims(&self) -> bool {
        self.times.load(Ordering::Relaxed)
    }

    /// The held gains.
    #[must_use]
    pub fn gains(&self) -> [f32; 2] {
        [
            f32::from_bits(self.gain[0].load(Ordering::Relaxed)),
            f32::from_bits(self.gain[1].load(Ordering::Relaxed)),
        ]
    }

    /// The glide's length in frames.
    #[must_use]
    pub fn glide_frames(&self) -> u32 {
        self.glide_frames.load(Ordering::Relaxed)
    }

    /// Which hold or release the control is on.
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The glide begun at `epoch` has run its length: back to [`FOLLOW`],
    /// unless the control has been held or let go again since.
    pub(crate) fn finish_glide(&self, epoch: u32) {
        if self.epoch() == epoch {
            let _ = self
                .mode
                .compare_exchange(GLIDE, FOLLOW, Ordering::AcqRel, Ordering::Relaxed);
        }
    }

    /// Take on another fader's state, for a plan swap.
    fn copy_from(&self, other: &LiveFader) {
        for (to, from) in self.gain.iter().zip(&other.gain) {
            to.store(from.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        self.glide_frames
            .store(other.glide_frames(), Ordering::Relaxed);
        self.times.store(other.trims(), Ordering::Relaxed);
        self.epoch.store(other.epoch(), Ordering::Release);
        self.mode.store(other.mode(), Ordering::Release);
    }
}

/// Every override a plan has: one per node and the master last, and one per
/// send, in the order the plan's nodes list them.
pub struct LiveBank {
    faders: Box<[LiveFader]>,
    sends: Box<[LiveFader]>,
    /// Per send, the old bank's send it was seeded from, or [`NONE`], so a
    /// mix can carry its own glide on that send across the swap.
    send_from: Box<[AtomicU16]>,
}

impl Default for LiveBank {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl LiveBank {
    /// A bank for `nodes` nodes and the master, and `sends` sends, none held.
    #[must_use]
    pub fn new(nodes: usize, sends: usize) -> Self {
        Self {
            faders: (0..=nodes).map(|_| LiveFader::default()).collect(),
            sends: (0..sends).map(|_| LiveFader::default()).collect(),
            send_from: (0..sends).map(|_| AtomicU16::new(NONE)).collect(),
        }
    }

    /// Fader `i`: a node's, or the master's at the last index.
    #[must_use]
    pub fn fader(&self, i: usize) -> Option<&LiveFader> {
        self.faders.get(i)
    }

    /// The master's fader.
    #[must_use]
    pub fn master(&self) -> Option<&LiveFader> {
        self.faders.last()
    }

    /// Send `i`'s level.
    #[must_use]
    pub fn send(&self, i: usize) -> Option<&LiveFader> {
        self.sends.get(i)
    }

    /// Which send of the bank before send `i` was seeded from, if any.
    pub(crate) fn send_carried(&self, i: usize) -> Option<u16> {
        let from = self.send_from.get(i)?.load(Ordering::Relaxed);
        (from != NONE).then_some(from)
    }

    /// Hold fader `i` at `gains` ([`LiveFader::hold`]).
    pub fn hold(&self, i: usize, gains: [f32; 2]) {
        if let Some(f) = self.fader(i) {
            f.hold(gains);
        }
    }

    /// Hold fader `i` as a trim of its keys by `gains` ([`LiveFader::trim`]).
    pub fn trim(&self, i: usize, gains: [f32; 2]) {
        if let Some(f) = self.fader(i) {
            f.trim(gains);
        }
    }

    /// Let fader `i` go over `glide_frames` ([`LiveFader::release`]).
    pub fn release(&self, i: usize, glide_frames: u32) {
        if let Some(f) = self.fader(i) {
            f.release(glide_frames);
        }
    }

    /// Fader `i` back to its keys at once ([`LiveFader::drop_hold`]).
    pub fn drop_hold(&self, i: usize) {
        if let Some(f) = self.fader(i) {
            f.drop_hold();
        }
    }

    /// Whether fader `i` is held.
    #[must_use]
    pub fn held(&self, i: usize) -> bool {
        self.fader(i).is_some_and(LiveFader::held)
    }

    /// Take on the held and gliding state of the bank before a swap.
    /// `carry[n]` is node `n`'s index in the old plan and `send_carry[s]`
    /// send `s`'s, or [`crate::plan::NONE`] for one that is new. The master
    /// carries to the master. [`crate::plan::MixPlan::seed_live`] works both
    /// lists out from the two plans.
    pub fn seed_from(&self, old: &LiveBank, carry: &[u16], send_carry: &[u16]) {
        let nodes = self.faders.len().saturating_sub(1);
        for (n, fader) in self.faders.iter().take(nodes).enumerate() {
            let was = carry
                .get(n)
                .filter(|&&o| usize::from(o) < old.faders.len().saturating_sub(1))
                .and_then(|&o| old.faders.get(usize::from(o)));
            if let Some(was) = was {
                fader.copy_from(was);
            }
        }
        if let (Some(master), Some(was)) = (self.master(), old.master()) {
            master.copy_from(was);
        }
        for (s, send) in self.sends.iter().enumerate() {
            if let Some(from) = self.send_from.get(s) {
                from.store(
                    send_carry.get(s).copied().unwrap_or(NONE),
                    Ordering::Relaxed,
                );
            }
            if let Some(was) = send_carry
                .get(s)
                .and_then(|&o| old.sends.get(usize::from(o)))
            {
                send.copy_from(was);
            }
        }
    }
}
