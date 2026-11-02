//! How the render worker waits and keeps time.
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

/// A moment `ago` before now, or now itself when the clock has not run that
/// long. A page's clock starts when the page loads.
pub(crate) fn long_ago(ago: Duration) -> web_time::Instant {
    let now = web_time::Instant::now();
    now.checked_sub(ago).unwrap_or(now)
}

/// The next request, waiting up to `wait` for one. The browser build never
/// waits in place: it takes what is there and comes back after `wait`.
pub(crate) fn take_request<T>(
    receiver: &std::sync::mpsc::Receiver<T>,
    wait: Duration,
) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
    #[cfg(not(target_arch = "wasm32"))]
    return receiver.recv_timeout(wait);
    #[cfg(target_arch = "wasm32")]
    match receiver.try_recv() {
        Ok(request) => Ok(request),
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            pause(wait);
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        }
    }
}

/// Ask the operating system for 1 ms sleep granularity (Windows; a no-op
/// elsewhere, where sleeps are already fine-grained).
///
/// The worker paces presents with short sleeps, and Windows rounds a sleep UP
/// to the system timer's next tick — by default ~15.6 ms apart. That is twice
/// a 120 fps frame period: every sleep-then-present overshot its due time, the
/// present grid re-anchored, and the achieved rate fell to whatever the timer
/// allowed (~85 of 120 in practice; at 60 fps the grid held but presents
/// jittered by several milliseconds, which is what kept stopping the sound).
/// One millisecond is the granularity every media application requests; the
/// setting is process-wide and lives for the process, exactly as it does in
/// any NLE. A refusal is harmless — pacing then leans on the spin window
/// alone.
#[cfg(windows)]
pub(crate) fn raise_timer_resolution() {
    // SAFETY: `timeBeginPeriod` touches no memory owned by this program; it
    // adjusts a scheduler setting and reports acceptance or refusal in its
    // return value, which is ignored because a refusal leaves nothing to do.
    unsafe {
        let _ = windows::Win32::Media::timeBeginPeriod(1);
    }
}

#[cfg(not(windows))]
pub(crate) fn raise_timer_resolution() {}

/// Run this thread a step above the rest of the machine while it plays, and
/// back at the ordinary priority when it stops (Windows).
///
/// A picture is due at a moment, and on a machine with every core busy the
/// thread that sends it waited its turn like any other: with sixteen cores
/// loaded the pictures of a cached span went out ten milliseconds late, and
/// on time with this. Only while playing, so that an idle fill or a heavy
/// still frame never takes a core ahead of the interface.
// ponytail: Windows only. macOS and Linux want their own call (a QoS class,
// a nice value) and a machine to measure it on.
#[cfg(windows)]
pub(crate) fn playback_priority(playing: bool) {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_NORMAL,
    };
    let priority = if playing {
        THREAD_PRIORITY_ABOVE_NORMAL
    } else {
        THREAD_PRIORITY_NORMAL
    };
    // SAFETY: the pseudo-handle of the calling thread and a priority constant.
    // A refusal leaves the thread as it was, which is all there is to do.
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), priority);
    }
}

#[cfg(not(windows))]
pub(crate) fn playback_priority(_playing: bool) {}

/// Start a worker: `turn` runs again and again from the page's timer until it
/// answers false.
#[cfg(target_arch = "wasm32")]
pub(crate) fn start(turn: impl FnMut() -> bool + 'static) {
    web::TURNS.with(|turns| turns.borrow_mut().push(Box::new(turn)));
    web::schedule(0.0);
}

/// A render worker's turn has run, so its renderer is built and the page can
/// stop showing that it is loading (`lumitEngineReady` in
/// flutter_ui/web/flutter_bootstrap.js). Said once.
#[cfg(target_arch = "wasm32")]
pub(crate) fn engine_ready() {
    thread_local!(static SAID: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if SAID.with(|said| said.replace(true)) {
        return;
    }
    let global = js_sys::global();
    if let Ok(ready) = js_sys::Reflect::get(&global, &"lumitEngineReady".into()) {
        if let Some(ready) = wasm_bindgen::JsCast::dyn_ref::<js_sys::Function>(&ready) {
            let _ = ready.call0(&global);
        }
    }
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
