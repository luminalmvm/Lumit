//! Holding an export between two frames. One for the process, because one
//! export runs at a time in it, and each one starts let go.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the export running now is held.
static HELD: AtomicBool = AtomicBool::new(false);

/// Hold the export running now, or let it go on.
pub(super) fn set(on: bool) {
    HELD.store(on, Ordering::Relaxed);
}

/// Wait here while the export is held. A cancel lets it go at once.
pub(super) fn wait(cancel: &AtomicBool) {
    while HELD.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
