//! Where the [`AudioEngine`] lives: a thread of its own on the desktop, since
//! the cpal stream is not `Send`, and turns of its own in the browser, where
//! there is only the page's thread. Either way the engine is touched only
//! here, by the commands sent to it.

use super::Cmd;
use lumit_audio::meter::Meters;
use lumit_audio::{AudioEngine, ClockHandle};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

/// What an engine that opened tells the rest: its clock, its meters and the
/// rate media decodes at.
type Opened = (ClockHandle, Arc<Meters>, u32);

/// Open the engine on its own thread, which then lives on the commands from
/// `rx` until the sender goes. `None` when there is no device.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn open(wanted: Option<String>, rx: Receiver<Cmd>) -> Option<Opened> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let engine = match AudioEngine::new_on(wanted.as_deref()) {
            Ok(engine) => {
                let _ = ready_tx.send(Some((
                    engine.clock(),
                    engine.meters(),
                    engine.device_rate(),
                )));
                engine
            }
            Err(_) => {
                let _ = ready_tx.send(None);
                return; // no device: the thread ends, the state stays Unavailable
            }
        };
        // The engine lives here for the session; each command is a plan swap
        // or an atomic store — nothing slow, nothing that can block the
        // realtime callback.
        while let Ok(cmd) = rx.recv() {
            apply(&engine, cmd);
        }
    });
    ready_rx.recv().ok().flatten()
}

/// How soon the page's sound takes its next turn: often while it plays, so
/// the ring never runs dry, and less often while it is quiet. A command
/// brings the next turn forward.
#[cfg(target_arch = "wasm32")]
const PLAYING_TURN: std::time::Duration = std::time::Duration::from_millis(10);
#[cfg(target_arch = "wasm32")]
const QUIET_TURN: std::time::Duration = std::time::Duration::from_millis(40);

/// The soonest the sound is fed again when nothing was sent to it. Every
/// worker takes a turn whenever any of them does, and playback asks for
/// turns far more often than this.
#[cfg(target_arch = "wasm32")]
const SOONEST_TURN: std::time::Duration = std::time::Duration::from_millis(4);

/// Open the page's output, and give the engine turns of its own that take
/// the commands from `rx` and keep the sound fed, until the sender goes.
#[cfg(target_arch = "wasm32")]
pub(super) fn open(wanted: Option<String>, rx: Receiver<Cmd>) -> Option<Opened> {
    let engine = AudioEngine::new_on(wanted.as_deref()).ok()?;
    let opened = (engine.clock(), engine.meters(), engine.device_rate());
    let mut fed = crate::turns::long_ago(SOONEST_TURN);
    crate::turns::start(move || {
        let mut sent = false;
        loop {
            match rx.try_recv() {
                Ok(cmd) => apply(&engine, cmd),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return false,
            }
            sent = true;
        }
        if sent || fed.elapsed() >= SOONEST_TURN {
            engine.turn();
            fed = web_time::Instant::now();
        }
        crate::turns::pause(if engine.is_playing() {
            PLAYING_TURN
        } else {
            QUIET_TURN
        });
        true
    });
    Some(opened)
}

fn apply(engine: &AudioEngine, cmd: Cmd) {
    match cmd {
        Cmd::Load { plan, start, play } => {
            engine.load_plan(plan);
            if let Some(s) = start {
                engine.seek_seconds(s);
            }
            if play {
                engine.play();
            }
        }
        Cmd::Swap(plan) => engine.swap_plan(plan),
        Cmd::Play => engine.play(),
        Cmd::Pause => engine.pause(),
        Cmd::Mute(m) => engine.set_muted(m),
        Cmd::Slow(s) => engine.set_slow(s),
        Cmd::Seek(s) => engine.seek_seconds(s),
        Cmd::Scrub { at, len } => engine.scrub(at, len),
        Cmd::Unload => engine.unload(),
    }
}

/// Do `work` away from the caller: on a thread of its own on the desktop,
/// and in a turn of the page's straight after this one in the browser.
pub(super) fn spawn(work: impl FnOnce() + Send + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(work);
    #[cfg(target_arch = "wasm32")]
    {
        let mut work = Some(work);
        crate::turns::start(move || {
            if let Some(work) = work.take() {
                work();
            }
            false
        });
    }
}
