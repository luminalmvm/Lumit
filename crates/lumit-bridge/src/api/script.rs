//! File > Run script: a script run on the open project.

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
    /// Answers at once, false when a script is running already. `events`
    /// hears the rest, and the edits arrive on the change stream as usual.
    ///
    /// The interface must not edit until it hears `Finished`, since the
    /// whole run is one undo step and an edit made meanwhile would join it.
    #[frb(sync)]
    pub fn run_script(
        &self,
        path: String,
        events: Option<StreamSink<BridgeScriptEvent>>,
    ) -> Result<bool, BridgeError> {
        crate::script::start(self.clone(), path.into(), move |event| {
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
    crate::script::answer(path.map(Into::into));
}
