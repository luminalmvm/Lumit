use std::{error::Error, fmt};

use lumit_core::OpError;

pub mod assets;
pub mod audio;
// Always compiled, feature or no feature: the generated Dart is one shape
// whatever the build (docs/17 §Feature gates), so an API function that
// disappears with a feature breaks `--no-default-features` at the generated
// call site rather than at anything a person wrote (K-273). Detection itself
// needs the audio pipeline and says so calmly when the build has none.
pub mod beats;
pub mod cache;
pub mod colour;
pub mod composition;
pub mod effect;
pub mod export;
pub mod expressions;
pub mod folder;
pub mod footage;
pub mod graph;
pub mod import;
pub mod keymap;
pub mod layer;
pub mod project;
pub mod project_item;
pub mod retime;
pub mod shell;
pub mod solid;
pub mod state;
pub mod system;
pub mod track;

mod worker_thread;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum BridgeError {
    InvalidProject,
    InvalidComp,
    InvalidItem,
    InvalidLayer,
    /// A folder was asked to move inside itself, or inside one of its own
    /// descendants. Refused rather than applied: that branch would leave the
    /// panel root with nothing left to drag it back by.
    FolderCycle,
    /// A media path could not be resolved, or a relink found nothing to point at.
    MediaPathUnresolved,
    /// A frame rate of zero, or one whose frame count cannot be expressed.
    InvalidFrameRate,
    /// Save was asked to write a project that has never been saved, without being
    /// told where. The caller has to pick a path.
    NoProjectPath,
    /// A rename was given a blank name. Refused rather than applied, so a row
    /// cannot lose its label.
    EmptyName,
    /// No parameter of that id on the effect.
    InvalidParam,
    /// No effect of that id in the layer's stack — a reference that outlived the
    /// effect it named.
    InvalidEffect,
    /// No built-in effect goes by that match name.
    UnknownEffectName,
    /// The value written to a parameter is of a different kind from the
    /// parameter. A parameter's kind is the effect's schema to declare, not the
    /// panel's to change, so this is refused rather than applied.
    ParamKindMismatch,
    /// A keyframed value whose keys are not a curve the engine can evaluate:
    /// none at all, an invalid time, or times that do not strictly ascend.
    InvalidKeyframes,
    /// A time whose denominator is zero or negative — a span or marker built
    /// wrongly by the caller. Refused rather than normalised: quietly fixing it
    /// would put the thing somewhere nobody asked for.
    InvalidTime,
    /// `set_transforms` was given a different number of properties and values.
    MismatchedTransforms,
    /// A blend-mode index outside the list `list_blend_modes` hands out.
    InvalidBlendMode,
    /// A scope's colour list was not five `[r, g, b]` triples.
    InvalidScopeColours,
    /// A region of interest that is not four finite comp fractions, or that
    /// comes to less than a pixel either way (K-687). There is no composition
    /// inside it to crop to.
    InvalidRegion,
    /// The text handed to `load_preset` is not a `.lumfx` document.
    InvalidPreset,
    /// The text handed to `set_shader_graph` is not a graph document — a
    /// caller bug, never a user state (custom-shader.md §4, CS4).
    InvalidShaderGraph,
    /// The export could not start — already running, no GPU, or a spec the
    /// encoder will not take. Carries the engine's own words.
    ExportFailed(String),
    /// No audio pipeline on this machine (no adapter, or a build without one).
    NoAudioPipeline,
    /// The composition has no audible sources to analyse.
    NoAudio,
    /// The layer has no retiming to edit.
    NotRetimed,
    /// The retime curve is a ramp or an explicit map, so there is no single
    /// speed to set — writing one would discard the shape.
    RetimeVaries,
    /// The edit named a text layer and the layer is not one.
    NotText,
    /// The edit named a camera layer and the layer is not one.
    NotCamera,
    /// Text to shapes was asked of a layer whose words come to nothing — an
    /// empty line, or one made only of spaces (K-608). There is no art to make,
    /// so the command says so rather than leaving an empty layer behind.
    NothingToConvert,
    /// Analyse was pressed while another analysis is already running. One at a
    /// time is deliberate (K-417): two disk-bound jobs share one drive and
    /// halve each other.
    AnalysisBusy,
    /// Convert to keyframes was asked of a camera with no solve link to bake,
    /// or one whose link resolves nowhere.
    NotLinked,
    /// The selection names no solved point, so there is nowhere to put a
    /// layer.
    NoSolve,
    /// The razor was pointed at a layer that is not a Sequence layer.
    NotSequence,
    /// Only a Footage layer converts to a Sequence layer.
    NotFootage,
    /// The adjustment switch (K-537) was asked of a layer with no picture to
    /// set aside — a Camera, a Light, a Null or an Audio layer. Every layer
    /// that shows something in the Viewer takes it; the cell is not drawn on
    /// the four that do not.
    NotConvertible,
    /// A Sequence layer's retiming belongs to its clips, not to the layer
    /// (K-075), so it has no Retime channel to switch on.
    NotRetimeable,
    /// Converting back to a plain Footage layer needs one clip, and this row
    /// has several — which of them the layer would become is the user's
    /// decision, not the command's.
    ManyClips,
    /// No clip sits under the playhead.
    NoClipThere,
    /// A mask path with fewer than two vertices — not a shape.
    EmptyPath,
    /// The edit named a mask this layer does not have.
    NoSuchMask,
    /// A paint stroke with no points in it (K-227).
    EmptyStroke,
    /// No stroke of that id on this layer.
    NoSuchStroke,
    /// The layer is not a shape layer (K-237).
    NotShape,
    /// The razor was pointed at a time outside the layer's span, or at one of
    /// its ends — either way there is no second layer to make.
    NothingToSplit,
    /// There is no cut to make in the clip under the playhead. An eased speed
    /// ramp is no longer one of these (K-573): a cubic splits into two cubics
    /// that are the same curve, so the razor goes through a ramp exactly. What
    /// is left is the moment landing on one of the clip's own ends — an end is
    /// not a cut — and a retime driven by an expression, which cannot be split
    /// without rewriting what was typed.
    UncuttableClip,
    /// A staged effect stack no longer matches the document's — something else
    /// added, removed or reordered an effect while it was being edited.
    StaleEffectStack,
    /// The text offered as a keyboard chord is not one — empty, or naming a
    /// modifier this build does not know. Carries the keymap's own words.
    InvalidKeyChord(String),
    /// The JSON offered as a keymap is not one. Refused whole rather than
    /// applied in part, so a corrupt stored blob leaves the live keymap alone.
    InvalidKeymapFile(String),
    ReadFailed,
    WriteFailed,
    InvalidWorkerState,
    OpError(OpError),
}

impl Error for BridgeError {}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let _ = match &self {
            BridgeError::ReadFailed => write!(f, "Read Failed"),
            BridgeError::InvalidProject => write!(f, "Invalid ProjectItem"),
            BridgeError::InvalidComp => write!(f, "Invalid Comp"),
            BridgeError::InvalidItem => write!(f, "Invalid Item"),
            BridgeError::InvalidLayer => write!(f, "Invalid Layer"),
            BridgeError::EmptyName => write!(f, "The name cannot be empty"),
            BridgeError::InvalidFrameRate => write!(f, "Invalid frame rate"),
            BridgeError::NoProjectPath => {
                write!(
                    f,
                    "This project has never been saved, so a path is required"
                )
            }
            BridgeError::FolderCycle => {
                write!(f, "A folder cannot be filed inside itself")
            }
            BridgeError::MediaPathUnresolved => write!(f, "Nothing to relink at that path"),
            BridgeError::InvalidParam => write!(f, "No such effect parameter"),
            BridgeError::InvalidEffect => write!(f, "No such effect on this layer"),
            BridgeError::UnknownEffectName => write!(f, "No built-in effect by that name"),
            BridgeError::ParamKindMismatch => {
                write!(f, "That value is the wrong kind for this effect parameter")
            }
            BridgeError::InvalidKeyframes => write!(
                f,
                "A keyframed value needs at least one key, in ascending time order"
            ),
            BridgeError::InvalidTime => write!(f, "That time is not a valid duration"),
            BridgeError::MismatchedTransforms => {
                write!(f, "One value is needed per transform property")
            }
            BridgeError::InvalidBlendMode => write!(f, "No blend mode at that index"),
            BridgeError::InvalidScopeColours => {
                write!(f, "A scope needs five red/green/blue triples")
            }
            BridgeError::InvalidRegion => {
                write!(f, "That region of interest is smaller than a pixel")
            }
            BridgeError::InvalidPreset => write!(f, "That is not a valid effect preset"),
            BridgeError::InvalidShaderGraph => write!(f, "That is not a shader graph"),
            BridgeError::InvalidKeyChord(why) => {
                write!(f, "That is not a keyboard shortcut: {why}")
            }
            BridgeError::InvalidKeymapFile(why) => write!(f, "That is not a keymap file: {why}"),
            BridgeError::ExportFailed(why) => write!(f, "{why}"),
            BridgeError::NoAudioPipeline => {
                write!(f, "This machine has no audio pipeline")
            }
            BridgeError::NoAudio => {
                write!(f, "There is no audio in this composition")
            }
            BridgeError::NotRetimed => write!(f, "That layer is not retimed"),
            BridgeError::RetimeVaries => {
                write!(f, "That layer's speed varies; edit it in the Retime graph")
            }
            BridgeError::NotText => write!(f, "That is not a text layer"),
            BridgeError::NotCamera => write!(f, "That is not a camera layer"),
            BridgeError::NothingToConvert => {
                write!(f, "That layer has no words to convert")
            }
            BridgeError::AnalysisBusy => write!(f, "Another analysis is already running"),
            BridgeError::NotLinked => write!(f, "That camera has no solve to bake"),
            BridgeError::NoSolve => write!(f, "Nothing has been solved at those points"),
            BridgeError::NotSequence => write!(f, "That is not a sequence layer"),
            BridgeError::NotFootage => {
                write!(f, "Only footage layers convert to sequenced")
            }
            BridgeError::NotConvertible => {
                write!(
                    f,
                    "Only a layer with a picture of its own can become an adjustment layer"
                )
            }
            BridgeError::NotRetimeable => write!(
                f,
                "A sequence layer retimes its clips, not the whole layer — open it and use its speed graph"
            ),
            BridgeError::ManyClips => write!(
                f,
                "This layer holds several clips — delete all but the one to keep first"
            ),
            BridgeError::NoClipThere => write!(f, "No clip under the playhead"),
            BridgeError::EmptyPath => write!(f, "A mask needs at least two points"),
            BridgeError::NoSuchMask => write!(f, "No such mask on this layer"),
            BridgeError::EmptyStroke => write!(f, "A paint stroke needs at least one point"),
            BridgeError::NoSuchStroke => write!(f, "No such paint stroke on this layer"),
            BridgeError::NotShape => write!(f, "That layer is not a shape layer"),
            BridgeError::NothingToSplit => {
                write!(f, "That time is not inside the layer")
            }
            BridgeError::UncuttableClip => {
                write!(f, "There is no cut to make at that moment")
            }
            BridgeError::StaleEffectStack => {
                write!(f, "The effect stack changed while it was being edited")
            }
            BridgeError::WriteFailed => write!(f, "Write Failed"),
            BridgeError::InvalidWorkerState => write!(f, "Invalid worker state"),
            BridgeError::OpError(op_error) => write!(f, "{}", op_error),
        };

        Ok(())
    }
}
