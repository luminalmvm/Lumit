//! Accumulation motion blur (docs/08 §3.26, docs/impl/temporal-rerender.md): the
//! expensive, correct motion blur — the whole scene below is rendered several
//! times at in-between moments and the finished frames averaged.
//!
//! **In plain terms.** Like Posterize time, this effect draws nothing itself. It
//! changes *what time the layers below it render at*, which the frame walk shared
//! by the preview and the export decides ([`crate::fx::stack_accumulation_mb`]
//! reads the instance directly). So it declares its controls, and declares that
//! it has no image operation.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Accumulation motion blur's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "accumulation_mb",
    // The user-facing motion blur (docs/08 §3.26): the accumulation kind is the
    // correct, whole-scene one, so it takes the plain name. The optical-flow
    // effect (match_name "motion_blur") is "Fast motion blur".
    label = "Motion blur",
    version = 1,
    category = Temporal,
    // ≈ N× a full comp render.
    cost = Heavy,
    roi = FullFrame,
)]
pub struct AccumulationMb {
    /// Sub-frame renders of the scene below across the open shutter (≥ 2 to
    /// blur). The schema has no integer kind for this row's history, so it is a
    /// Float (as Echo's Echoes and flow Motion blur's Samples are); the detector
    /// rounds and clamps. Heavy — each sample is a full comp re-render — so a
    /// tasteful default of 8.
    #[slider(min = 2.0, max = 32.0, default = 8.0, hard_min = 2.0, hard_max = 64.0)]
    pub samples: f32,

    /// Degrees: the fraction of the frame interval the shutter is open is
    /// shutter ÷ 360, so the samples span that much of the motion. 180° (half a
    /// frame) is the film-standard look.
    #[slider(
        min = 0.0,
        max = 720.0,
        default = 180.0,
        hard_min = 0.0,
        hard_max = 720.0
    )]
    pub shutter_angle: f32,

    /// Degrees: where the open interval sits relative to the frame time. -90
    /// centres the samples on the frame (pairing with a 180 angle to open a
    /// quarter-frame either side).
    #[slider(
        min = -360.0,
        max = 360.0,
        default = -90.0,
        hard_min = -720.0,
        hard_max = 720.0
    )]
    pub shutter_phase: f32,

    /// Force per-layer motion blur (K-120) on every layer during the sub-frame
    /// sample renders — the shutter above stands in for the comp master and each
    /// layer's own switch, without mutating the comp. So one effect blurs every
    /// moving layer without toggling each one; each accumulation sample is itself
    /// transform-smeared, smoothing the result at lower sample counts.
    #[toggle(label = "Force on all layers", default = false)]
    pub force_all: bool,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent —
    /// here blending the averaged result against the frame-time composite.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

/// Accumulation motion blur's behaviour: none, by design.
pub struct AccumulationMbDef;

impl EffectDef for AccumulationMbDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AccumulationMb as EffectMetadata>::SCHEMA
    }

    /// Orchestration only — it re-renders the scene below, it does not draw a
    /// pass of its own. The resolve step pushes no op for it at all, which is
    /// exactly what the old `resolve_one` returning `None` meant.
    fn is_image_op(&self) -> bool {
        false
    }
}
