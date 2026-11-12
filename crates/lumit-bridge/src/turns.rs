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

/// Let a `wait` too short to sleep go by: spun on the desktop, where a sleep
/// is only as fine as the system timer, and a pause in the browser build.
pub(crate) fn spin(wait: Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let due = web_time::Instant::now() + wait;
        while web_time::Instant::now() < due {
            std::hint::spin_loop();
        }
    }
    #[cfg(target_arch = "wasm32")]
    pause(wait);
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

/// Keep Windows from throttling this process to save power (Windows; a no-op
/// elsewhere).
///
/// On a laptop, and more so on battery, Windows may run a process it judges
/// to be background work at low clocks ("efficiency mode") and may ignore its
/// timer resolution request, so a 1 ms sleep lasts a whole 15.6 ms tick. Both
/// land on a frame: the worker waits for the graphics card several times a
/// frame (the present, the D3D11 hop, a read-back), and each wait that sleeps
/// then sleeps a tick. Lumit is an editor somebody is looking at, so it asks
/// for neither, as a video player does. A refusal is harmless: things stay as
/// they were.
#[cfg(windows)]
pub(crate) fn opt_out_of_power_throttling() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
        PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    };
    // The mask names what this process decides; a state of 0 for each says
    // "not throttled".
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: the pseudo-handle of this process and a correctly sized,
    // initialised PROCESS_POWER_THROTTLING_STATE that outlives the call; the
    // result is ignored because a refusal leaves nothing to do.
    unsafe {
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            std::ptr::from_ref(&state).cast(),
            u32::try_from(std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>()).unwrap_or(0),
        );
    }
}

#[cfg(not(windows))]
pub(crate) fn opt_out_of_power_throttling() {}

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
        /// The shortest wait the worker taking its turn has asked for.
        pub(super) static NEXT: Cell<Option<Duration>> = const { Cell::new(None) };
        /// When the next round is due, on the page's clock, and the handle
        /// of its timer when it has one. Only ever one, or each wake would
        /// start another.
        static DUE: Cell<Option<(f64, Option<i32>)>> = const { Cell::new(None) };
        static TICK: Closure<dyn FnMut()> = Closure::new(round);
        /// Where a round wanted now is posted. Browsers hold a timer back at
        /// least 4 ms once they nest, which is a quarter of a frame at 60.
        static NOW: Option<web_sys::MessageChannel> = {
            let channel = web_sys::MessageChannel::new().ok();
            if let Some(channel) = &channel {
                let tick = TICK.with(|tick| tick.as_ref().clone());
                channel.port1().set_onmessage(Some(tick.unchecked_ref()));
            }
            channel
        };
    }

    pub(super) fn schedule(after_ms: f64) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let now = window.performance().map_or(0.0, |p| p.now());
        if let Some((due, timer)) = DUE.with(Cell::get) {
            if due <= now + after_ms {
                return;
            }
            if let Some(timer) = timer {
                window.clear_timeout_with_handle(timer);
            }
        }
        let posted = after_ms < 1.0
            && NOW.with(|channel| {
                channel.as_ref().is_some_and(|channel| {
                    channel
                        .port2()
                        .post_message(&wasm_bindgen::JsValue::NULL)
                        .is_ok()
                })
            });
        if posted {
            DUE.with(|due| due.set(Some((now, None))));
            return;
        }
        let set = TICK.with(|tick| {
            window.set_timeout_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                after_ms.max(0.0) as i32,
            )
        });
        if let Ok(timer) = set {
            DUE.with(|due| due.set(Some((now + after_ms, Some(timer)))));
        }
    }

    /// One turn of every worker, then the timer set for the soonest wait.
    fn round() {
        DUE.with(|due| due.set(None));
        // Taken out while they run, so a turn that starts another worker or
        // wakes this one finds the list free.
        let mut turns = TURNS.with(|turns| std::mem::take(&mut *turns.borrow_mut()));
        let mut soonest = None::<Duration>;
        turns.retain_mut(|turn| {
            NEXT.with(|next| next.set(None));
            let going = turn();
            // A worker that asked for no wait wants its next turn now, however
            // long another is content to wait.
            let wait = NEXT.with(|next| next.get()).unwrap_or_default();
            if going {
                soonest = Some(soonest.map_or(wait, |had| had.min(wait)));
            }
            going
        });
        TURNS.with(|now| {
            let mut now = now.borrow_mut();
            turns.append(&mut now);
            *now = turns;
        });
        if TURNS.with(|turns| turns.borrow().is_empty()) {
            return;
        }
        schedule(soonest.unwrap_or_default().as_secs_f64() * 1000.0);
    }
}
