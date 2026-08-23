//! Whether the render worker is measuring what each frame costs (docs/13 §7.1).
//!
//! # In plain terms
//!
//! The render-time indicators — the Timeline's column, the numbers on the
//! effect rows — are only worth their cost while something is showing them.
//! Measuring a frame makes the processor wait for the graphics card at every
//! layer and every effect (see `lumit_render::profile`), which is exactly the
//! overlap a fast preview depends on. So the frontend says when it wants the
//! numbers, and the worker reads that wish before each frame. A frame a tier
//! already holds is served regardless, and measured on the idle turn after
//! (K-420): the picture never waits for its own numbers.
//!
//! One flag, one atomic: it is written from Dart's thread and read on the
//! render thread, and neither may wait for the other — the same shape the cache
//! budgets take (see [`crate::framecache`]).

use std::sync::atomic::{AtomicBool, Ordering};

/// The wish. **On by default (K-276 revision)**: the numbers are what the
/// column is for, and a diagnostic nobody can find is not shipped — the first
/// arrangement asked the user to press a glyph in a column header, and the
/// answer to "why is it empty" was "you have to switch it on", which is no
/// answer at all. The frontend's switch now lives in the bottom strip beside
/// the cache meters, where a session-wide toggle belongs, and both sides start
/// in the same state without a call at startup.
///
/// Relaxed ordering throughout: this decides whether the *next* frame is
/// measured, and a frame either side of the change is a correct answer to a
/// question about a moving picture.
static WANTED: AtomicBool = AtomicBool::new(true);

/// Whether the first measured frame since the switch went on has been announced
/// — see [`announce_first`].
static ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// Ask for (or stop asking for) per-layer and per-effect timings.
pub(crate) fn set_wanted(on: bool) {
    WANTED.store(on, Ordering::Relaxed);
    ANNOUNCED.store(false, Ordering::Relaxed);
    println!("Render profiling {}", if on { "on" } else { "off" });
}

/// Say — **once** per switching on — that a frame really was measured, and what
/// came out of it.
///
/// Two lines in a session's console, and they answer the question that took a
/// day of guessing to answer without them: the column shows numbers only if the
/// engine measures a frame *and* the frontend recognises the rows it names, and
/// from the outside those two failures look identical. One line on the switch,
/// one on the first measured frame: no second line means the engine never
/// measured, and a second line with layers in it means the numbers left here.
pub(crate) fn announce_first(frame: u64, layers: usize, total_ms: f32) {
    if !ANNOUNCED.swap(true, Ordering::Relaxed) {
        println!("Render profiling: measured frame {frame} — {layers} layer(s), {total_ms:.1} ms");
    }
}

/// Whether the next frame should be measured.
pub(crate) fn wanted() -> bool {
    WANTED.load(Ordering::Relaxed)
}
