//! Fill: the edges a steadied frame drags into view, filled from the frames
//! either side of it.
//!
//! In plain terms: the analysis already measured how the picture moved from
//! each frame to the next. Chaining those steps says where a point of this
//! frame is on a frame a little before or after, so the part of the scene this
//! frame does not reach can be read off the frames that do. The nearest frame
//! in time wins wherever it reaches, which keeps the fill sharp, and every
//! frame fades out over the feather at its own edge. Where no frame reaches,
//! the picture stays empty.
//!
//! The CPU half here is the reference the `stabilise_fill` kernel in
//! `fx_stabilise.wgsl` is held to, op for op.

use super::analysis::Step;
use super::motion::{invert, mul, Mat3, IDENTITY};
use super::path::Model;
use super::plan::judged_by;
use super::warp::{bicubic, unroll};
use crate::fx::effects::stabilise;
use crate::fx::params::{ParamId, Params, Value};
use crate::model::EffectInstance;

/// The most neighbours one frame is filled from.
pub const MAX_NEIGHBOURS: usize = 8;

/// `(1, count, 0, 0)` when the frame is filled, followed by `count` neighbour
/// maps. Absent otherwise, so a plain frame's bag is what it always was.
pub const DERIVED_FILL: ParamId = ParamId::new("derived.fill");

/// Neighbour `k`'s map, from a pixel of this frame to the same point of the
/// neighbour, as three rows in the layer's own pixels. The first row's `w` is
/// the neighbour's offset, which names the decoded frame it is laid with.
pub const NEIGHBOUR_ROWS: [[ParamId; 3]; MAX_NEIGHBOURS] = [
    [
        ParamId::new("derived.fill0_0"),
        ParamId::new("derived.fill0_1"),
        ParamId::new("derived.fill0_2"),
    ],
    [
        ParamId::new("derived.fill1_0"),
        ParamId::new("derived.fill1_1"),
        ParamId::new("derived.fill1_2"),
    ],
    [
        ParamId::new("derived.fill2_0"),
        ParamId::new("derived.fill2_1"),
        ParamId::new("derived.fill2_2"),
    ],
    [
        ParamId::new("derived.fill3_0"),
        ParamId::new("derived.fill3_1"),
        ParamId::new("derived.fill3_2"),
    ],
    [
        ParamId::new("derived.fill4_0"),
        ParamId::new("derived.fill4_1"),
        ParamId::new("derived.fill4_2"),
    ],
    [
        ParamId::new("derived.fill5_0"),
        ParamId::new("derived.fill5_1"),
        ParamId::new("derived.fill5_2"),
    ],
    [
        ParamId::new("derived.fill6_0"),
        ParamId::new("derived.fill6_1"),
        ParamId::new("derived.fill6_2"),
    ],
    [
        ParamId::new("derived.fill7_0"),
        ParamId::new("derived.fill7_1"),
        ParamId::new("derived.fill7_2"),
    ],
];

/// The neighbours a fill reaching `reach` frames either side reads, as
/// offsets: four each side, spread from the next frame out to the farthest, so
/// a quick shake and a slow drift both find a frame that covers the edge.
/// Farthest first, the order they are laid down in.
#[must_use]
pub fn offsets(reach: u32) -> Vec<i32> {
    let r = f64::from(reach.max(1));
    let mut far: Vec<i32> = (0..4)
        .map(|k| r.powf(f64::from(k) / 3.0).round().clamp(1.0, r) as i32)
        .collect();
    far.sort_unstable_by(|a, b| b.cmp(a));
    far.dedup();
    far.iter().flat_map(|&d| [d, -d]).collect()
}

/// The neighbours a Fill at layer frame `frame`, counted in frames of a comp
/// running at `fps`, reads as offsets in those frames: none unless the edges
/// are filled and an analysis is in hand. Fill from is seconds on the layer's
/// clock, whatever the footage's own rate. Read where no layer context is in
/// hand (the decode asks it too), so an expression on Fill from runs without
/// one, alike for both askers.
#[must_use]
pub fn reads(inst: &EffectInstance, frame: f64, fps: f64) -> Vec<i32> {
    let choice = |id: &str, fallback: u32| super::rows::choice(inst, id, fallback);
    if choice("mode", 0) == stabilise::MODE_REVERSE
        || choice("edges", stabilise::EDGES_DEFAULT) != stabilise::EDGES_FILL
    {
        return Vec::new();
    }
    if crate::stabilise::analysis::filed(inst.id).is_none() {
        return Vec::new();
    }
    let fps = fps.max(1.0);
    let seconds = inst
        .float_at("fill_from", frame / fps)
        .unwrap_or(0.5)
        .clamp(0.0, 10.0);
    offsets((seconds * fps).round().clamp(1.0, 10_000.0) as u32)
}

/// Where a point of analysed frame `from` is on analysed frame `to`: the steps
/// between them chained, each its four-corner fit, or its similarity where
/// only that was trusted. `None` across a cut or a step with no usable motion,
/// since a guess there would put the wrong part of the scene in the edge.
#[must_use]
pub fn chain(steps: &[Step], from: usize, to: usize) -> Option<Mat3> {
    let usable = |k: usize| judged_by(steps.get(k)?, Model::Perspective);
    let mut m = IDENTITY;
    if to >= from {
        for k in from..to {
            m = mul(&usable(k)?, &m);
        }
    } else {
        for k in (to..from).rev() {
            m = mul(&invert(&usable(k)?)?, &m);
        }
    }
    Some(m)
}

/// The maps a frame showing analysed frame `at` is filled with, from the
/// neighbours `shown` (each offset with the analysed frame it shows): farthest
/// in time first, and without those outside the analysis, on this frame
/// itself, or across a cut or an untrusted step. Each with its offset and the
/// analysed frame it reads.
#[must_use]
pub fn neighbours(
    steps: &[Step],
    frames: usize,
    at: f64,
    shown: &[(i32, f64)],
) -> Vec<(i32, usize, Mat3)> {
    let whole = |f: f64| {
        (f.is_finite() && f.round() >= 0.0 && f.round() < frames as f64).then(|| f.round() as usize)
    };
    let Some(i) = whole(at) else {
        return Vec::new();
    };
    let mut found: Vec<(usize, i32, usize, Mat3)> = shown
        .iter()
        .filter_map(|&(o, f)| {
            let j = whole(f).filter(|&j| j != i)?;
            Some((j.abs_diff(i), o, j, chain(steps, i, j)?))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    found
        .into_iter()
        .take(MAX_NEIGHBOURS)
        .map(|(_, o, j, m)| (o, j, m))
        .collect()
}

/// Hand the neighbour maps to the GPU op as the values it reads.
pub fn push(push: &mut dyn FnMut(ParamId, Value), maps: &[(i32, usize, Mat3)]) {
    let maps = maps.get(..maps.len().min(MAX_NEIGHBOURS)).unwrap_or(&[]);
    push(
        DERIVED_FILL,
        Value::Vec4([1.0, maps.len() as f32, 0.0, 0.0]),
    );
    for ((o, _, m), ids) in maps.iter().zip(NEIGHBOUR_ROWS.iter()) {
        for (r, id) in ids.iter().enumerate() {
            let w = if r == 0 { *o as f32 } else { 0.0 };
            push(
                *id,
                Value::Vec4([m[r][0] as f32, m[r][1] as f32, m[r][2] as f32, w]),
            );
        }
    }
}

/// One neighbour as the raster reads it: its offset, the map from a pixel of
/// this frame to the same point of it, and how it moved while it was read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbour {
    pub offset: i32,
    pub to_frame: [[f32; 3]; 3],
    pub motion: [f32; 4],
}

/// A filled frame in the raster being drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    /// The part of every frame that is used, `[x0, y0, x1, y1]`: inside the
    /// trims.
    pub rect: [f32; 4],
    /// How far in from that edge a frame fades in, in pixels.
    pub feather: f32,
    /// Farthest first.
    pub neighbours: Vec<Neighbour>,
}

/// The fill for a `w` by `h` raster from the resolved rows, `None` unless the
/// frame is filled. `s` is the raster's pixels to one of the layer's, which the
/// neighbour maps are conjugated by as the frame's own map is, and `off` how
/// far in the layer's frame sits on a raster an earlier effect grew. The
/// rectangle every frame is used inside is the frame's, never the grown
/// raster's.
#[must_use]
pub fn fill_params(p: Params<'_>, w: u32, h: u32, s: f32, off: [f32; 2]) -> Option<Fill> {
    let on = p.vec4(DERIVED_FILL, [0.0; 4]);
    if on[0] == 0.0 {
        return None;
    }
    let px = |id: &str| p.float(ParamId::new(id), 0.0).max(0.0);
    let count = (on[1].max(0.0) as usize).min(MAX_NEIGHBOURS);
    let neighbours = NEIGHBOUR_ROWS
        .iter()
        .take(count)
        .enumerate()
        .map(|(k, ids)| {
            let r = ids.map(|id| p.vec4(id, [0.0; 4]));
            let to_frame = [
                [r[0][0], r[0][1], r[0][2] * s],
                [r[1][0], r[1][1], r[1][2] * s],
                [r[2][0] / s, r[2][1] / s, r[2][2]],
            ];
            Neighbour {
                offset: r[0][3] as i32,
                to_frame: super::warp::about(to_frame, off),
                motion: super::shutter::neighbour(p, k, s, off),
            }
        })
        .collect();
    Some(Fill {
        rect: [
            off[0] + px("trim_left"),
            off[1] + px("trim_top"),
            w as f32 - off[0] - px("trim_right"),
            h as f32 - off[1] - px("trim_bottom"),
        ],
        feather: px("fill_feather"),
        neighbours,
    })
}

/// The map a frame's own pixels are laid with: each point is where it is.
pub const SAME_FRAME: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// How much of a frame shows at `q`, a point of it: all of it farther than
/// `feather` inside `rect`, fading to none at the edge, none outside.
#[must_use]
pub fn coverage(q: [f32; 2], rect: [f32; 4], feather: f32) -> f32 {
    let d = (q[0] - rect[0])
        .min(rect[2] - q[0])
        .min(q[1] - rect[1])
        .min(rect[3] - q[1]);
    if feather > 0.0 {
        (d / feather).clamp(0.0, 1.0)
    } else if d >= 0.0 {
        1.0
    } else {
        0.0
    }
}

/// One frame of a fill, as one pass of the kernel lays it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    /// Output pixel to a pixel of the frame being drawn.
    pub map: [[f32; 3]; 3],
    /// That pixel to the same point of the frame laid in this pass.
    pub to_frame: [[f32; 3]; 3],
    pub rect: [f32; 4],
    pub feather: f32,
    /// How the laid frame moved while it was read, and the rolling shutter:
    /// see [`unroll`]. A nought readout is no term.
    pub motion: [f32; 4],
    pub readout: [f32; 4],
}

fn times(m: &[[f32; 3]; 3], x: f32, y: f32) -> [f32; 3] {
    [
        m[0][0] * x + m[0][1] * y + m[0][2],
        m[1][0] * x + m[1][1] * y + m[1][2],
        m[2][0] * x + m[2][1] * y + m[2][2],
    ]
}

/// Lay `src` (`w` by `h`, premultiplied RGBA) over `under` through `layer`:
/// each output pixel reads the frame where the two maps take it, and the
/// frame's coverage there decides how much of it replaces what is under it. `None` under is the
/// first pass, with nothing beneath.
#[must_use]
pub fn lay(src: &[f32], under: Option<&[f32]>, w: u32, h: u32, layer: &Layer) -> Vec<f32> {
    let mut out = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let base = under
                .and_then(|u| u.get(i..i + 4))
                .map_or([0.0; 4], |b| [b[0], b[1], b[2], b[3]]);
            let mut v = base;
            let q = times(&layer.map, x as f32 + 0.5, y as f32 + 0.5);
            if q[2] > 0.0 {
                let f = times(&layer.to_frame, q[0] / q[2], q[1] / q[2]);
                if f[2] > 0.0 {
                    let mut at = [f[0] / f[2], f[1] / f[2]];
                    if layer.readout[0] != 0.0 {
                        at = unroll(at, layer.motion, layer.readout);
                    }
                    // Laid by how much of the frame reaches here, never by
                    // its picture's own alpha: where the frame covers, its
                    // transparency stays transparent.
                    let k = coverage(at, layer.rect, layer.feather);
                    if k > 0.0 {
                        let c = bicubic(src, w, h, at[0], at[1]);
                        v = [0, 1, 2, 3].map(|n| c[n] * k + base[n] * (1.0 - k));
                    }
                }
            }
            if let Some(dst) = out.get_mut(i..i + 4) {
                dst.copy_from_slice(&v);
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Where the frame itself covers, its own transparency stays
    /// transparent: a neighbour laid under it never shows through the
    /// picture, only round its edges.
    #[test]
    fn a_transparent_picture_stays_transparent_where_its_frame_covers() {
        let (w, h) = (8u32, 4u32);
        let clear = vec![0.0f32; (w * h * 4) as usize];
        let solid: Vec<f32> = [0.2, 0.4, 0.6, 1.0].repeat((w * h) as usize);
        let laid = |to_frame| Layer {
            map: SAME_FRAME,
            to_frame,
            rect: [0.0, 0.0, w as f32, h as f32],
            feather: 0.0,
            motion: [0.0; 4],
            readout: [0.0; 4],
        };
        let under = lay(&solid, None, w, h, &laid(SAME_FRAME));
        let out = lay(&clear, Some(&under), w, h, &laid(SAME_FRAME));
        assert!(out.iter().all(|&v| v == 0.0), "{out:?}");
    }

    /// Fill from is seconds on the layer's clock, so it reaches as far in a
    /// comp at any rate, whatever rate the footage was shot at.
    #[test]
    fn fill_from_reaches_its_seconds_whatever_the_footage_rate() {
        use crate::model::EffectValue;
        let mut inst = crate::fx::builtins::instantiate(stabilise::MATCH_NAME).unwrap();
        for p in &mut inst.params {
            if p.id == "edges" {
                p.value = EffectValue::Choice(stabilise::EDGES_FILL);
            }
        }
        for shot in [60.0, 24.0] {
            let analysis = crate::stabilise::analysis::Analysis {
                fps: shot,
                rate_scale: 1.0,
                width: 8,
                height: 8,
                scale: 1.0,
                first: 0,
                frames: 2,
                asked: 2,
                steps: Vec::new(),
                runs: Vec::new(),
                readout: [0.0; 2],
            };
            crate::stabilise::analysis::publish(inst.id, [1; 32], inst.id, analysis);
            for comp in [24.0, 60.0] {
                let far = reads(&inst, 0.0, comp).into_iter().max().unwrap();
                assert_eq!(f64::from(far), 0.5 * comp, "{shot} fps footage in {comp}");
            }
        }
    }
}
