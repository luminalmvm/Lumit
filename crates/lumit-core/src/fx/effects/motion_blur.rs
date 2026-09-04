//! Fast motion blur (docs/08 §3.2): the optical-flow, footage-internal smear.
//! "Fast" because it is a single-pass per-pixel streak, distinct from the
//! whole-scene, re-rendering Motion blur (accumulation, §3.26).
//!
//! **In plain terms.** The motion itself is not a control: the decode worker
//! computes a dense per-pixel `(u, v)` field with a confidence channel from the
//! current and next source frames, and hands the whole field to the GPU pass
//! beside the resolved op. So what this effect resolves to is only how
//! *much* of that motion to draw — the shutter fraction, the tap count along the
//! streak, the diagnostic view, and Mix. With no field (a plain layer, or a
//! decode that dropped the neighbour) the pass is a passthrough, never a fault.
//!
//! There is no CPU reference through the single-buffer dispatcher, which does
//! not carry a flow field, so `apply_cpu` keeps its identity default — exactly
//! as the old `Resolved::MotionBlur` arm of `cpu::apply` was a passthrough. The
//! §1.6 oracle is [`crate::fx::cpu::motion_blur`], exercised directly from the
//! lumit-gpu test, which can upload one.

use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, MbQuality, MbView,
};
use lumit_fx_macros::Effect;

/// Vector scale means nothing until a Motion vectors layer is picked:
/// with none, the field is the measured flow, which is already in pixels.
pub const MOTION_BLUR_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "vector_scale",
    on: "motion_vectors",
    cond: EnabledCond::LayerSet,
}];

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
    enabled_when = MOTION_BLUR_ENABLED_WHEN,
    // The matte scales the amount, inside the kernel (the owner's rule for
    // mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Shutter angle per pixel: the streak is genuinely shorter where \
         the matte is dark, gathering from nearer along the motion, rather \
         than a long streak faded back over a sharp picture",
    ),
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
        hard_max = 720.0,
        unit = Degrees
    )]
    pub shutter_angle: f32,

    /// The *cap* on taps along the streak (§3.2). The spec's integer, carried as
    /// a Float row (Echo's Echoes does the same); [`MotionBlur::packed`] rounds
    /// and clamps. The kernel spends fewer than this on a short streak (the
    /// adaptive taps), so this is the ceiling on quality and cost, not the count.
    #[slider(min = 8.0, max = 32.0, default = 16.0, hard_min = 2.0, hard_max = 64.0, unit = Raw)]
    pub samples: f32,

    /// The reconstruction tier. Normal draws straight streaks; High bends each
    /// streak along the motion field and halves the sample spacing.
    /// **The only method choice there is** — one method adapts internally.
    #[choice(
        options = ["Normal", "High"],
        default = 0
    )]
    pub quality: u32,

    /// **A motion field somebody else already knew** (docs/08 §3.2).
    /// Point this at a layer whose red and green channels carry the per-pixel
    /// motion — a game engine's velocity pass, a 3D renderer's vector pass, a
    /// plugin's output — and the streak follows *that* instead of the flow the
    /// decode measured. The encoding is the one every such pass uses: **red is
    /// sideways, green is up-and-down, and mid-grey (0.5) is standing still**,
    /// so `(r − ½)·Vector scale` is the horizontal motion in pixels and
    /// `(g − ½)·Vector scale` the vertical. Blue and alpha are not read.
    /// Confidence comes out at 1 everywhere, because a supplied vector is not
    /// a measurement that can have failed to match.
    ///
    /// Unset is the default and the labelled no-op: the measured flow is used,
    /// exactly as before this row existed. It is the ordinary auxiliary-layer
    /// input, which is why a layer can be given here *and* a Matte
    /// above.
    ///
    /// **Always `false` here, by design**, as every Layer row is: the binding
    /// is decided by the caller, so the picture arrives at the GPU pass as its
    /// aux slot rather than as a value.
    #[layer(label = "Motion vectors", self_default = false)]
    pub motion_vectors: bool,

    /// px@comp: how far a full channel of the Motion vectors layer means —
    /// what `r = 1` (or `r = 0`) stands for in pixels of movement. Different
    /// engines normalise their vector passes differently, so this is the dial
    /// that makes one agree with the frame it came from. Declared `Px`, so it
    /// scales with the preview raster like every other distance (§2.3).
    /// Greyed until a layer is picked, since the measured flow is already in
    /// pixels.
    #[slider(
        min = 0.0,
        max = 200.0,
        default = 32.0,
        hard_min = 0.0,
        hard_max = 4000.0,
        unit = Px
    )]
    pub vector_scale: f32,

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
        hard_max = 100.0,
        unit = Percent
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
    /// a diagnostic or to the expensive tier), Mix as a plain 0..1 fraction,
    /// and Vector scale floored at zero (a negative scale would read a supplied
    /// field backwards, which is a thing to say with the field, not here).
    /// Both render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> (f32, i32, f32, MbView, MbQuality, f32) {
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
            self.vector_scale.max(0.0),
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
