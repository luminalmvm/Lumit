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

use crate::fx::{
    builtins::EDGE_OPTIONS, cpu, params::Params, registry::EffectDef, registry::EffectMetadata,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Corner pin's controls.
///
/// Every point is px@comp (§2.3 — point parameters are pixels, never per
/// cent). The schema defaults are a nominal 1080p keystone; a fresh instance is
/// put on the actual comp by
/// [`instantiate_for_raster`](crate::fx::builtins::instantiate_for_raster), because a
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
    // The matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the pull from the frame's own corners per pixel, read where the \
         pixel lands: white moves it all the way to the pinned position, grey \
         part way, black leaves it where it was",
    ),
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
        hard_max = 100.0,
        unit = Percent
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
        let mut inv = adjugate(m);
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

    /// [`Self::packed`] for a raster `w` by `h`. On a node graph box whose
    /// picture reaches past the frame, it pins the frame and not the whole
    /// raster.
    #[must_use]
    pub fn packed_in(p: Params<'_>, w: u32, h: u32) -> cpu::CornerPinParams {
        let mut packed = Self::read(p).packed();
        if let Some([x, y, fw, fh]) = crate::fx::schema::frame_rect(p) {
            // The kernel spreads the unit square over the raster, so this
            // spreads it over the frame instead and divides the raster back out.
            let [a, b, d] = packed.inv;
            let row = |r: [f32; 3], size: f32, at: f32, n: u32| {
                [0, 1, 2].map(|i| (r[i] * size + d[i] * at) / n as f32)
            };
            packed.inv = [row(a, fw, x, w), row(b, fh, y, h), d];
        }
        packed
    }
}

/// `m`'s adjugate: its inverse times its determinant.
fn adjugate(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
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
    ]
}

/// Corner pin's behaviour.
pub struct CornerPinDef;

impl EffectDef for CornerPinDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CornerPin as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::corner_pin(rgba, w, h, &CornerPin::packed_in(p, w, h));
    }

    /// The picture's corners carried onto the four points. Past the horizon it
    /// reaches without end, so it gets all the room there is. Repeat and Mirror
    /// fill what they are handed, and a matte or a Mix can leave the picture
    /// where it was, so the rectangle it was handed is always kept as well.
    fn bounds(&self, p: Params<'_>, rect: [f32; 4]) -> [f32; 4] {
        let c = CornerPin::read(p).packed();
        if !c.active || c.edge != 0 {
            return rect;
        }
        let [x, y, w, h] = crate::fx::schema::frame_rect(p).unwrap_or([
            rect[0],
            rect[1],
            rect[2] - rect[0],
            rect[3] - rect[1],
        ]);
        let m = adjugate(c.inv);
        // The kernel keeps a pixel where `inv` gives it a positive w, and
        // `inv` times `m` is this times the identity.
        let side = c.inv[0][0] * m[0][0] + c.inv[0][1] * m[1][0] + c.inv[0][2] * m[2][0];
        // Half a pixel out for the bilinear tap.
        let [x0, y0, x1, y1] = [rect[0] - 0.5, rect[1] - 0.5, rect[2] + 0.5, rect[3] + 0.5];
        let mut out = rect;
        for (px, py) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let (u, v) = ((px - x) / w, (py - y) / h);
            let [sx, sy, sw] = m.map(|r| r[0] * u + r[1] * v + r[2]);
            if sw * side <= 0.0 || sw.is_nan() {
                let far = crate::largest_picture::side() as f32;
                return [rect[0] - far, rect[1] - far, rect[2] + far, rect[3] + far];
            }
            let (sx, sy) = (sx / sw, sy / sw);
            out = [
                out[0].min(sx),
                out[1].min(sy),
                out[2].max(sx),
                out[3].max(sy),
            ];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::params::{ParamId, Value};

    fn pin(corners: [f32; 8], frame: [f32; 4]) -> Vec<(ParamId, Value)> {
        let ids = [
            "upper_left_x",
            "upper_left_y",
            "upper_right_x",
            "upper_right_y",
            "lower_left_x",
            "lower_left_y",
            "lower_right_x",
            "lower_right_y",
        ];
        let mut p: Vec<_> = ids
            .iter()
            .zip(corners)
            .map(|(id, v)| (ParamId::new(id), Value::Float(v)))
            .collect();
        p.push((ParamId::new("mix"), Value::Float(100.0)));
        p.push((crate::fx::schema::FRAME_RECT, Value::Vec4(frame)));
        p
    }

    /// Pulled out past the frame, a node graph box gives it room out to
    /// the corners. On a raster grown past the frame, the frame is what is
    /// pinned, so a pin onto the frame's own corners leaves every pixel be.
    #[test]
    fn a_pin_reaches_its_corners_and_pins_the_frame() {
        let out = pin(
            [-32.0, -32.0, 96.0, -32.0, -32.0, 96.0, 96.0, 96.0],
            [0.0, 0.0, 64.0, 64.0],
        );
        let r = CornerPinDef.bounds(Params::new(&out), [0.0, 0.0, 64.0, 64.0]);
        assert!(
            r[0] <= -32.0 && r[1] <= -32.0 && r[2] >= 96.0 && r[3] >= 96.0,
            "{r:?}"
        );

        let (n, at) = (128u32, 32.0);
        let held = pin(
            [at, at, at + 64.0, at, at, at + 64.0, at + 64.0, at + 64.0],
            [at, at, 64.0, 64.0],
        );
        let picture: Vec<f32> = (0..n * n)
            .flat_map(|i| {
                let (x, y) = (i % n, i / n);
                let inside = (32..96).contains(&x) && (32..96).contains(&y);
                let v = if inside { (x + y) as f32 / 256.0 } else { 0.0 };
                [v, v, v, f32::from(u8::from(inside))]
            })
            .collect();
        let mut rgba = picture.clone();
        CornerPinDef.apply_cpu(&mut rgba, n, n, Params::new(&held));
        let worst = rgba
            .iter()
            .zip(&picture)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-3, "{worst}");
    }
}
