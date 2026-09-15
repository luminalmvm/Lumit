//! Every way the addon machinery can refuse, and nothing else.
//!
//! # Thread role and contract
//!
//! Pure data. No IO, no threads, no interior mutability: an [`MlError`] is
//! made on whichever thread noticed and is carried back to the caller
//! (14-ENGINEERING-RULES §1.1).

use crate::manifest::Task;

/// A refusal from the model runtime, the pack store or a session.
///
/// None of these is a fault. Every one names what was missing or what the file
/// said, and the caller shows a calm sentence (docs/impl/addons.md §9). Only
/// three carry text, and that text is the library's or the manifest's own
/// words riding the one channel where they are allowed across the bridge: the
/// badge's detail slot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MlError {
    /// No model runtime is installed, so nothing can be opened at all.
    #[error("no model runtime is installed")]
    RuntimeMissing,

    /// The runtime is installed and would not load. Carries the library's own
    /// sentence, which is the only thing that ever says why.
    #[error("the model runtime would not load: {0}")]
    RuntimeFailed(String),

    /// The runtime has loaded, and the library stays open for the life of the
    /// process, so its folder can be neither replaced nor deleted yet.
    #[error("the model runtime is in use until Lumit is restarted")]
    RuntimeInUse,

    /// Nothing installed does this task.
    #[error("no model pack is installed for the {0} task")]
    PackMissing(Task),

    /// A pack folder is there and its manifest will not read, or a file the
    /// manifest lists is missing or the wrong size.
    #[error("that model pack cannot be read")]
    PackUnreadable,

    /// An addon of that id is not installed.
    #[error("no addon of that id is installed")]
    NotInstalled,

    /// An install was refused: the manifest, the files handed over, or the
    /// unpack. Carries the engine's own sentence saying which.
    #[error("{0}")]
    Invalid(String),

    /// Another install is already running. One at a time, never a queue (§9).
    #[error("another addon is installing")]
    Busy,

    /// The model itself failed while it was running. Carries ONNX Runtime's
    /// own words.
    #[error("the model failed: {0}")]
    ModelFailed(String),

    /// A tensor the manifest names is not one the file has, or its shape is
    /// not the one the engine feeds.
    #[error("the model does not take the tensors this pack's manifest names")]
    ShapeMismatch,

    /// The job was cancelled between frames.
    #[error("cancelled")]
    Cancelled,
}
