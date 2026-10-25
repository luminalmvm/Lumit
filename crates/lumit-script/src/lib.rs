//! Commands: the named edits scripts, the command line and MCP make.
//!
//! # In plain terms
//!
//! A command is one name, some JSON arguments and a JSON answer:
//! `layer.add` with `{"comp": "Main", "kind": "camera"}` adds a camera and
//! answers with the layer it made. Each one becomes one op, or one
//! `Op::Batch`, committed through the project's [`DocumentStore`], so a
//! command is one undo step and every front end edits the document the same
//! way. They all go through [`dispatch`], and [`COMMANDS`] lists them with a
//! JSON Schema each, which is what MCP offers as its tools.
//!
//! What only the program around the document can do, saving, reading footage
//! files and exporting, goes through [`Host`].
//!
//! Commands run on the caller's thread and commit through the store, as a
//! shared project's threads do (docs/14 §1).

pub mod commands;
mod include;
pub mod live;
pub mod mcp;
mod reach;
pub mod reference;
pub mod script;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use lumit_core::model::{Composition, Layer};
use lumit_core::time::{CompTime, Duration, FrameRate};
use lumit_core::{ops::Op, store::DocumentStore};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// What a command needs from the program it runs in. Lumit has one, and the
/// command line has its own.
pub trait Host {
    /// The open project's store. Every command commits through it.
    fn store(&self) -> &DocumentStore;

    /// Where the project was last saved, or `None` before its first save.
    fn path(&self) -> Option<PathBuf>;

    /// Save the project to `path`, or where it was last saved when that is
    /// `None`. Answers where it went.
    fn save(&self, path: Option<&Path>) -> Result<PathBuf, ScriptError>;

    /// Bring `paths` in as footage items. Answers the new items' ids, in
    /// order.
    fn import_footage(&self, paths: &[PathBuf]) -> Result<Vec<Uuid>, ScriptError>;

    /// The layers placing footage item `item` in `comp` makes, topmost first,
    /// sized and timed from the file. Nothing is added: the command does that.
    fn footage_layers(&self, comp: &Composition, item: Uuid) -> Result<Vec<Layer>, ScriptError>;

    /// Start exporting `comp` to `out`, or wherever the host puts an export
    /// when that is `None`. Answers a number to ask after it with.
    fn export_start(&self, comp: Uuid, out: Option<&Path>) -> Result<u64, ScriptError>;

    /// How export `job` is going.
    fn export_status(&self, job: u64) -> Result<ExportStatus, ScriptError>;

    /// Stop export `job`. Stopping one that has finished does nothing.
    fn export_cancel(&self, job: u64) -> Result<(), ScriptError>;

    /// Render frame `frame` of `comp` at `scale` of its size to a new PNG at
    /// `out`, never over a file. Answers the picture's width and height. A
    /// host that does not say otherwise renders nothing.
    fn render_frame(
        &self,
        comp: Uuid,
        frame: u64,
        scale: f32,
        out: &Path,
    ) -> Result<(u32, u32), ScriptError> {
        let _ = (comp, frame, scale, out);
        Err(ScriptError::Failed(
            "Frames cannot be rendered here.".into(),
        ))
    }

    /// Show `comp` in the Viewer. Where there is no Viewer, as on the command
    /// line, this does nothing.
    fn open_comp(&self, comp: Uuid) -> Result<(), ScriptError> {
        let _ = comp;
        Ok(())
    }

    /// The comp open in the Viewer, which After Effects calls the active
    /// item. Where there is no Viewer there is none.
    fn active_comp(&self) -> Option<Uuid> {
        None
    }

    /// The ops that make `parent` the parent of `layer` in `comp`, or leave it
    /// with none. By default only the parent changes, so the layer jumps with
    /// it; a host that can place layers also changes the layer's transform so
    /// it stays where it is at comp time `at`, as After Effects does.
    fn parent_ops(&self, comp: Uuid, layer: Uuid, parent: Option<Uuid>, at: CompTime) -> Vec<Op> {
        let _ = at;
        vec![Op::SetLayerParent {
            comp,
            layer,
            parent,
        }]
    }

    /// What stops a script run through MCP at its next step, when the host
    /// has a Stop for it. By default nothing does.
    fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::default()
    }

    /// How long the source of `layer` in comp `comp` runs, when the host can
    /// read it: a footage file's length. By default it is not known.
    fn source_length(&self, comp: Uuid, layer: &Layer) -> Option<Duration> {
        let _ = (comp, layer);
        None
    }

    /// Footage item `item`'s file as it is read, when the host can read it.
    /// By default it is not known.
    fn footage_info(&self, item: Uuid) -> Option<FootageInfo> {
        let _ = item;
        None
    }
}

/// A footage item's file as it is read, interpretation and all: what After
/// Effects' `FootageItem` answers for its size, rate and length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootageInfo {
    /// The picture's size, nought for sound alone.
    pub width: u32,
    pub height: u32,
    /// The rate it plays at. `None` for a still or for sound alone.
    pub frame_rate: Option<FrameRate>,
    /// How long it runs. `None` for a still.
    pub duration: Option<Duration>,
}

/// Where an export has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// An export's progress, as `export.status` answers it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportStatus {
    pub state: ExportState,
    /// Frames written so far, out of `total`.
    pub done: u64,
    pub total: u64,
    /// The file being written.
    pub path: PathBuf,
    /// Why it failed, when it did.
    pub message: Option<String>,
}

/// Why a command did nothing. Each has a [`code`](ScriptError::code) a
/// caller can match on, and a message a person can read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("There is no command called {0}.")]
    UnknownCommand(String),
    #[error("{0}")]
    BadArgs(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Failed(String),
    #[error("Stopped.")]
    Stopped,
}

impl ScriptError {
    /// The error's stable name: `unknown-command`, `bad-args`, `not-found`,
    /// `failed` or `stopped`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            ScriptError::UnknownCommand(_) => "unknown-command",
            ScriptError::BadArgs(_) => "bad-args",
            ScriptError::NotFound(_) => "not-found",
            ScriptError::Failed(_) => "failed",
            ScriptError::Stopped => "stopped",
        }
    }
}

impl From<lumit_core::ops::OpError> for ScriptError {
    /// The op's reason as a sentence: "the layer is locked" reads as
    /// "The layer is locked."
    fn from(e: lumit_core::ops::OpError) -> Self {
        let reason = e.to_string();
        let mut letters = reason.chars();
        let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
        ScriptError::Failed(format!(
            "{}{}.",
            first.unwrap_or_default(),
            letters.as_str()
        ))
    }
}

/// One entry of [`COMMANDS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    /// One line saying what it does.
    pub summary: &'static str,
    /// A JSON Schema for its arguments, as JSON text.
    pub schema: &'static str,
}

/// Run command `name` with `args` (an object, or null for none) and answer
/// what it did. Its paths go wherever they say, as the caller is the person.
pub fn dispatch(host: &dyn Host, name: &str, args: Value) -> Result<Value, ScriptError> {
    commands::run(host, None, name, args)
}

/// Run `run` as one undo step: whatever it commits is undone together, and
/// taken back straight away when it fails or is stopped, so a run that went
/// wrong leaves nothing half done and nothing to redo.
///
/// Refused while another undo group is open, so the run owns its group. Any
/// local commit made during the run joins the group, so the app keeps the
/// interface from editing while a run is going.
pub fn one_step<T>(
    store: &DocumentStore,
    run: impl FnOnce() -> Result<T, ScriptError>,
) -> Result<T, ScriptError> {
    if !store.begin_own_undo_group() {
        return Err(ScriptError::Failed(
            "Another change is still being made. Try again when it has finished.".into(),
        ));
    }
    let out = run();
    if out.is_ok() {
        store.end_undo_group();
    } else {
        // If an edit will not come off, the store keeps the run as one undo
        // step instead, and why the run stopped is still the thing to hear.
        let _ = store.abandon_undo_group();
    }
    out
}
