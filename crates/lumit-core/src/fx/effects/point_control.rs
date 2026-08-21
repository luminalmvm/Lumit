//! Point control (docs/08 §3.84): one place in the frame, held for something
//! else to read.
//!
//! **In plain terms.** A crosshair you can drag on the picture and keyframe, and
//! which draws nothing itself. An expression reads it, so one dragged point can
//! move a flare, a mask and a light together.
//!
//! It is two parameters rather than one, because a point in Lumit is an adjacent
//! `_x`/`_y` pair the panel folds into a single row with a crosshair pick
//! (docs/08 §1.1) — a point needs no schema kind of its own, only the naming
//! convention.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Point control's one control, as its two halves.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "point_control",
    label = "Point control",
    version = 1,
    category = Controls,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct PointControl {
    /// px@comp (K-260 — point parameters are pixels, never per cent of frame).
    /// The schema default is the nominal 1080p centre; `instantiate_for_raster`
    /// centres a fresh instance on the actual comp, because a control that
    /// lands in the top-left corner of a 4K frame is a control somebody has to
    /// go and find.
    #[slider(min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub point_x: f32,

    /// px@comp; see [`point_x`](Self::point_x).
    #[slider(min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub point_y: f32,
}

/// The Point control's behaviour: none, by design.
pub struct PointControlDef;

impl EffectDef for PointControlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PointControl as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}
