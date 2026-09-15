//! Depth (docs/08 §3.104): the handle for a depth analysis.
//!
//! **In plain terms.** Drop this on a shot and press Analyse. A model reads
//! every frame and guesses how far away each pixel is, and the answer is kept
//! beside the shot so it is worked out once. Then point a Depth of field, a Set
//! matte or a track matte at this layer and the depth is what it reads.
//!
//! The numbers have no unit. What a depth model knows is what is *nearer* than
//! what, not how many metres away anything is, so the plane is scaled to each
//! frame's own nearest and furthest and nearer is brighter. That is enough for
//! every consumer of it, and pretending otherwise would be pretending.
//!
//! **What is not here.** The model itself, which is an addon the user installs
//! from Settings and Lumit never ships (docs/impl/addons.md). Without it the
//! effect wears a calm badge saying which pack is missing, renders identity,
//! and keeps every value it holds.
//!
//! **What is not here either.** The progress. "Frame 214 of 900" is live job
//! state, and a string row pretending to be a parameter would put a progress
//! bar in the save file - the Camera track's reasoning, unchanged.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The depth models this build has code for, in index order. One today: the
/// architecture is what the engine binds to, and a second entry means a second
/// tensor contract rather than a second download.
pub const MODEL_OPTIONS: &[&str] = &["Depth Anything V2 Small"];

/// The catalogue id each [`MODEL_OPTIONS`] entry means, so the badge can name
/// the pack that is missing.
///
/// The table lives here rather than in the crate that opens a model, for the
/// Camera track's reason: the crate that owns the control cannot depend on the
/// crate that owns the runtime, and the job reads it the other way round, which
/// is the only direction there is.
pub const MODEL_PACKS: [&str; 1] = ["depth-anything-v2-small"];

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

/// What the effect draws, in index order. **Depth** is the plane itself as a
/// grey picture, which is how a depth pass is judged; **Source** leaves the
/// layer alone so it can be looked at while the plane rides to a consumer.
pub const VIEW_OPTIONS: &[&str] = &["Depth", "Source"];

/// The **View** row's resolved id, so the render seam reads which picture to
/// draw without a string lookup per op.
pub const VIEW_ID: crate::fx::params::ParamId = crate::fx::params::ParamId::new("view");

/// The **Invert** row's resolved id.
pub const INVERT_ID: crate::fx::params::ParamId = crate::fx::params::ParamId::new("invert");

/// The Depth effect's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "depth",
    label = "Depth",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // The plane is coverage-shaped rather than colour-shaped: it is drawn
    // straight, never multiplied into a premultiplied picture, so it runs on
    // straight values exactly as the Roto brush does.
    premultiplied = false,
    // The effect that IS a plane carries no Matte row. What it draws is a
    // reading of the picture underneath, and a second picture saying how much
    // of that reading happens here would gate a measurement.
    matte = false,
)]
pub struct Depth {
    /// Read the shot and file its depth. A button, not a value.
    #[action(label = "Analyse")]
    pub analyse: (),
    /// Stop a running analysis. **Cancel keeps what it had**: the frames
    /// already read are correct and correctly named, so they are kept and the
    /// span says how far it got.
    #[action(label = "Cancel")]
    pub cancel: (),
    /// Which model reads the frames - see [`MODEL_OPTIONS`]. Part of the
    /// analysis's own name, so changing it asks for a new one.
    #[choice(options = *MODEL_OPTIONS, default = 0, label = "Model")]
    pub model: u32,
    /// What the effect draws - see [`VIEW_OPTIONS`]. Not part of the analysis's
    /// name: looking at the depth must not throw one away.
    #[choice(options = *VIEW_OPTIONS, default = 0, label = "View")]
    pub view: u32,
    /// Draw the plane the other way up, so far is bright. A reading of the
    /// plane rather than a different plane, so it renames nothing either.
    #[toggle(default = false, label = "Invert")]
    pub invert: bool,
}

/// The Depth effect's behaviour: draw the plane the analysis filed for this
/// source frame, where it stands in the stack.
///
/// An **image operation**, like the Roto brush and unlike the two tracking
/// handles: those hold a job whose answer another layer reads, and this one
/// holds a job whose answer is a picture drawn right here - which is also what
/// lets another layer read it, through the ordinary matte and layer-input
/// carriages. Outside the analysed span it is a passthrough with an honest span
/// reading, never a held neighbouring plane.
pub struct DepthDef;

impl EffectDef for DepthDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Depth as EffectMetadata>::SCHEMA
    }
}
