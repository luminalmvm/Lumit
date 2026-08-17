//! Transform (docs/08 §3.5): an anchor, a position, a scale, a rotation and an
//! opacity applied inside the effect stack, rather than to the layer as a whole.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Transform's controls.
///
/// The four point components are px@comp (K-260: point parameters are pixels,
/// never per cent), declared `Px` so the resolve step scales them by the §2.3
/// preview factor and
/// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::rescale_spatial)
/// moves them again if the stack is reused at another size — exactly what the old
/// arm and `rescale_px` did between them. Scale is per cent and rotation is
/// degrees, and neither follows the raster, which is why they stay `Raw`.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "transform",
    label = "Transform",
    version = 1,
    category = Utility,
    cost = Trivial,
    // §3.5: exact under pure translation, full-frame otherwise — the static
    // declaration carries the general case.
    roi = FullFrame,
)]
pub struct Transform {
    /// px@comp, exactly like the layer transform's Anchor; unbounded (K-090).
    #[slider(min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub anchor_x: f32,

    /// See [`anchor_x`](Self::anchor_x).
    #[slider(min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub anchor_y: f32,

    /// px@comp; the anchor point lands here. Defaults equal the anchor's, so a
    /// fresh instance is the identity.
    #[slider(min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub position_x: f32,

    /// See [`position_x`](Self::position_x).
    #[slider(min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub position_y: f32,

    /// Per cent, 100 = natural size; negative flips (like the layer transform),
    /// so both hard sides stay open.
    #[slider(label = "Scale x %", min = 0.0, max = 400.0, default = 100.0)]
    pub scale_x: f32,

    /// See [`scale_x`](Self::scale_x).
    #[slider(label = "Scale y %", min = 0.0, max = 400.0, default = 100.0)]
    pub scale_y: f32,

    /// Degrees on a dial (docs/07 §6), unbounded — whip transitions spin whole
    /// turns, and a dial that stopped at 360 could not.
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// Per cent.
    #[slider(
        label = "Opacity %",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub opacity: f32,

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

impl Transform {
    /// The anchor, position, scale, rotation, opacity and mix the kernel wants
    /// (docs/impl/effect-registry.md §2.4). The two points arrive already scaled
    /// by the §2.3 preview factor; the per-cent pairs become plain fractions,
    /// exactly as the old resolve arm's `px`/`pct` helpers made them. Both render
    /// paths read this one method, so the CPU reference and the WGSL kernel
    /// cannot drift apart.
    pub fn packed(self) -> ([f32; 2], [f32; 2], [f32; 2], f32, f32, f32) {
        (
            [self.anchor_x, self.anchor_y],
            [self.position_x, self.position_y],
            [self.scale_x / 100.0, self.scale_y / 100.0],
            self.rotation,
            (self.opacity / 100.0).clamp(0.0, 1.0),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Transform's behaviour.
pub struct TransformDef;

impl EffectDef for TransformDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Transform as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (anchor, position, scale, rotation_deg, opacity, mix) = Transform::read(p).packed();
        cpu::transform(
            rgba,
            w,
            h,
            anchor,
            position,
            scale,
            rotation_deg,
            // The Transform effect has no Edges control: a transparent border,
            // its long-standing behaviour.
            0,
            opacity,
            mix,
        );
    }
}
