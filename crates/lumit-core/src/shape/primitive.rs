//! Parametric shapes: a rectangle, an ellipse, a polygon or a star kept as the
//! numbers that make it, so its size, roundness and points can be keyed, as
//! After Effects' Rectangle, Ellipse and Polystar paths are.
//!
//! The path is made from the numbers whenever it is read, so everything that
//! draws, bounds or trims a shape item sees an ordinary path.

use serde::{Deserialize, Serialize};

use crate::anim::{Animation, Property};
use crate::mask::{BezierPath, Mask, Vertex};

/// The most points a polygon or a star has, as in After Effects.
pub const MAX_POINTS: f64 = 100.0;

/// The circle-from-cubics constant, the same one `Mask::ellipse` uses.
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// A shape made from numbers. `kind` 0 is a rectangle, 1 an ellipse, 2 a
/// polygon and 3 a star. The position is the middle of the shape, in the art's
/// own coordinates. A rectangle and an ellipse read the width and height, and
/// the rectangle its corner roundness in pixels. A polygon and a star read the
/// points, the turn in degrees and the radii, and the roundness of their
/// corners per cent; a polygon has only the outer ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapePrimitive {
    pub kind: u32,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub position_x: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub position_y: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub width: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub height: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub roundness: Property,
    #[serde(default = "five", with = "crate::mask::still_or_keyed")]
    pub points: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub rotation: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub outer_radius: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub inner_radius: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub outer_roundness: Property,
    #[serde(default = "Property::zero", with = "crate::mask::still_or_keyed")]
    pub inner_roundness: Property,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn five() -> Property {
    Property::fixed(5.0)
}

impl ShapePrimitive {
    fn numbers(&self) -> [&Property; 11] {
        [
            &self.position_x,
            &self.position_y,
            &self.width,
            &self.height,
            &self.roundness,
            &self.points,
            &self.rotation,
            &self.outer_radius,
            &self.inner_radius,
            &self.outer_roundness,
            &self.inner_roundness,
        ]
    }

    /// Whether any of its numbers moves, by keys or by an expression.
    pub fn is_animated(&self) -> bool {
        self.numbers()
            .iter()
            .any(|p| !matches!(p.animation, Animation::Static(_)))
    }

    /// The path these numbers make at `t`, on the layer's clock.
    pub fn path_at(&self, t: f64) -> BezierPath {
        let at = |p: &Property| {
            let v = p.value_at(t);
            if v.is_finite() {
                v
            } else {
                0.0
            }
        };
        let (cx, cy) = (at(&self.position_x), at(&self.position_y));
        let (w, h) = (at(&self.width).max(0.0), at(&self.height).max(0.0));
        match self.kind {
            0 => rectangle(cx, cy, w, h, at(&self.roundness)),
            1 => Mask::ellipse(cx, cy, w / 2.0, h / 2.0).path,
            kind => {
                let star = kind == 3;
                let (outer, inner) = (at(&self.outer_radius), at(&self.inner_radius));
                let (outer_round, inner_round) =
                    (at(&self.outer_roundness), at(&self.inner_roundness));
                let points = at(&self.points).round().clamp(3.0, MAX_POINTS) as usize;
                let count = if star { points * 2 } else { points };
                let step = std::f64::consts::TAU / count as f64;
                // The first point is at the top, turned by the rotation.
                let start = at(&self.rotation).to_radians() - std::f64::consts::FRAC_PI_2;
                let vertices = (0..count)
                    .map(|i| {
                        let (r, round) = if star && i % 2 == 1 {
                            (inner, inner_round)
                        } else {
                            (outer, outer_round)
                        };
                        let (sin, cos) = (start + step * i as f64).sin_cos();
                        // Handles along the circle through the point: at 100
                        // per cent a polygon is that circle.
                        let reach = r * 4.0 / 3.0 * (step / 4.0).tan() * round / 100.0;
                        Vertex {
                            pos: (cx + r * cos, cy + r * sin),
                            tan_in: (sin * reach, -cos * reach),
                            tan_out: (-sin * reach, cos * reach),
                        }
                    })
                    .collect();
                BezierPath {
                    vertices,
                    closed: true,
                }
            }
        }
    }
}

/// A rectangle round its middle, its corners rounded by `r` pixels: four
/// corners, or two points a corner once they are rounded. The same points the
/// rectangle tools draw, in the same order.
fn rectangle(cx: f64, cy: f64, w: f64, h: f64, r: f64) -> BezierPath {
    let (left, top) = (cx - w / 2.0, cy - h / 2.0);
    let r = r.clamp(0.0, w.min(h) / 2.0);
    if r <= 0.0 {
        return Mask::rectangle(left, top, w, h).path;
    }
    let (right, bottom) = (left + w, top + h);
    let k = r * KAPPA;
    let v = |pos, tan_in, tan_out| Vertex {
        pos,
        tan_in,
        tan_out,
    };
    BezierPath {
        vertices: vec![
            v((left + r, top), (-k, 0.0), (0.0, 0.0)),
            v((right - r, top), (0.0, 0.0), (k, 0.0)),
            v((right, top + r), (0.0, -k), (0.0, 0.0)),
            v((right, bottom - r), (0.0, 0.0), (0.0, k)),
            v((right - r, bottom), (k, 0.0), (0.0, 0.0)),
            v((left + r, bottom), (0.0, 0.0), (-k, 0.0)),
            v((left, bottom - r), (0.0, k), (0.0, 0.0)),
            v((left, top + r), (0.0, 0.0), (0.0, -k)),
        ],
        closed: true,
    }
}
