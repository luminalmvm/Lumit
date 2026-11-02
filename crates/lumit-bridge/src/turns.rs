//! How the render worker waits.
//!
//! On the desktop the worker is a thread of its own, and a wait is a sleep.
//! The browser build has one thread, the page's, which must never be held. So
//! there the worker takes one turn at a time from the page's timer, a wait is
//! how long until its next turn, and a request sent to it brings that turn
//! forward to now.

use std::time::Duration;

/// Let `wait` go by before the worker's next step. The caller ends its turn
/// straight after, as it would after a sleep.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pause(wait: Duration) {
    std::thread::sleep(wait);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn pause(wait: Duration) {
    web::NEXT.with(|next| {
        let soonest = next.get().map_or(wait, |had| had.min(wait));
        next.set(Some(soonest));
    });
}

/// Start a worker: `turn` runs again and again from the page's timer until it
/// answers false.
#[cfg(target_arch = "wasm32")]
pub(crate) fn start(turn: impl FnMut() -> bool + 'static) {
    web::TURNS.with(|turns| turns.borrow_mut().push(Box::new(turn)));
    web::schedule(0.0);
}

/// A request is waiting: take the next turn now.
#[cfg(target_arch = "wasm32")]
pub(crate) fn wake() {
    web::schedule(0.0);
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    type Turn = Box<dyn FnMut() -> bool>;

    thread_local! {
        /// Every running worker, one per open project.
        pub(super) static TURNS: RefCell<Vec<Turn>> = const { RefCell::new(Vec::new()) };
        /// The shortest wait asked for during this round of turns.
        pub(super) static NEXT: Cell<Option<Duration>> = const { Cell::new(None) };
        /// When the timer set is due, on the page's clock, if one is set.
        static DUE: Cell<Option<f64>> = const { Cell::new(None) };
        static TICK: Closure<dyn FnMut()> = Closure::new(round);
    }

    // ponytail: setTimeout, which browsers hold to 4 ms once nested. A
    // MessageChannel would make wake() sooner if that ever shows.
    pub(super) fn schedule(after_ms: f64) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let now = window.performance().map_or(0.0, |p| p.now());
        if DUE.with(|due| due.get().is_some_and(|due| due <= now + after_ms)) {
            return;
        }
        let set = TICK.with(|tick| {
            window.set_timeout_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                after_ms.max(0.0) as i32,
            )
        });
        if set.is_ok() {
            DUE.with(|due| due.set(Some(now + after_ms)));
        }
    }

    /// One turn of every worker, then the timer set for the soonest wait.
    fn round() {
        DUE.with(|due| due.set(None));
        NEXT.with(|next| next.set(None));
        // Taken out while they run, so a turn that starts another worker or
        // wakes this one finds the list free.
        let mut turns = TURNS.with(|turns| std::mem::take(&mut *turns.borrow_mut()));
        turns.retain_mut(|turn| turn());
        TURNS.with(|now| {
            let mut now = now.borrow_mut();
            turns.append(&mut now);
            *now = turns;
        });
        if TURNS.with(|turns| turns.borrow().is_empty()) {
            return;
        }
        let wait = NEXT.with(|next| next.get()).unwrap_or_default();
        schedule(wait.as_secs_f64() * 1000.0);
    }
}
