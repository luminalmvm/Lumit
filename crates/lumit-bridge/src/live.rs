//! "Let other programs drive Lumit": Lumit's end of the live endpoint.
//!
//! `lumit_script::live` listens and checks the secret. This is the thread it
//! runs on and what each line does there. A tool call works on the project
//! that is open, through the same [`AppHost`] File > Run script uses, and
//! commits through its store, so every edit reaches the interface and the
//! History as usual. Each call takes the script's place while it works
//! ([`crate::script::hold`]), so it never lands inside a script's undo step,
//! and Stop on the busy card reaches a `script_run`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lumit_script::Host;
use serde_json::Value;

use crate::api::{
    project::ProjectReference,
    script::{BridgeLiveEvent, BridgeScriptEvent},
    state::PROJECTS,
};
use crate::script::AppHost;

const NO_PROJECT: &str = "No project is open in Lumit.";
pub(crate) const SCRIPT_RUNNING: &str =
    "Lumit is running a script. Try again when it has finished.";
pub(crate) const BUSY: &str = "Lumit is busy with another change. Try again in a moment.";
const NOT_SHOWN: &str = "Lumit did not answer in time. Try again.";

/// How long an edit waits for a change being made in the window, such as a
/// drag, to finish.
const DRAG_WAIT: Duration = Duration::from_secs(2);
/// How long a script waits for the window to put its busy card up.
const CARD_WAIT: Duration = Duration::from_secs(5);

/// Where the interface says the busy card for a program's script is up.
static SHOWN: Mutex<Option<SyncSender<()>>> = Mutex::new(None);

/// The endpoint while it is on.
struct Live {
    stop: Arc<AtomicBool>,
    /// Lets go of the program that is connected.
    leave: Arc<AtomicBool>,
    /// Stops a script the program is running.
    run: Arc<AtomicBool>,
    file: PathBuf,
    secret: lumit_script::live::Secret,
}

impl Live {
    fn end(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.run.store(true, Ordering::Relaxed);
        // Gone at once, so no program finds a name nobody answers.
        lumit_script::live::remove(&self.file, &self.secret);
    }
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

/// Start listening, in place of any endpoint already on. `tell` hears who
/// connects and what they do, and answers false when nobody is listening.
/// False when it could not start.
pub(crate) fn start(tell: impl Fn(BridgeLiveEvent) -> bool + Send + 'static) -> bool {
    stop();
    let Some(file) = lumit_project::live_endpoint_file() else {
        return false;
    };
    let Ok((listener, secret)) = lumit_script::live::open(&file) else {
        return false;
    };
    let live = Live {
        stop: Arc::default(),
        leave: Arc::default(),
        run: Arc::default(),
        file,
        secret: secret.clone(),
    };
    let (stop, leave, run) = (live.stop.clone(), live.leave.clone(), live.run.clone());
    let spawned = std::thread::Builder::new()
        .name("live".into())
        .spawn(move || {
            // The listener goes with this thread, and with it the name.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                serve(&listener, &secret, &stop, &leave, &run, &tell);
            }));
        });
    let Ok(mut on) = LIVE.lock() else {
        live.end();
        return false;
    };
    if spawned.is_err() {
        live.end();
        return false;
    }
    *on = Some(live);
    true
}

/// Stop listening and let go of the program connected. Harmless when off.
pub(crate) fn stop() {
    let live = LIVE.lock().ok().and_then(|mut on| on.take());
    if let Some(live) = live {
        live.end();
    }
}

/// The busy card for a program's script is up, so the script may start.
pub(crate) fn shown() {
    if let Some(shown) = SHOWN.lock().ok().and_then(|mut s| s.take()) {
        let _ = shown.try_send(());
    }
}

/// Let go of the program connected, and stop its script.
pub(crate) fn disconnect() {
    if let Ok(on) = LIVE.lock() {
        if let Some(live) = on.as_ref() {
            live.leave.store(true, Ordering::Relaxed);
            live.run.store(true, Ordering::Relaxed);
        }
    }
}

fn serve(
    listener: &lumit_script::live::Listener,
    secret: &lumit_script::live::Secret,
    stop: &AtomicBool,
    leave: &AtomicBool,
    run: &Arc<AtomicBool>,
    tell: &dyn Fn(BridgeLiveEvent) -> bool,
) {
    // What a script says, for the interface. Nobody can be asked for a file.
    let said = |event: BridgeScriptEvent| match event {
        BridgeScriptEvent::OpenComp { comp } => tell(BridgeLiveEvent::OpenComp { comp }),
        BridgeScriptEvent::Alert { message } => tell(BridgeLiveEvent::Alert { message }),
        _ => false,
    };
    let mut host: Option<AppHost> = None;
    lumit_script::live::serve(
        listener,
        secret,
        stop,
        leave,
        &mut |on| {
            tell(if on {
                BridgeLiveEvent::Attached {
                    name: String::new(),
                }
            } else {
                BridgeLiveEvent::Detached
            });
        },
        &mut |line| {
            // A Stop from before this line is not meant for it.
            run.store(
                stop.load(Ordering::Relaxed) || leave.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            answer(line, &mut host, run, &said, tell)
        },
    );
}

/// The reply to one line from the program.
fn answer<'a>(
    line: &str,
    host: &mut Option<AppHost<'a>>,
    run: &Arc<AtomicBool>,
    said: &'a dyn Fn(BridgeScriptEvent) -> bool,
    tell: &dyn Fn(BridgeLiveEvent) -> bool,
) -> Option<String> {
    let asked: Value = serde_json::from_str(line).unwrap_or_default();
    let method = asked.get("method").and_then(Value::as_str);
    let name = |at: &str| asked.pointer(at).and_then(Value::as_str);
    if method == Some("initialize") {
        if let Some(name) = name("/params/clientInfo/name") {
            tell(BridgeLiveEvent::Attached { name: name.into() });
        }
    }
    // Only a tool call touches the project.
    if method != Some("tools/call") {
        return lumit_script::mcp::answer_without(line, NO_PROJECT);
    }
    let Some(project) = open_project() else {
        return lumit_script::mcp::answer_without(line, NO_PROJECT);
    };
    // Made again when another project is opened.
    if host.as_ref().is_none_or(|h| h.project.id != project.id) {
        let store = project
            .state()
            .ok()
            .and_then(|state| Some(state.read().ok()?.store.clone()));
        let Some(store) = store else {
            return lumit_script::mcp::answer_without(line, NO_PROJECT);
        };
        *host = Some(AppHost::new(project, store, run.clone(), said));
    }
    let host = host.as_ref()?;
    let Some(_held) = crate::script::hold(run.clone()) else {
        return lumit_script::mcp::answer_without(line, SCRIPT_RUNNING);
    };
    let script = name("/params/name") == Some("script_run");
    if script {
        // The card goes up before the run's undo step opens, or a click
        // meanwhile would land inside it.
        let (shown, heard) = mpsc::sync_channel(1);
        if let Ok(mut waiting) = SHOWN.lock() {
            *waiting = Some(shown);
        }
        let name = name("/params/arguments/name").unwrap_or_default().into();
        let told = tell(BridgeLiveEvent::ScriptStarted { name });
        if told && heard.recv_timeout(CARD_WAIT).is_err() {
            tell(BridgeLiveEvent::ScriptEnded);
            return lumit_script::mcp::answer_without(line, NOT_SHOWN);
        }
    } else if !drag_over(host) {
        return lumit_script::mcp::answer_without(line, BUSY);
    }
    let reply = lumit_script::mcp::answer(line, host);
    if script {
        tell(BridgeLiveEvent::ScriptEnded);
    }
    reply
}

/// Wait for a change being made in the window, such as a drag, to finish,
/// so an edit from here does not join its undo step. False when it goes on
/// past [`DRAG_WAIT`].
pub(crate) fn drag_over(host: &AppHost) -> bool {
    let store = host.store();
    let started = Instant::now();
    // Opening a group of our own works only when none is open, and an empty
    // one closes leaving the History as it was.
    while !store.begin_own_undo_group() {
        if started.elapsed() > DRAG_WAIT {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    store.end_undo_group();
    true
}

/// The project open in the window: the newest, since opening one closes
/// the one before.
fn open_project() -> Option<ProjectReference> {
    let projects = PROJECTS.read().ok()?;
    projects
        .keys()
        .next_back()
        .map(|id| ProjectReference::new(*id))
}
