//! Corner pin (docs/08 §3.48): the picture pulled onto four points — AE's
//! Corner Pin, and the import workhorse of the distort family.
//!
//! **In plain terms.** Drag the frame's four corners anywhere and the picture
//! follows, stretching between them the way a photograph pinned to a wall does.
//! It is how a screen replacement is done: put the four corners on the four
//! corners of the phone in the shot, and the layer sits on the phone.
//!
//! The maths is a **homography** — a projective transform, the one kind of
//! flat-picture warp that keeps straight lines straight while letting parallel
//! ones converge, which is exactly what a camera does. The eight numbers it
//! needs are the eight the four points already are, so nothing is stored beyond
//! the points; [`CornerPin::packed`] derives the matrix, inverts it, and hands
//! both kernels the same nine numbers.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params, EDGE_OPTIONS};
use lumit_fx_macros::Effect;

/// Corner pin's controls.
///
/// Every point is px@comp (§2.3, K-260 — point parameters are pixels, never per
/// cent). The schema defaults are a nominal 1080p keystone; a fresh instance is
/// put on the actual comp by
/// [`instantiate_for_raster`](crate::fx::instantiate_for_raster), because a
/// schema constant cannot know the raster and a pin sitting in the top-left
/// quarter of a 4K comp is exactly the "drop it on and it already looks right"
/// failure §1.2 names.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "corner_pin",
    label = "Corner pin",
    version = 1,
    category = Distortion,
    // One matrix multiply, one divide and one bilinear tap a pixel.
    cost = Cheap,
    // Four points anywhere: the output pixel can come from anywhere in the input.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct CornerPin {
    /// px@comp: where the frame's top-left corner ends up.
    #[slider(label = "Upper left x", min = -1920.0, max = 3840.0, default = 96.0, unit = Px)]
    pub upper_left_x: f32,

    /// px@comp; see [`upper_left_x`](Self::upper_left_x).
    #[slider(label = "Upper left y", min = -1080.0, max = 2160.0, default = 54.0, unit = Px)]
    pub upper_left_y: f32,

    /// px@comp: where the frame's top-right corner ends up.
    #[slider(label = "Upper right x", min = -1920.0, max = 3840.0, default = 1824.0, unit = Px)]
    pub upper_right_x: f32,

    /// px@comp; see [`upper_right_x`](Self::upper_right_x).
    #[slider(label = "Upper right y", min = -1080.0, max = 2160.0, default = 54.0, unit = Px)]
    pub upper_right_y: f32,

    /// px@comp: where the frame's bottom-left corner ends up.
    #[slider(label = "Lower left x", min = -1920.0, max = 3840.0, default = 0.0, unit = Px)]
    pub lower_left_x: f32,

    /// px@comp; see [`lower_left_x`](Self::lower_left_x).
    #[slider(label = "Lower left y", min = -1080.0, max = 2160.0, default = 1026.0, unit = Px)]
    pub lower_left_y: f32,

    /// px@comp: where the frame's bottom-right corner ends up.
    #[slider(label = "Lower right x", min = -1920.0, max = 3840.0, default = 1920.0, unit = Px)]
    pub lower_right_x: f32,

    /// px@comp; see [`lower_right_x`](Self::lower_right_x).
    #[slider(label = "Lower right y", min = -1080.0, max = 2160.0, default = 1026.0, unit = Px)]
    pub lower_right_y: f32,

    /// What a sample that lands outside the frame reads. **Transparent by
    /// default, which is AE's only behaviour** (§3.48 decision 5); Repeat is
    /// here because a corner pin is also how a camera tilt is faked on an
    /// over-scanned plate, and there the smear is wanted rather than a hole.
    #[choice(label = "Edges", options = *EDGE_OPTIONS, default = 0)]
    pub edge: u32,

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

impl CornerPin {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The whole projective derivation happens here, once a frame: the unit
    /// square is mapped to the quad in Heckbert's closed form, the result is
    /// inverted by its adjugate (a homography is only defined up to a scale, so
    /// the determinant divides out and is never taken), and the sign is
    /// normalised so that "in front of the horizon" is `w > 0` whichever way
    /// round the quad was dragged. The kernel is then one matrix multiply, one
    /// divide and one tap.
    ///
    /// A degenerate quad — three corners in a line, or two on top of one another
    /// — leaves `active` false, which both paths render as the exact identity
    /// rather than as a division by zero (14-ENGINEERING-RULES §4).
    #[must_use]
    pub fn packed(self) -> cpu::CornerPinParams {
        let ul = [self.upper_left_x, self.upper_left_y];
        let ur = [self.upper_right_x, self.upper_right_y];
        let ll = [self.lower_left_x, self.lower_left_y];
        let lr = [self.lower_right_x, self.lower_right_y];
        // Heckbert's unit-square → quad map, with (u, v) = (0,0) at UL, (1,0) at
        // UR, (1,1) at LR and (0,1) at LL.
        let d1 = [ur[0] - lr[0], ur[1] - lr[1]];
        let d2 = [ll[0] - lr[0], ll[1] - lr[1]];
        let d3 = [ul[0] - ur[0] + lr[0] - ll[0], ul[1] - ur[1] + lr[1] - ll[1]];
        let den = d1[0] * d2[1] - d1[1] * d2[0];
        // A parallelogram has d3 = 0 and needs no projective part; a genuinely
        // degenerate quad has den = 0 with d3 ≠ 0 and has no map at all.
        let affine = d3[0].abs() < 1e-6 && d3[1].abs() < 1e-6;
        let (g, h) = if affine {
            (0.0, 0.0)
        } else if den.abs() < 1e-6 {
            return cpu::CornerPinParams {
                inv: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                active: false,
                edge: self.edge.min(2),
                mix: (self.mix / 100.0).clamp(0.0, 1.0),
            };
        } else {
            (
                (d3[0] * d2[1] - d3[1] * d2[0]) / den,
                (d1[0] * d3[1] - d1[1] * d3[0]) / den,
            )
        };
        let m = [
            [ur[0] - ul[0] + g * ur[0], ll[0] - ul[0] + h * ll[0], ul[0]],
            [ur[1] - ul[1] + g * ur[1], ll[1] - ul[1] + h * ll[1], ul[1]],
            [g, h, 1.0],
        ];
        // The adjugate: the inverse up to the determinant, which cancels in the
        // perspective divide the kernel takes anyway.
        let mut inv = [
            [
                m[1][1] * m[2][2] - m[1][2] * m[2][1],
                m[0][2] * m[2][1] - m[0][1] * m[2][2],
                m[0][1] * m[1][2] - m[0][2] * m[1][1],
            ],
            [
                m[1][2] * m[2][0] - m[1][0] * m[2][2],
                m[0][0] * m[2][2] - m[0][2] * m[2][0],
                m[0][2] * m[1][0] - m[0][0] * m[1][2],
            ],
            [
                m[1][0] * m[2][1] - m[1][1] * m[2][0],
                m[0][1] * m[2][0] - m[0][0] * m[2][1],
                m[0][0] * m[1][1] - m[0][1] * m[1][0],
            ],
        ];
        // A quad with no area maps everything onto a line: no inverse.
        let det = m[0][0] * inv[0][0] + m[0][1] * inv[1][0] + m[0][2] * inv[2][0];
        if !det.is_finite() || det.abs() < 1e-6 {
            return cpu::CornerPinParams {
                inv: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                active: false,
                edge: self.edge.min(2),
                mix: (self.mix / 100.0).clamp(0.0, 1.0),
            };
        }
        // Sign-normalised so the horizon test in the kernel is a plain `w > 0`
        // on both paths, whichever way round the four points were dragged.
        if inv[2][2] < 0.0 {
            for row in &mut inv {
                for v in row.iter_mut() {
                    *v = -*v;
                }
            }
        }
        cpu::CornerPinParams {
            inv,
            active: true,
            edge: self.edge.min(2),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Corner pin's behaviour.
pub struct CornerPinDef;

impl EffectDef for CornerPinDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CornerPin as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::corner_pin(rgba, w, h, &CornerPin::read(p).packed());
    }
}
