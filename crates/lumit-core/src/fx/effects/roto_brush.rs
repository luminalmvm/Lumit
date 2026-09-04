//! Roto brush (docs/08 §3.88): the handle for a propagated matte.
//!
//! **In plain terms.** Scribble through the subject on one clear frame and
//! through the background beside it, and this effect cuts the subject out of
//! that frame. Press **Propagate** and a background job carries the cut-out
//! outward through the shot, watching how the picture moved and re-deciding the
//! edge on every frame from that frame's own colours. Where it drifts wrong,
//! scribble on that frame and propagate again: the correction carries forward,
//! and the frames before it are kept rather than solved a second time.
//!
//! **Why an effect and not a mode.** The matte belongs to the layer, in the
//! stack, at a position the user chooses — everything below it in the stack is
//! cut, everything above sees the cut picture. That is what an effect is. And
//! the strokes belong with the controls that decide what they mean, not in a
//! window that owns the application while it thinks.
//!
//! **What is not here.** The strokes. They are not parameters: a parameter is a
//! number the timeline animates and the frame key hashes whole, and a stroke
//! table hashed whole would rename every cached frame in the shot each time the
//! user corrected one of them. They live on the effect instance
//! ([`crate::roto::RotoBlock`]) with their own op and their own hash. The span
//! and the progress are not here either, for the Camera track's reason: "frame
//! 214 of 900" is live job state, and a parameter is something the file saves.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// What the matte does to the layer's alpha, in index order.
pub const MODE_OPTIONS: &[&str] = &["Matte", "Matte inverted"];

/// What the effect draws, in index order.
///
/// **Result** is the cut picture; **Matte** is the matte itself as a grey
/// picture, which is how a matte is judged; **Boundary** keeps the picture and
/// asks the viewer to draw the edge over it, which is the overlay's business
/// rather than the stack's — at the stack seam it renders as Result.
pub const VIEW_OPTIONS: &[&str] = &["Result", "Matte", "Boundary"];

/// The resolution the flow is measured at, in index order. **Half by default**:
/// the note's own default, and the setting the per-frame budget was measured
/// against (docs/impl/roto.md §7).
pub const FLOW_RESOLUTION_OPTIONS: &[&str] = &["Native", "Half", "Quarter"];

/// The divisor one [`FLOW_RESOLUTION_OPTIONS`] index means to the flow engine.
/// An index this build does not know reads as Half — the tasteful default,
/// never a fault (14-ENGINEERING-RULES §4).
#[must_use]
pub fn flow_divisor(index: u32) -> u32 {
    match index {
        0 => 1,
        2 => 4,
        _ => 2,
    }
}

/// The **View** row's resolved id, so the render seam can read which picture to
/// draw without a string lookup per op.
pub const VIEW_ID: crate::fx::params::ParamId = crate::fx::params::ParamId::new("view");

/// The **Matte mode** row's resolved id.
pub const MODE_ID: crate::fx::params::ParamId = crate::fx::params::ParamId::new("mode");

/// The Roto brush's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "roto_brush",
    label = "Roto brush",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // §2.2: the effect changes COVERAGE and must leave colour alone, so it runs
    // on straight values — Set matte's reasoning exactly, for the same
    // arithmetic (a premultiplied colour multiplied by a new alpha would be
    // scaled twice).
    premultiplied = false,
    // The effect that IS a matte carries no Matte row (the owner's rule
    // for mattes, as Set matte and Matte key already read it). What gates this
    // effect is the propagated matte, and a second picture saying "how much of
    // me happens here" would be a coverage laid over a coverage.
    matte = false,
)]
pub struct RotoBrush {
    /// Carry the base frame's matte outward through the shot. A button, not a
    /// value.
    #[action(label = "Propagate")]
    pub propagate: (),
    /// Stop a running propagation. **Cancel finalises rather than discards**:
    /// the frames already solved are correct and correctly named, so they are
    /// kept and the span says how far it got.
    #[action(label = "Cancel")]
    pub cancel: (),
    /// Whether the matte keeps the subject or drops it.
    #[choice(options = *MODE_OPTIONS, default = 0, label = "Matte mode")]
    pub mode: u32,
    /// What the effect draws — see [`VIEW_OPTIONS`]. Not part of the matte's own
    /// name: switching the view must not throw away a propagation.
    #[choice(options = *VIEW_OPTIONS, default = 0, label = "View")]
    pub view: u32,
    /// The guided filter's window radius, and the half-width of the band its
    /// answer is allowed into (docs/impl/roto.md §4). Wider recovers more of a
    /// soft edge — hair, motion blur, smoke — and costs more per frame.
    ///
    /// **`Raw` and not `Px`, deliberately.** px@comp means "converted to the
    /// raster in play by the resolve step" (docs/08 §2.3), and this number is
    /// never read at a comp raster: the matte is solved on the propagation
    /// thread at the **source's own** raster, where a preview tier does
    /// not reach and the resolve step never runs. Declaring it px@comp would
    /// hand the panel a rider that scaled with a quality switch the propagation
    /// cannot see.
    #[slider(
        label = "Refine radius",
        min = 0.0,
        max = 64.0,
        default = 8.0,
        hard_min = 0.0,
        hard_max = 256.0,
        unit = Raw
    )]
    pub refine_radius: f32,
    /// The resolution the propagation's optical flow is measured at.
    #[choice(options = *FLOW_RESOLUTION_OPTIONS, default = 1, label = "Flow resolution")]
    pub flow_resolution: u32,
    /// The flow's regularisation, 0–100: high means fewer tears and a gloopier
    /// field, low means crisper motion boundaries.
    #[slider(
        label = "Flow smoothness",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub flow_smoothness: f32,
}

/// The Roto brush's behaviour: it multiplies the layer's alpha by the matte the
/// propagation filed for this source frame, where it stands in the stack.
///
/// An **image operation**, unlike the two tracking handles beside it in Utility:
/// those hold a job whose answer another layer reads, and this one holds a job
/// whose answer is a picture applied right here. Outside the propagated span it
/// is a passthrough with an honest span reading — never a held neighbouring
/// matte, which would be a wrong answer wearing a right one's face.
pub struct RotoBrushDef;

impl EffectDef for RotoBrushDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RotoBrush as EffectMetadata>::SCHEMA
    }
}
