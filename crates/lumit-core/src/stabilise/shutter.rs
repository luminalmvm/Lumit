//! The rolling shutter: how the picture moved while each frame was read, and
//! the values the warp is handed to take the skew out.
//!
//! In plain terms: the analysis measured how long the sensor took to read a
//! frame ([`super::analysis::Analysis::readout`]). While a frame was read the
//! picture moved by the mean of the steps either side of it, its slide, turn
//! and growth. A pixel read late is found that much further along that motion
//! ([`super::warp::unroll`]). Only the frame's slide, turn and growth are read
//! per row; the four-corner fit is applied once per frame, by the map.

use super::analysis::{Analysis, Step};
use super::fill::MAX_NEIGHBOURS;
use super::shown::Shown;
use crate::fx::params::{ParamId, Params, Value};

/// The frame's motion while it was read, `(α, β, vx, vy)` in the layer's own
/// pixels a frame: see [`super::warp::unroll`].
pub const DERIVED_MOTION: ParamId = ParamId::new("derived.shutter");

/// `(r, axis, 0, 0)`: the read time with the Amount on it, and the axis it ran
/// along. Absent when the term is off, so a plain frame's bag is what it
/// always was.
pub const DERIVED_READOUT: ParamId = ParamId::new("derived.readout");

/// Neighbour `k`'s motion while it was read, as [`DERIVED_MOTION`] is the
/// frame's own. Pushed beside [`super::fill::NEIGHBOUR_ROWS`] only while the
/// term is on.
pub const NEIGHBOUR_MOTION: [ParamId; MAX_NEIGHBOURS] = [
    ParamId::new("derived.fill0_3"),
    ParamId::new("derived.fill1_3"),
    ParamId::new("derived.fill2_3"),
    ParamId::new("derived.fill3_3"),
    ParamId::new("derived.fill4_3"),
    ParamId::new("derived.fill5_3"),
    ParamId::new("derived.fill6_3"),
    ParamId::new("derived.fill7_3"),
];

/// The read time and its axis (0 along x, 1 along y), or `None` when the
/// analysis saw no rolling shutter.
#[must_use]
pub fn readout(a: &Analysis) -> Option<(f64, f64)> {
    match a.readout {
        [x, _] if x != 0.0 => Some((f64::from(x), 0.0)),
        [_, y] if y != 0.0 => Some((f64::from(y), 1.0)),
        _ => None,
    }
}

/// How the picture moved while each analysed frame was read: the mean of the
/// two steps either side, each as its similarity's move `(a − 1, b, tx, ty)`.
/// With only one of them trusted that one; with neither, or both across a cut,
/// no motion.
#[must_use]
pub fn motions(steps: &[Step], frames: usize) -> Vec<[f64; 4]> {
    let part = |k: Option<usize>| {
        let s = steps.get(k?).filter(|s| s.trusted_similarity())?;
        let m = s.similarity;
        Some([m[0] - 1.0, m[1], m[2], m[3]])
    };
    (0..frames)
        .map(|t| match (part(t.checked_sub(1)), part(Some(t))) {
            (Some(a), Some(b)) => [0, 1, 2, 3].map(|i| 0.5 * (a[i] + b[i])),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => [0.0; 4],
        })
        .collect()
}

/// The motion while the frames `shown` were read: one frame's, or two mixed
/// by the weight their maps are.
#[must_use]
pub fn motion_at(motions: &[[f64; 4]], shown: Shown) -> [f64; 4] {
    let one = |i: usize| motions.get(i).copied().unwrap_or([0.0; 4]);
    shown.mix(one, |a, b, w| {
        [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * w)
    })
}

fn vec4(m: [f64; 4]) -> Value {
    Value::Vec4(m.map(|v| v as f32))
}

/// Hand the warp the frame's motion and the read time `r` along `axis`.
pub fn push(push: &mut dyn FnMut(ParamId, Value), motion: [f64; 4], r: f64, axis: f64) {
    push(DERIVED_MOTION, vec4(motion));
    push(DERIVED_READOUT, vec4([r, axis, 0.0, 0.0]));
}

/// Hand the fill each neighbour's motion, in the order of its maps.
pub fn push_neighbours(push: &mut dyn FnMut(ParamId, Value), motions: &[[f64; 4]]) {
    for (m, id) in motions.iter().zip(NEIGHBOUR_MOTION.iter()) {
        push(*id, vec4(*m));
    }
}

/// A motion in the layer's pixels as the raster reads it: the slide scaled by
/// `s` raster pixels to one of the layer's, the turn and growth unchanged, and
/// taken about the frame's corner where it sits `off` pixels in.
fn in_raster(m: [f32; 4], s: f32, off: [f32; 2]) -> [f32; 4] {
    let [ox, oy] = off;
    [
        m[0],
        m[1],
        m[2] * s - m[0] * ox + m[1] * oy,
        m[3] * s - m[1] * ox - m[0] * oy,
    ]
}

/// The frame's motion and the readout `(r, axis, size, start)` in a raster
/// with `s` pixels to one of a `layer` sized layer's, the frame sitting `off`
/// pixels in, or `None` when the term is off, which leaves the warp exactly
/// as it was.
#[must_use]
pub fn params(
    p: Params<'_>,
    layer: [f32; 2],
    s: f32,
    off: [f32; 2],
) -> Option<([f32; 4], [f32; 4])> {
    let ro = p.vec4(DERIVED_READOUT, [0.0; 4]);
    let y = ro[1] > 0.5;
    let size = if y { layer[1] } else { layer[0] } * s;
    let start = if y { off[1] } else { off[0] };
    (ro[0] != 0.0 && size > 0.0).then(|| {
        (
            in_raster(p.vec4(DERIVED_MOTION, [0.0; 4]), s, off),
            [ro[0], ro[1], size, start],
        )
    })
}

/// Neighbour `k`'s motion in the raster, as [`params`] reads the frame's.
#[must_use]
pub fn neighbour(p: Params<'_>, k: usize, s: f32, off: [f32; 2]) -> [f32; 4] {
    NEIGHBOUR_MOTION
        .get(k)
        .map_or([0.0; 4], |id| in_raster(p.vec4(*id, [0.0; 4]), s, off))
}
