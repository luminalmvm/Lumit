//! Transform (docs/08 §3.5): an anchor, a position, a scale, a skew, a rotation
//! and an opacity applied inside the effect stack, rather than to the layer as a
//! whole.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Transform's controls.
///
/// The four point components are px@comp (point parameters are pixels, never
/// per cent), declared `Px` so the resolve step scales them by the §2.3
/// preview factor and
/// [`ResolvedStack::rescale_spatial`](crate::fx::params::ResolvedStack::rescale_spatial)
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
    /// px@comp, exactly like the layer transform's Anchor; unbounded.
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
    #[slider(label = "Scale x %", min = 0.0, max = 400.0, default = 100.0, unit = Percent)]
    pub scale_x: f32,

    /// See [`scale_x`](Self::scale_x).
    #[slider(label = "Scale y %", min = 0.0, max = 400.0, default = 100.0, unit = Percent)]
    pub scale_y: f32,

    /// After Effects' unitless Skew amount, read as the lean angle in
    /// degrees: 45 shears the picture by one unit of x per unit of y, 0 is no
    /// shear at all and is byte-identical to a stack that has never heard of
    /// the control. Hard-limited short of ±90, where the lean would be
    /// infinite.
    #[slider(
        min = -70.0,
        max = 70.0,
        default = 0.0,
        hard_min = -89.0,
        hard_max = 89.0,
        unit = Raw
    )]
    pub skew: f32,

    /// The axis the lean is measured from, degrees on a dial like Rotation and
    /// unbounded for the same reason. At 0 the shear is horizontal, positive
    /// Skew leaning the top to the right; the axis turns that lean clockwise on
    /// screen.
    #[dial(default = 0.0, step = 15.0)]
    pub skew_axis: f32,

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
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

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

/// What [`Transform::packed`] hands the kernel, in order: anchor, position,
/// scale, rotation in degrees, skew as `[amount, axis_deg]`, opacity and mix.
pub type TransformPacked = ([f32; 2], [f32; 2], [f32; 2], f32, [f32; 2], f32, f32);

impl Transform {
    /// The anchor, position, scale, rotation, skew, opacity and mix the kernel
    /// wants (docs/impl/effect-registry.md §2.4). The two points arrive already
    /// scaled by the §2.3 preview factor; the per-cent pairs become plain
    /// fractions, exactly as the old resolve arm's `px`/`pct` helpers made them.
    /// Both render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> TransformPacked {
        (
            [self.anchor_x, self.anchor_y],
            [self.position_x, self.position_y],
            [self.scale_x / 100.0, self.scale_y / 100.0],
            self.rotation,
            [self.skew, self.skew_axis],
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
        let (anchor, position, scale, rotation_deg, skew, opacity, mix) =
            Transform::read(p).packed();
        cpu::transform(
            rgba,
            w,
            h,
            anchor,
            position,
            scale,
            rotation_deg,
            skew,
            // The Transform effect has no Edges control: a transparent border,
            // its long-standing behaviour.
            0,
            opacity,
            mix,
        );
    }

    /// The picture's corners carried through the transform. Under a Mix the
    /// untouched picture shows too, so it keeps where it was as well.
    fn bounds(&self, p: Params<'_>, rect: [f32; 4]) -> [f32; 4] {
        let (anchor, position, scale, rotation_deg, skew, opacity, mix) =
            Transform::read(p).packed();
        // The kernel's map runs from where a pixel lands back to where it was
        // read, so the corners go through its inverse.
        let Some(([a, b, c, d], [ox, oy])) =
            crate::fx::maths::transform_inverse(anchor, position, scale, rotation_deg, skew)
        else {
            return rect;
        };
        let det = a * d - b * c;
        if det == 0.0 || opacity <= 0.0 {
            return rect;
        }
        // The bilinear tap reads half a pixel past the picture's edge, so that
        // much more goes through, and a pixel shows if its centre lands inside.
        let [x0, y0, x1, y1] = [rect[0] - 0.5, rect[1] - 0.5, rect[2] + 0.5, rect[3] + 0.5];
        let mut out = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let (u, v) = (x - ox, y - oy);
            let (px, py) = ((d * u - b * v) / det, (a * v - c * u) / det);
            out = [
                out[0].min(px),
                out[1].min(py),
                out[2].max(px),
                out[3].max(py),
            ];
        }
        let out = [out[0] + 0.5, out[1] + 0.5, out[2] - 0.5, out[3] - 0.5];
        if mix < 1.0 {
            [
                out[0].min(rect[0]),
                out[1].min(rect[1]),
                out[2].max(rect[2]),
                out[3].max(rect[3]),
            ]
        } else {
            out
        }
    }
}
