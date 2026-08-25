//! Slider control (docs/08 §3.80): one number, held for something else to read.
//!
//! **In plain terms.** This effect draws nothing. Its whole job is to be a
//! number you can keyframe, sitting on a layer where an expression on some other
//! property can read it — one dial that drives six things at once. It is After
//! Effects' Slider Control, which half the rigs in the world are wired through,
//! and it is why the Controls category exists (K-414).
//!
//! **Why the row is a plain Float and not the new Slider kind.** A Slider
//! control has no range: whatever it is about to drive decides what its numbers
//! mean, and a rig that wants 0 to 3000 is as ordinary as one that wants 0 to 1.
//! So the row declares the soft 0..100 travel After Effects gives it and no hard
//! bound at all — typing past the end is the point. The
//! [`Slider`](crate::fx::ParamKind::Slider) *kind* is the opposite case, a
//! parameter whose whole meaning lives inside a closed range.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Slider control's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "slider_control",
    label = "Slider control",
    version = 1,
    category = Controls,
    cost = Trivial,
    roi = Exact,
    // No picture, so no matte: a strength dissolve on an effect that changes
    // nothing would be a row that could never do anything (K-395's `None`).
    matte = false,
)]
pub struct SliderControl {
    /// The number. Unbounded on purpose — the soft 0..100 is only where the
    /// thumb starts.
    #[slider(min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub slider: f32,
}

/// The Slider control's behaviour: none, by design.
pub struct SliderControlDef;

impl EffectDef for SliderControlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SliderControl as EffectMetadata>::SCHEMA
    }

    /// It holds a value; it does not draw. The resolve step pushes no op for
    /// it, exactly as it pushes none for Posterize time — a different reason
    /// for the same honest answer.
    fn is_image_op(&self) -> bool {
        false
    }
}
