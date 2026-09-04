//! Posterize time (docs/08 §3.25, docs/impl/temporal-rerender.md): holding the
//! input on a coarser frame-rate grid, for the choppy stop-motion look.
//!
//! **In plain terms.** This is the first effect in the catalogue that draws
//! nothing. It does not change the colours of any pixel — it changes *what time*
//! the layers it covers render at, which is a decision taken a level above the
//! effect stack, in the frame walk that both the preview and the export share
//! ([`crate::fx::stack_posterize`] reads the instance directly). So it declares
//! its controls, and declares that it has no image operation: the render path
//! skips it, and the registry-agreement test excuses it from needing a GPU pass.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// Posterize time's controls.
///
/// The Scope choice was removed (owner, 2026-07-19): the reach is implied
/// by the carrier now — a plain layer holds its own source and effect stack, an
/// adjustment layer holds everything below (that *is* its effect input). A stored
/// `scope` on an old instance is simply unread.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "posterize_time",
    label = "Posterize time",
    version = 1,
    category = Temporal,
    // One render at the held time — often the SAME held time across many frames.
    cost = Cheap,
    // It re-renders the composite below at a held time; no per-pixel ROI applies,
    // so full-frame is the safe static declaration.
    roi = FullFrame,
)]
pub struct PosterizeTime {
    /// The posterised grid in fps: the animation updates only this many times a
    /// second. Default 12 (the classic on-twos look).
    #[slider(
        label = "Frame rate",
        min = 1.0,
        max = 60.0,
        default = 12.0,
        hard_min = 0.01,
        unit = Raw
    )]
    pub rate: f32,

    /// Comp seconds: shifts where the steps land, so the hold can be aligned to a
    /// beat. 0 snaps to the comp's own zero.
    #[slider(min = -1.0, max = 1.0, default = 0.0, unit = Seconds)]
    pub phase: f32,
}

/// Posterize time's behaviour: none, by design.
pub struct PosterizeTimeDef;

impl EffectDef for PosterizeTimeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PosterizeTime as EffectMetadata>::SCHEMA
    }

    /// Orchestration only — it holds time, it does not draw. The resolve step
    /// pushes no op for it at all, which is exactly what the old `resolve_one`
    /// returning `None` meant.
    fn is_image_op(&self) -> bool {
        false
    }
}
