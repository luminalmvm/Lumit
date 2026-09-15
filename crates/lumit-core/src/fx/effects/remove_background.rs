//! Remove background (docs/08 §3.105): the handle for a matte analysis.
//!
//! **In plain terms.** Drop this on a shot of a person and press Analyse. A
//! model reads every frame and works out how much of each pixel is them rather
//! than what is behind them, and the answer is kept beside the shot so it is
//! worked out once. Composite view then cuts the background away; Matte view
//! shows the coverage itself, which is how a matte is judged.
//!
//! **Which model.** Robust Video Matting reads the shot in order and carries
//! what it made of one frame into the next, so an edge holds steady while
//! somebody moves; it is the one to reach for on footage of a person. BiRefNet
//! reads each frame on its own, knows nothing of the frame before it, and is
//! slower by a long way, but it is not trained on people alone.
//!
//! **What is not here.** The model itself, which is an addon the user installs
//! from Settings and Lumit never ships (docs/impl/addons.md). Without it the
//! effect wears a calm badge saying which pack is missing, renders identity,
//! and keeps every value it holds.
//!
//! **What is not here either.** The progress, for the reason the Camera track
//! and Depth both give: "Frame 214 of 900" is live job state, and a string row
//! pretending to be a parameter would put a progress bar in the save file.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen};
use lumit_fx_macros::Effect;

/// The matte models this build has code for, in index order. The architecture
/// is what the engine binds to, so a second entry means a second tensor
/// contract rather than a second download.
pub const MODEL_OPTIONS: &[&str] = &["Robust Video Matting", "BiRefNet"];

/// The catalogue id each [`MODEL_OPTIONS`] entry means, so the badge can name
/// the pack that is missing.
///
/// The table lives here rather than in the crate that opens a model, for the
/// Camera track's reason: the crate that owns the control cannot depend on the
/// crate that owns the runtime, and the job reads it the other way round, which
/// is the only direction there is.
pub const MODEL_PACKS: [&str; 2] = ["rvm", "birefnet-lite"];

/// The pack a stored model index names. An index this build does not know
/// reads as the first, which is the tasteful default rather than a fault
/// (14-ENGINEERING-RULES §4).
#[must_use]
pub fn model_pack(index: u32) -> &'static str {
    MODEL_PACKS
        .get(index as usize)
        .copied()
        .unwrap_or(MODEL_PACKS[0])
}

/// What the effect draws, in index order. **Composite** is the layer with its
/// background cut away; **Matte** is the coverage itself as a grey picture.
pub const VIEW_OPTIONS: &[&str] = &["Composite", "Matte"];

/// How much of the frame Robust Video Matting works at, in index order.
/// **Portrait** is head and shoulders, which is most of the frame; **Full
/// body** is a whole person at a distance, where the finer setting loses the
/// limbs.
pub const DETAIL_OPTIONS: &[&str] = &["Portrait", "Full body"];

/// The **View** row's resolved id, so the render seam reads which picture to
/// draw without a string lookup per op.
pub const VIEW_ID: crate::fx::params::ParamId = crate::fx::params::ParamId::new("view");

/// The **Invert** row's resolved id.
pub const INVERT_ID: crate::fx::params::ParamId = crate::fx::params::ParamId::new("invert");

/// Detail is Robust Video Matting's own control and means nothing to BiRefNet,
/// which has a square of its own, so it greys out while the other model is
/// chosen rather than sitting there doing nothing.
pub const REMOVE_BACKGROUND_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "detail",
    on: "model",
    cond: EnabledCond::ChoiceIs(0),
}];

/// The Remove background effect's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "remove_background",
    label = "Remove background",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // The coverage is multiplied into the alpha and the colour is left alone,
    // which is the straight-alpha arithmetic `set_matte` already fuses into one
    // pass, exactly as the Roto brush does it.
    premultiplied = false,
    // The effect that IS a coverage carries no Matte row. What it applies is
    // the matte its analysis made, and a second picture saying how much of that
    // happens here would be a coverage laid over a coverage.
    matte = false,
    enabled_when = REMOVE_BACKGROUND_ENABLED_WHEN,
)]
pub struct RemoveBackground {
    /// Read the shot and file its matte. A button, not a value.
    #[action(label = "Analyse")]
    pub analyse: (),
    /// Stop a running analysis. **Robust Video Matting starts again**: it
    /// carries its state from one frame to the next and the record does not
    /// keep it, so the frames already read are kept to look at and the next
    /// Analyse reads the shot from the top (docs/impl/addons.md §13).
    #[action(label = "Cancel")]
    pub cancel: (),
    /// Which model reads the frames - see [`MODEL_OPTIONS`]. Part of the
    /// analysis's own name, so changing it asks for a new one.
    #[choice(options = *MODEL_OPTIONS, default = 0, label = "Model")]
    pub model: u32,
    /// What the effect draws - see [`VIEW_OPTIONS`]. Not part of the analysis's
    /// name: looking at the matte must not throw one away.
    #[choice(options = *VIEW_OPTIONS, default = 0, label = "View")]
    pub view: u32,
    /// Keep the background and cut the subject away instead. A reading of the
    /// matte rather than a different matte, so it renames nothing either.
    #[toggle(default = false, label = "Invert")]
    pub invert: bool,
    /// How much of the frame Robust Video Matting works at - see
    /// [`DETAIL_OPTIONS`]. It changes what the model produces, so it is part of
    /// the analysis's name.
    #[choice(options = *DETAIL_OPTIONS, default = 0, label = "Detail")]
    pub detail: u32,
}

/// The Remove background effect's behaviour: draw the matte the analysis filed
/// for this source frame, where it stands in the stack.
///
/// An **image operation**, like Depth and the Roto brush: what it holds is a
/// picture applied right here, which is also what lets another layer read it
/// through the ordinary matte and layer-input carriages. Outside the analysed
/// span it is a passthrough with an honest span reading, never a held
/// neighbouring matte.
pub struct RemoveBackgroundDef;

impl EffectDef for RemoveBackgroundDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RemoveBackground as EffectMetadata>::SCHEMA
    }
}
