//! File > Run script, and other programs driving Lumit: work on the open
//! project from outside the interface.

use std::collections::HashMap;

use flutter_rust_bridge::frb;

use crate::api::{composition::CompositionReference, project::ProjectReference, BridgeError};
use crate::frb_generated::StreamSink;

/// What a running script says.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeScriptEvent {
    /// It asks for a file. [`answer_script_file`] gives it one.
    ChooseFile {
        title: String,
        /// The extensions the file may have, without the dot. Empty for any.
        extensions: Vec<String>,
    },
    /// It shows `message`, and carries on without waiting.
    Alert { message: String },
    /// It shows an After Effects ScriptUI window, or shows it again as it now
    /// is. `tree` is the window as JSON: controls with ids, kinds, texts and
    /// values, groups and panels holding more. [`answer_script_dialog`] says
    /// what the person did next.
    ShowDialog { tree: String },
    /// It has closed its window.
    CloseDialog,
    /// It opens `comp` in the Viewer.
    OpenComp { comp: CompositionReference },
    /// It has ended, and its edits are one undo step, or none when it failed
    /// or was stopped. `error` says why it did not finish, and `line` where,
    /// when the script's own text is to blame. `stopped` is Stop's doing.
    Finished {
        /// What it printed.
        log: Vec<String>,
        stopped: bool,
        error: Option<String>,
        line: Option<u32>,
    },
}

impl ProjectReference {
    /// Run the script at `path` on this project, on a thread of its own.
    /// Answers at once, false when a script is running already. `comp` is
    /// the one open in the Viewer, if any. `events` hears the rest, and the
    /// edits arrive on the change stream as usual. `args` is what the script
    /// reads as `lumit.args`.
    ///
    /// The interface must not edit until it hears `Finished`, since the
    /// whole run is one undo step and an edit made meanwhile would join it.
    #[frb(sync)]
    pub fn run_script(
        &self,
        path: String,
        comp: Option<CompositionReference>,
        args: HashMap<String, String>,
        events: Option<StreamSink<BridgeScriptEvent>>,
    ) -> Result<bool, BridgeError> {
        let comp = comp.map(|c| c.id);
        let args = args.into_iter().collect();
        crate::script::start(self.clone(), path.into(), comp, args, move |event| {
            events.as_ref().is_some_and(|sink| sink.add(event).is_ok())
        })
    }
}

/// Stop the running script at its next step. Harmless when none is.
#[frb(sync)]
pub fn stop_script() {
    crate::script::stop();
}

/// The file a script's `ChooseFile` asked for, or `None` when the person
/// cancelled.
#[frb(sync)]
pub fn answer_script_file(path: Option<String>) {
    crate::script::answer(path);
}

/// What the person did in a script's window: `{"id": the control pressed or
/// changed, "values": {id: value}}` as JSON, each value a text, a tick, a
/// number or a list's chosen place. `None` when they closed the window.
#[frb(sync)]
pub fn answer_script_dialog(event: Option<String>) {
    crate::script::answer(event);
}

/// Lumit's own Scripts folder, made the first time it is asked for. `None`
/// when this machine has nowhere for it.
#[frb(sync)]
pub fn scripts_folder() -> Option<String> {
    let folder = lumit_project::scripts_dir()?;
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder.to_string_lossy().into_owned())
}

/// What File > Scripts lists from `folder`: each `.js` and `.jsx` file in it
/// and the folders inside it, as a path under `folder` with `/` between the
/// parts, in name order.
#[frb(sync)]
pub fn list_scripts(folder: String) -> Vec<String> {
    crate::script::list(std::path::Path::new(&folder))
}

/// What happens on the endpoint other programs drive Lumit through.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeLiveEvent {
    /// A program has connected and proved it holds the secret. `name` is
    /// what it calls itself, empty until it says.
    Attached {
        name: String,
    },
    /// It has gone, or was let go.
    Detached,
    /// It is running a script, `name`, as one undo step. The interface must
    /// not edit until `ScriptEnded`, as for File > Run script.
    ScriptStarted {
        name: String,
    },
    ScriptEnded,
    /// It opens `comp` in the Viewer.
    OpenComp {
        comp: CompositionReference,
    },
    /// Its script shows `message`.
    Alert {
        message: String,
    },
}

/// Let other programs on this computer drive Lumit: listen on a local
/// socket, never a network port, for one program at a time that holds the
/// secret written beside it. `events` hears who connects and what they do,
/// and edits arrive on the change stream as usual. False when it could not
/// start.
pub fn start_live_endpoint(events: Option<StreamSink<BridgeLiveEvent>>) -> bool {
    crate::live::start(move |event| events.as_ref().is_some_and(|sink| sink.add(event).is_ok()))
}

/// Stop listening and let go of the program connected. Harmless when off.
pub fn stop_live_endpoint() {
    crate::live::stop();
}

/// The busy card for a program's script is up, after `ScriptStarted`. The
/// script waits for this, so nothing from the window lands in its undo step.
#[frb(sync)]
pub fn live_script_shown() {
    crate::live::shown();
}

/// Let go of the program connected. It may connect again.
#[frb(sync)]
pub fn disconnect_live_program() {
    crate::live::disconnect();
}
