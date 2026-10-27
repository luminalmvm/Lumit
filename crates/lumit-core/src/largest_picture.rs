//! The widest and tallest picture the renderer will make.
//!
//! # In plain terms
//!
//! Every working picture lives in one texture on the graphics card, so it can
//! be no bigger than the card allows, and each doubling of its side costs four
//! times the memory. 8192 is what every card takes. A machine with a big card
//! can be given more in Settings, because a distort that throws the picture
//! far past the frame, an effect, then the undistort that brings it back needs
//! the middle picture whole.
//!
//! The setting belongs to the machine, not the project: a project opened on a
//! smaller machine should not ask it for more than it has.
//!
//! Anything that makes a picture smaller to keep under this calls [`fit`] or
//! [`room`], or [`note_cut`] itself, so the Viewer can say a frame was cut
//! rather than show a picture with its corners quietly missing.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

/// What every card takes, and what Lumit uses until it is told otherwise.
pub const DEFAULT_SIDE: u32 = 8192;

static ASKED: AtomicU32 = AtomicU32::new(DEFAULT_SIDE);
static CARD: AtomicU32 = AtomicU32::new(DEFAULT_SIDE);
static CUTS: AtomicU64 = AtomicU64::new(0);

/// The largest side in pixels: what was asked for, held to what the card takes.
#[must_use]
pub fn side() -> u32 {
    ASKED.load(Relaxed).min(CARD.load(Relaxed))
}

/// Ask for a largest side. Never below [`DEFAULT_SIDE`].
pub fn set(side: u32) {
    ASKED.store(side.max(DEFAULT_SIDE), Relaxed);
}

/// The most the graphics card takes, once a device is open on it.
#[must_use]
pub fn card() -> u32 {
    CARD.load(Relaxed)
}

/// Said by whoever opens the device, with the side it was opened with.
pub fn set_card(side: u32) {
    CARD.store(side.max(DEFAULT_SIDE), Relaxed);
}

/// Something was made smaller, or cut, to stay inside [`side`].
pub fn note_cut() {
    CUTS.fetch_add(1, Relaxed);
}

/// How many times anything has been cut. Read before and after a frame: if
/// it moved, that frame was cut.
// ponytail: one count for the whole process, so a frame made while another
// render is cut at the same moment is told it was cut too. Carry the count on
// the render if that shows.
#[must_use]
pub fn cuts() -> u64 {
    CUTS.load(Relaxed)
}

/// `want` pixels, or [`side`] if that is fewer.
#[must_use]
pub fn fit(want: u32) -> u32 {
    let most = side();
    if want > most {
        note_cut();
        return most;
    }
    want
}

/// `want` pixels of room on each side of an `n`-pixel picture, or as many as
/// keep the whole inside [`side`].
#[must_use]
pub fn room(want: u32, n: u32) -> u32 {
    let most = side().saturating_sub(n) / 2;
    if want > most {
        note_cut();
        return most;
    }
    want
}
