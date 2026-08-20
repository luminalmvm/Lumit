//! Fast motion blur (docs/08 §3.2): the optical-flow, footage-internal smear.
//! "Fast" because it is a single-pass per-pixel streak, distinct from the
//! whole-scene, re-rendering Motion blur (accumulation, §3.26).
//!
//! **In plain terms.** The motion itself is not a control: the decode worker
//! computes a dense per-pixel `(u, v)` field with a confidence channel from the
//! current and next source frames, and hands the whole field to the GPU pass
//! beside the resolved op (K-387). So what this effect resolves to is only how
//! *much* of that motion to draw — the shutter fraction, the tap count along the
//! streak, the diagnostic view, and Mix. With no field (a plain layer, or a
//! decode that dropped the neighbour) the pass is a passthrough, never a fault.
//!
//! There is no CPU reference through the single-buffer dispatcher, which does
//! not carry a flow field, so `apply_cpu` keeps its identity default — exactly
//! as the old `Resolved::MotionBlur` arm of `cpu::apply` was a passthrough. The
//! §1.6 oracle is [`crate::fx::cpu::motion_blur`], exercised directly from the
//! lumit-gpu test, which can upload one.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema, MbQuality, MbView};
use lumit_fx_macros::Effect;

/// Fast motion blur's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "motion_blur",
    label = "Fast motion blur",
    version = 1,
    category = Temporal,
    cost = Heavy,
    roi = FullFrame,
    // Current frame + one ahead: the flow engine brackets the motion between
    // them. The +1 neighbour is fetched by the same decode planner Echo's
    // negative offsets use.
    temporal = &[0, 1],
)]
pub struct MotionBlur {
    /// Degrees (§3.2: 0–720, default 180): the fraction of the frame interval
    /// the shutter is open, so the streak length is shutter ÷ 360 of the
    /// inter-frame motion. 180° = half the motion, the film-standard look.
    #[slider(
        min = 0.0,
        max = 720.0,
        default = 180.0,
        hard_min = 0.0,
        hard_max = 720.0
    )]
    pub shutter_angle: f32,

    /// The *cap* on taps along the streak (§3.2). The spec's integer, carried as
    /// a Float row (Echo's Echoes does the same); [`MotionBlur::packed`] rounds
    /// and clamps. The kernel spends fewer than this on a short streak (K-390's
    /// adaptive taps), so this is the ceiling on quality and cost, not the count.
    #[slider(min = 8.0, max = 32.0, default = 16.0, hard_min = 2.0, hard_max = 64.0)]
    pub samples: f32,

    /// The reconstruction tier (K-390). Normal draws straight streaks; High
    /// bends each streak along the motion field and halves the sample spacing.
    /// **The only method choice there is** — one method adapts internally.
    #[choice(
        options = ["Normal", "High"],
        default = 0
    )]
    pub quality: u32,

    /// Diagnostic outputs (FX-19): the blurred picture, the flow vectors
    /// colour-coded (red +x, green +y), the confidence as greyscale (white =
    /// trusted, black = suspect — where the streak is steered by its
    /// neighbourhood), or that neighbourhood's own dominant motion. Rendered by
    /// default.
    #[choice(
        options = ["Rendered", "Motion vectors", "Confidence", "Dominant motion"],
        default = 0
    )]
    pub view: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl MotionBlur {
    /// What the kernel wants, converted exactly as the old resolve arm converted
    /// it: the shutter as a fraction of the frame interval (floored at zero, so a
    /// negative angle cannot reverse the streak), the tap cap rounded and
    /// clamped to the 2..=64 range both the kernel and the CPU oracle bound
    /// themselves by, the view and the quality tier as their wire codes (an
    /// unknown stored index falls back to Rendered and to Normal rather than to
    /// a diagnostic or to the expensive tier), and Mix as a plain 0..1 fraction.
    /// Both render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> (f32, i32, f32, MbView, MbQuality) {
        (
            (self.shutter_angle / 360.0).max(0.0),
            (self.samples.round() as i32).clamp(2, 64),
            (self.mix / 100.0).clamp(0.0, 1.0),
            match self.view {
                1 => MbView::MotionVectors,
                2 => MbView::Confidence,
                3 => MbView::TileMax,
                _ => MbView::Rendered,
            },
            match self.quality {
                1 => MbQuality::High,
                _ => MbQuality::Normal,
            },
        )
    }
}

/// Fast motion blur's behaviour: no CPU reference through the single-buffer
/// dispatcher (the flow field is a texture), so `apply_cpu` keeps its identity
/// default — the passthrough the old `Resolved::MotionBlur` arm was.
pub struct MotionBlurDef;

impl EffectDef for MotionBlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MotionBlur as EffectMetadata>::SCHEMA
    }
}
