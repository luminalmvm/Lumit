//! Iris wipe (docs/08 §3.71): a polygon or a star opened out of the middle —
//! AE's Iris Wipe.
//!
//! **In plain terms.** A hole in the shape of a polygon opens in the picture and
//! grows. Iris points says how many corners it has, Outer radius how big it is,
//! Rotation which way it is turned, and Feather how soft its edge is; switch
//! Use inner radius on and every other corner is pulled in, which turns the
//! polygon into a star.
//!
//! There is no Completion here, and AE has none either: **the radius is the
//! transition**, so the shape is animated by growing it.
//!
//! The trick that makes it cheap is worth knowing. A polygon and a star are both
//! the same wedge repeated round a circle, so a pixel's angle is folded into one
//! wedge and mirrored about its middle — and what is left is a single straight
//! edge, whose distance is one dot product. No outline is ever drawn.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, Params};
use lumit_fx_macros::Effect;

/// An inner radius means nothing until the star is switched on.
pub const IRIS_WIPE_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "inner_radius",
    on: "use_inner_radius",
    cond: EnabledCond::BoolIs(true),
}];

/// Iris wipe's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "iris_wipe",
    label = "Iris wipe",
    version = 1,
    category = Transition,
    // One `atan2` a pixel — docs/08 §3.47's admission again, recorded by K-399:
    // the angle IS a function of the pixel and cannot be lifted host-side.
    cost = Cheap,
    roi = Exact,
    premultiplied = true,
    enabled_when = IRIS_WIPE_ENABLED_WHEN,
    // K-429: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the iris radius per pixel: the polygon opens wide where the \
         matte is bright and shuts to nothing where it is black, this \
         effect having no Completion to scale",
    ),
)]
pub struct IrisWipe {
    /// Where the iris opens, px@comp (K-260: point parameters are PIXELS). The
    /// schema default is nominal 1080p centre;
    /// [`instantiate_for_raster`](crate::fx::instantiate_for_raster) centres a
    /// fresh instance on the actual comp.
    #[slider(label = "Iris centre x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Iris centre y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// How many corners the polygon has — AE's range exactly, and hard at both
    /// ends because six is where a polygon stops reading as a circle from the
    /// inside and thirty-two is where it starts reading as one from the outside.
    #[counter(
        label = "Iris points",
        min = 6,
        max = 32,
        default = 6,
        hard_min = 6,
        hard_max = 32
    )]
    pub points: i32,

    /// How far the corners reach, px@comp (§2.3), scaled with the preview
    /// raster so the hole survives a reframe. 0 is the exact identity.
    #[slider(
        label = "Outer radius",
        min = 0.0,
        max = 2000.0,
        default = 330.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub outer_radius: f32,

    /// On, every other corner is pulled in to [`inner_radius`](Self::inner_radius)
    /// and the polygon becomes a star. Off, as AE's is.
    #[toggle(label = "Use inner radius", default = false)]
    pub use_inner_radius: bool,

    /// Where the pulled-in corners sit, px@comp; see
    /// [`outer_radius`](Self::outer_radius).
    #[slider(
        label = "Inner radius",
        min = 0.0,
        max = 2000.0,
        default = 165.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub inner_radius: f32,

    /// Which way the shape is turned, degrees, from straight up and clockwise
    /// (§3.46's convention, and AE's).
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// How soft the iris edge is, px@comp — a true perpendicular width, because
    /// the kernel's distance is a perpendicular one (§3.71's first note).
    #[slider(min = 0.0, max = 500.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub feather: f32,

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

impl IrisWipe {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The whole of the shape is decided here: one sector's two vertices and the
    /// outward unit normal of the edge between them. Plain polygon and star
    /// differ only in where the second vertex goes, so the kernel has no branch
    /// and the toggle costs nothing per pixel (docs/08 §3.71).
    #[must_use]
    pub fn packed(self) -> cpu::IrisWipeParams {
        use std::f32::consts::TAU;
        let n = self.points.clamp(6, 32) as f32;
        let period = TAU / n;
        let outer = self.outer_radius.max(0.0);
        let (angle_b, radius_b) = if self.use_inner_radius {
            (period * 0.5, self.inner_radius.max(0.0))
        } else {
            (period, outer)
        };
        let a = [outer, 0.0];
        let b = [radius_b * angle_b.cos(), radius_b * angle_b.sin()];
        // The edge's outward normal: rotate B − A by a quarter turn. It points
        // away from the centre because the centre is inside the polygon, which
        // `(A − 0)·n = outer·radius_b·sin(angle_b) ≥ 0` says.
        let normal = [b[1] - a[1], a[0] - b[0]];
        let len = (normal[0] * normal[0] + normal[1] * normal[1]).sqrt();
        // Floored so a degenerate shape (an inner radius of zero, whose "edge"
        // runs through the centre) divides by something rather than by nothing.
        let inv_len = 1.0 / len.max(1e-6);
        cpu::IrisWipeParams {
            centre: [self.centre_x, self.centre_y],
            vertex: a,
            normal: [normal[0] * inv_len, normal[1] * inv_len],
            period,
            rotation: self.rotation.to_radians(),
            // Floored so the hard-edged case is a step rather than a divide by
            // zero (docs/14 §4); neither path divides per pixel.
            band: self.feather.max(0.0).max(1e-3),
            // Outer radius 0 is the identity by short-circuit: with no polygon
            // there is no edge to be a distance from (§3.71's fifth note).
            active: outer > 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Iris wipe's behaviour.
pub struct IrisWipeDef;

impl EffectDef for IrisWipeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<IrisWipe as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::iris_wipe(rgba, w, h, &IrisWipe::read(p).packed());
    }
}
