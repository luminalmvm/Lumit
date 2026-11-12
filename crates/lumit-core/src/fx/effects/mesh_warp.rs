//! Mesh warp: the picture bent by a grid of points, as After Effects' Mesh Warp
//! is.
//!
//! **In plain terms.** A grid is laid over the layer, Rows by Columns cells.
//! Every point where the lines cross can be dragged, and so can the two
//! handles each point carries, and the picture follows smoothly, so a face can
//! be pulled into a grin, a logo bulged in one corner, a flag bent along its
//! length. Corner pin keeps the edges straight and Bezier warp bends only the
//! edges; this bends the inside too.
//!
//! **Where the points live.** Each crossing is a pair of numbers in per cent of
//! the layer: 0, 0 is its upper left corner and 100, 100 its lower right. So the
//! same mesh fits a shape, a line of text, a solid or a 4K clip, at any preview
//! resolution, without knowing how large the layer is.
//!
//! **Each crossing has two handles**, as in After Effects: one along its row
//! and one along its column, each held as how far it reaches from the crossing,
//! per cent of the layer, and mirrored on the other side. A fresh handle reaches
//! a third of the way to the next crossing, which is what keeps a flat grid
//! flat. Dragging a crossing takes its handles with it.
//!
//! **The crossings are the instance's own rows**, derived from Rows and
//! Columns ([`EffectDef::derived`]): each animates, wires and saves as any other
//! row. The grid's size is in every row's id, so a new grid starts flat, as
//! After Effects resets its mesh when Rows or Columns change, and the rows of
//! the old grid are left behind untouched. The panel hides them; they are
//! dragged in the Viewer.
//!
//! **Between the points** each cell is a bicubic Bezier patch: its edges are
//! the curves the crossings and their handles draw, and its inside follows
//! them. Rendering asks where each output pixel came from, so every pixel
//! solves the mesh backwards by Newton's method, as Bezier warp does, from its
//! own place first and from the nearest crossing when that fails.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::fx::{
    cpu,
    params::{ParamId, Params, Unit},
    registry::EffectDef,
    registry::EffectMetadata,
    schema::{EffectSchema, ParamKind, ParamSchema},
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// The most cells along either side, as in After Effects.
pub const MAX_CELLS: u32 = 31;

/// The most crossings a mesh has: (31 + 1) × (31 + 1).
pub const MAX_VERTICES: usize = ((MAX_CELLS + 1) * (MAX_CELLS + 1)) as usize;

/// The six rows of one crossing, by the end of their id: where it is, how far
/// its row handle reaches, how far its column handle reaches.
pub const AXES: [&str; 6] = ["x", "y", "hx", "hy", "vx", "vy"];

/// Mesh warp's controls. The crossings themselves are derived rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "mesh_warp",
    label = "Mesh warp",
    version = 1,
    category = Distortion,
    // A few Newton steps a pixel, each a sixteen-point patch and its slope.
    cost = Moderate,
    // Any output pixel can be drawn from anywhere in the layer.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct MeshWarp {
    /// Cells down the layer. A new number starts a new, flat grid.
    #[counter(min = 1, max = 31, default = 7, hard_min = 1, hard_max = 31, unit = Raw)]
    pub rows: i32,
    /// Cells across the layer. A new number starts a new, flat grid.
    #[counter(min = 1, max = 31, default = 7, hard_min = 1, hard_max = 31, unit = Raw)]
    pub columns: i32,
    /// How many Newton steps each pixel takes to find where it came from.
    #[counter(min = 1, max = 12, default = 8, hard_min = 1, hard_max = 12, unit = Raw)]
    pub quality: i32,
    /// After Effects' stopwatch on Distortion Mesh: on, a drag in the Viewer
    /// keys every crossing at the playhead, so a key holds the whole mesh.
    #[toggle(label = "Animate mesh", default = false)]
    pub animate_mesh: bool,
    /// Every crossing back where a flat grid has it, keys and all. A button,
    /// not a value; the panel does it.
    #[action(label = "Reset mesh")]
    pub reset_mesh: (),
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

/// The grid's size, held to what a mesh can be.
#[must_use]
pub fn clamp_grid(rows: i64, columns: i64) -> (u32, u32) {
    let side = |n: i64| n.clamp(1, i64::from(MAX_CELLS)) as u32;
    (side(rows), side(columns))
}

/// The id of one crossing's `axis` (one of [`AXES`]), on a grid of `rows` by
/// `columns` cells, `r` lines down and `c` across.
#[must_use]
pub fn vertex_id(rows: u32, columns: u32, r: u32, c: u32, axis: &str) -> String {
    format!("mesh_{rows}x{columns}_r{r}c{c}_{axis}")
}

/// Whether `id` is one of a mesh's rows, of any grid.
#[must_use]
pub fn is_vertex_id(id: &str) -> bool {
    id.starts_with("mesh_")
}

/// Where crossing `(r, c)` sits on a flat grid, per cent of the layer.
#[must_use]
pub fn rest(rows: u32, columns: u32, r: u32, c: u32) -> (f64, f64) {
    (
        f64::from(c) * 100.0 / f64::from(columns.max(1)),
        f64::from(r) * 100.0 / f64::from(rows.max(1)),
    )
}

/// How far a fresh crossing's row handle and column handle reach, per cent of
/// the layer: a third of a cell, so a flat grid draws flat.
#[must_use]
pub fn rest_handles(rows: u32, columns: u32) -> (f64, f64) {
    (
        100.0 / f64::from(columns.max(1)) / 3.0,
        100.0 / f64::from(rows.max(1)) / 3.0,
    )
}

/// What `axis` of crossing `(r, c)` holds on a flat grid, per cent.
#[must_use]
pub fn rest_value(rows: u32, columns: u32, r: u32, c: u32, axis: &str) -> f64 {
    let (x, y) = rest(rows, columns, r, c);
    let (h, v) = rest_handles(rows, columns);
    match axis {
        "x" => x,
        "y" => y,
        "hx" => h,
        "vy" => v,
        _ => 0.0,
    }
}

/// The grid an instance holds: its Rows and Columns, read where they stand.
/// Neither is meant to animate; a keyed one is read at its first key.
#[must_use]
pub fn grid_of(inst: &EffectInstance) -> (u32, u32) {
    let read = |id: &str, default: i64| -> i64 {
        match inst.params.iter().find(|p| p.id == id).map(|p| &p.value) {
            Some(EffectValue::Float(p)) => match &p.animation {
                crate::anim::Animation::Static(v) => v.round() as i64,
                crate::anim::Animation::Keyframed(keys) => {
                    keys.first().map_or(default, |k| k.value.round() as i64)
                }
                crate::anim::Animation::Expression(_) => default,
            },
            _ => default,
        }
    };
    clamp_grid(read("rows", 7), read("columns", 7))
}

/// Every grid this process has derived rows for, by its size.
type RowCache = RwLock<HashMap<(u32, u32), &'static [ParamSchema]>>;

fn cache() -> &'static RowCache {
    static CACHE: OnceLock<RowCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The crossings' rows for a grid, six to a crossing, row by row, built once
/// per size and kept until Lumit closes. At most 31 × 31 sizes, so the leak
/// is bounded.
#[must_use]
pub fn rows_for(rows: u32, columns: u32) -> &'static [ParamSchema] {
    if let Ok(map) = cache().read() {
        if let Some(hit) = map.get(&(rows, columns)) {
            return hit;
        }
    }
    let mut out = Vec::with_capacity(((rows + 1) * (columns + 1) * 6) as usize);
    for r in 0..=rows {
        for c in 0..=columns {
            for (axis, name) in AXES.iter().zip([
                "X",
                "Y",
                "row handle X",
                "row handle Y",
                "column handle X",
                "column handle Y",
            ]) {
                out.push(ParamSchema {
                    id: Box::leak(vertex_id(rows, columns, r, c, axis).into_boxed_str()),
                    label: Box::leak(
                        format!("Row {} column {} {name}", r + 1, c + 1).into_boxed_str(),
                    ),
                    kind: ParamKind::Float {
                        default: rest_value(rows, columns, r, c, axis),
                        slider: (-100.0, 200.0),
                        hard: (None, None),
                    },
                    unit: Unit::Percent,
                });
            }
        }
    }
    let built: &'static [ParamSchema] = Box::leak(out.into_boxed_slice());
    if let Ok(mut map) = cache().write() {
        map.insert((rows, columns), built);
    }
    built
}

/// One resolved Mesh warp.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshWarpParams {
    pub rows: u32,
    pub columns: u32,
    /// The crossings row by row, `(columns + 1)` to a row, as fractions of the
    /// frame: 0, 0 its upper left and 1, 1 its lower right.
    pub pts: Vec<[f32; 2]>,
    /// How far each crossing's row handle reaches, fractions of the frame.
    pub hs: Vec<[f32; 2]>,
    /// How far each crossing's column handle reaches, fractions of the frame.
    pub vs: Vec<[f32; 2]>,
    /// Newton steps a pixel, 1..=12 (the Quality control).
    pub steps: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Where the frame sits on the raster, `[x, y, width, height]`. A zero
    /// width is the whole raster.
    pub frame: [f32; 4],
}

impl MeshWarpParams {
    /// A flat grid of `rows` by `columns` cells.
    #[must_use]
    pub fn flat(rows: u32, columns: u32) -> Self {
        let n = ((rows + 1) * (columns + 1)) as usize;
        let mut pts = Vec::with_capacity(n);
        for r in 0..=rows {
            for c in 0..=columns {
                let (x, y) = rest(rows, columns, r, c);
                pts.push([(x / 100.0) as f32, (y / 100.0) as f32]);
            }
        }
        let (h, v) = rest_handles(rows, columns);
        Self {
            rows,
            columns,
            pts,
            hs: vec![[(h / 100.0) as f32, 0.0]; n],
            vs: vec![[0.0, (v / 100.0) as f32]; n],
            steps: 8,
            mix: 1.0,
            frame: [0.0; 4],
        }
    }

    /// Whether every crossing and handle is where a flat grid has it, so the
    /// effect changes nothing.
    #[must_use]
    pub fn is_flat(&self) -> bool {
        let flat = Self::flat(self.rows, self.columns);
        let near = |a: &[[f32; 2]], b: &[[f32; 2]]| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(p, q)| (p[0] - q[0]).abs() <= 1e-6 && (p[1] - q[1]).abs() <= 1e-6)
        };
        near(&self.pts, &flat.pts) && near(&self.hs, &flat.hs) && near(&self.vs, &flat.vs)
    }

    /// Whether the lists hold one entry for every crossing of the grid.
    #[must_use]
    pub fn is_whole(&self) -> bool {
        let n = ((self.rows + 1) * (self.columns + 1)) as usize;
        self.pts.len() == n && self.hs.len() == n && self.vs.len() == n
    }
}

impl MeshWarp {
    /// The op whose parameters are `p`, as both kernels read it.
    #[must_use]
    pub fn packed_in(p: Params<'_>) -> MeshWarpParams {
        let s = Self::read(p);
        let (rows, columns) = clamp_grid(i64::from(s.rows), i64::from(s.columns));
        let n = ((rows + 1) * (columns + 1)) as usize;
        let (mut pts, mut hs, mut vs) = (
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        );
        // Read once into a map: a 31 by 31 grid is over six thousand rows, and
        // looking each up in the bag's linear list would be millions of
        // comparisons a frame.
        let numbers: HashMap<ParamId, f32> = p
            .iter()
            .filter_map(|(id, value)| match value {
                crate::fx::params::Value::Float(v) => Some((id, v)),
                _ => None,
            })
            .collect();
        for r in 0..=rows {
            for c in 0..=columns {
                let read = |axis: &str| {
                    numbers
                        .get(&ParamId::new(&vertex_id(rows, columns, r, c, axis)))
                        .copied()
                        .unwrap_or(rest_value(rows, columns, r, c, axis) as f32)
                        / 100.0
                };
                pts.push([read("x"), read("y")]);
                hs.push([read("hx"), read("hy")]);
                vs.push([read("vx"), read("vy")]);
            }
        }
        MeshWarpParams {
            rows,
            columns,
            pts,
            hs,
            vs,
            steps: s.quality.clamp(1, 12) as u32,
            mix: (s.mix / 100.0).clamp(0.0, 1.0),
            frame: crate::fx::schema::frame_rect(p).unwrap_or([0.0; 4]),
        }
    }
}

/// The four cubic Bernstein weights at `t`, and their slopes.
fn bernstein(t: f32) -> ([f32; 4], [f32; 4]) {
    let s = 1.0 - t;
    (
        [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t],
        [
            -3.0 * s * s,
            3.0 * s * s - 6.0 * s * t,
            6.0 * s * t - 3.0 * t * t,
            3.0 * t * t,
        ],
    )
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

/// The sixteen control points of cell `(j, i)`, `j` down and `i` across, row by
/// row: its corners, the handles along its edges, and inside the points the
/// two handles of each corner reach together.
#[must_use]
pub fn cell_net(p: &MeshWarpParams, j: u32, i: u32) -> [[f32; 2]; 16] {
    let per_row = p.columns as usize + 1;
    let k = |r: u32, c: u32| r as usize * per_row + c as usize;
    let (a, b, c, d) = (k(j, i), k(j, i + 1), k(j + 1, i), k(j + 1, i + 1));
    let (pa, pb, pc, pd) = (p.pts[a], p.pts[b], p.pts[c], p.pts[d]);
    let (ha, hb, hc, hd) = (p.hs[a], p.hs[b], p.hs[c], p.hs[d]);
    let (va, vb, vc, vd) = (p.vs[a], p.vs[b], p.vs[c], p.vs[d]);
    [
        pa,
        add(pa, ha),
        sub(pb, hb),
        pb,
        add(pa, va),
        add(add(pa, ha), va),
        add(sub(pb, hb), vb),
        add(pb, vb),
        sub(pc, vc),
        sub(add(pc, hc), vc),
        sub(sub(pd, hd), vd),
        sub(pd, vd),
        pc,
        add(pc, hc),
        sub(pd, hd),
        pd,
    ]
}

/// The mesh at `(u, v)` in 0..1 of the grid, and its two slopes: fractions
/// of the frame, for each step of `u` and `v` across the whole grid.
#[must_use]
pub fn surface(p: &MeshWarpParams, u: f32, v: f32) -> ([f32; 2], [f32; 2], [f32; 2]) {
    let (cols, rows) = (p.columns as f32, p.rows as f32);
    let gu = u * cols;
    let gv = v * rows;
    let i = (gu.floor() as i32).clamp(0, p.columns as i32 - 1) as u32;
    let j = (gv.floor() as i32).clamp(0, p.rows as i32 - 1) as u32;
    let (wu, du) = bernstein(gu - i as f32);
    let (wv, dv) = bernstein(gv - j as f32);
    let net = cell_net(p, j, i);
    let mut s = [0.0f32; 2];
    let mut su = [0.0f32; 2];
    let mut sv = [0.0f32; 2];
    for b in 0..4 {
        for a in 0..4 {
            let q = net[b * 4 + a];
            for k in 0..2 {
                s[k] += wu[a] * wv[b] * q[k];
                su[k] += du[a] * wv[b] * q[k] * cols;
                sv[k] += wu[a] * dv[b] * q[k] * rows;
            }
        }
    }
    (s, su, sv)
}

/// The narrowest slope determinant Newton divides by.
const MIN_DET: f32 = 1e-12;

/// How far the solved point may still be from the pixel, raster pixels.
const MAX_RESIDUAL_PX: f32 = 1.0;

/// How near its own centre a sample has to land to be snapped to it.
const SNAP_PX: f32 = 1e-3;

/// Newton from `(u, v)` toward the frame fraction `(tx, ty)`.
fn solve(p: &MeshWarpParams, mut u: f32, mut v: f32, tx: f32, ty: f32) -> (f32, f32) {
    for _ in 0..p.steps {
        let (s, du, dv) = surface(p, u, v);
        let fx = s[0] - tx;
        let fy = s[1] - ty;
        let det = du[0] * dv[1] - du[1] * dv[0];
        if det.abs() < MIN_DET {
            break;
        }
        let inv = 1.0 / det;
        u -= (fx * dv[1] - fy * dv[0]) * inv;
        v -= (du[0] * fy - du[1] * fx) * inv;
    }
    (u, v)
}

/// Whether `(u, v)` is on the grid and lands within the residual of the pixel.
fn lands(p: &MeshWarpParams, u: f32, v: f32, tx: f32, ty: f32, fw: f32, fh: f32) -> bool {
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return false;
    }
    let (s, _, _) = surface(p, u, v);
    ((s[0] - tx) * fw).abs().max(((s[1] - ty) * fh).abs()) <= MAX_RESIDUAL_PX
}

/// Where on the grid the pixel at frame fraction `(tx, ty)` came from, or
/// `None` where no part of the grid reaches.
#[must_use]
pub fn source_of(p: &MeshWarpParams, tx: f32, ty: f32, fw: f32, fh: f32) -> Option<(f32, f32)> {
    // From its own place first: a flat grid's answer, and near it for most
    // pixels of a gentle warp.
    let (u, v) = solve(p, tx, ty, tx, ty);
    if lands(p, u, v, tx, ty, fw, fh) {
        return Some((u, v));
    }
    // Then from the crossing nearest the pixel, for a point dragged far.
    let mut best = (f32::MAX, 0usize);
    for (i, q) in p.pts.iter().enumerate() {
        let dx = (q[0] - tx) * fw;
        let dy = (q[1] - ty) * fh;
        let d = dx * dx + dy * dy;
        if d < best.0 {
            best = (d, i);
        }
    }
    let per_row = p.columns as usize + 1;
    let start_u = (best.1 % per_row) as f32 / p.columns as f32;
    let start_v = (best.1 / per_row) as f32 / p.rows as f32;
    let (u, v) = solve(p, start_u, start_v, tx, ty);
    lands(p, u, v, tx, ty, fw, fh).then_some((u, v))
}

/// Mesh warp on the CPU: the reference the WGSL kernel mirrors op for op.
/// Outside the bent grid is transparent.
pub fn mesh_warp(rgba: &mut [f32], w: u32, h: u32, p: &MeshWarpParams) {
    if !p.is_whole() || p.is_flat() {
        return;
    }
    let original = rgba.to_vec();
    let [ox, oy, fw, fh] = if p.frame[2] > 0.0 {
        p.frame
    } else {
        [0.0, 0.0, w as f32, h as f32]
    };
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let tx = (px - ox) / fw;
            let ty = (py - oy) / fh;
            let v4 = match source_of(p, tx, ty, fw, fh) {
                None => [0.0f32; 4],
                Some((u, v)) => {
                    let mut sx = ox + u * fw;
                    let mut sy = oy + v * fh;
                    if (sx - px).abs() < SNAP_PX && (sy - py).abs() < SNAP_PX {
                        sx = px;
                        sy = py;
                    }
                    cpu::bilinear_edge(&original, w, h, sx, sy, 0)
                }
            };
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v4[c] * p.mix;
            }
        }
    }
}

/// Mesh warp's behaviour.
pub struct MeshWarpDef;

impl EffectDef for MeshWarpDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MeshWarp as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        mesh_warp(rgba, w, h, &MeshWarp::packed_in(p));
    }

    fn derived(&self, inst: &EffectInstance) -> &'static [ParamSchema] {
        let (rows, columns) = grid_of(inst);
        rows_for(rows, columns)
    }

    /// The crossings are dragged in the Viewer, as in After Effects, rather
    /// than shown as a row each.
    fn row_state(&self, inst: &EffectInstance) -> crate::fx::registry::RowState {
        crate::fx::registry::RowState {
            hidden: self.derived(inst).iter().map(|row| row.id).collect(),
            ..crate::fx::registry::RowState::default()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_grid_is_the_layer_itself() {
        let p = MeshWarpParams::flat(3, 4);
        assert!(p.is_flat());
        for (u, v) in [(0.0, 0.0), (0.3, 0.7), (0.99, 0.5), (1.0, 1.0)] {
            let (s, su, sv) = surface(&p, u, v);
            assert!(
                (s[0] - u).abs() < 1e-5 && (s[1] - v).abs() < 1e-5,
                "{u},{v} → {s:?}"
            );
            assert!((su[0] - 1.0).abs() < 1e-4 && su[1].abs() < 1e-4);
            assert!(sv[0].abs() < 1e-4 && (sv[1] - 1.0).abs() < 1e-4);
        }
        let mut rgba: Vec<f32> = (0..8 * 6 * 4).map(|i| i as f32).collect();
        let before = rgba.clone();
        mesh_warp(&mut rgba, 8, 6, &p);
        assert_eq!(rgba, before, "a flat grid changes nothing");
    }

    #[test]
    fn the_mesh_passes_through_every_crossing() {
        let mut p = MeshWarpParams::flat(2, 2);
        p.pts[4] = [0.6, 0.4];
        let (s, _, _) = surface(&p, 0.5, 0.5);
        assert!((s[0] - 0.6).abs() < 1e-5 && (s[1] - 0.4).abs() < 1e-5);
        assert!(!p.is_flat());
    }

    #[test]
    fn a_handle_bends_the_line_it_lies_on() {
        let mut p = MeshWarpParams::flat(1, 2);
        // The middle crossing of the top line, its row handle tipped down.
        p.hs[1] = [p.hs[1][0], 0.2];
        // The handle reaches down on its right and, mirrored, up on its left.
        let (right, _, _) = surface(&p, 0.75, 0.0);
        assert!(
            right[1] > 0.01,
            "the top line bows down on the right: {right:?}"
        );
        let (left, _, _) = surface(&p, 0.25, 0.0);
        assert!(left[1] < -0.01, "and up on the left: {left:?}");
        let (crossing, _, _) = surface(&p, 0.5, 0.0);
        assert!(crossing[1].abs() < 1e-6, "the crossing stays put");
    }

    #[test]
    fn a_pixel_finds_where_it_came_from() {
        let mut p = MeshWarpParams::flat(2, 2);
        p.pts[4] = [0.65, 0.5];
        let (s, _, _) = surface(&p, 0.5, 0.5);
        let (u, v) = source_of(&p, s[0], s[1], 100.0, 100.0).unwrap();
        assert!((u - 0.5).abs() < 1e-3 && (v - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_new_grid_size_names_new_rows() {
        let a = rows_for(2, 3);
        assert_eq!(a.len(), 3 * 4 * 6);
        assert_eq!(a[0].id, "mesh_2x3_r0c0_x");
        assert_eq!(a[a.len() - 1].id, "mesh_2x3_r2c3_vy");
        assert!(rows_for(3, 3)
            .iter()
            .all(|r| !a.iter().any(|b| b.id == r.id)));
        assert_eq!(clamp_grid(0, 99), (1, MAX_CELLS));
    }
}
