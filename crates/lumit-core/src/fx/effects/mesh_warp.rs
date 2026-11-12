//! Mesh warp: the picture bent by a grid of points, as After Effects' Mesh Warp
//! is.
//!
//! **In plain terms.** A grid is laid over the layer, Rows by Columns cells.
//! Every point where the lines cross can be dragged, and the picture follows
//! smoothly between them, so a face can be pulled into a grin, a logo bulged in
//! one corner, a flag bent along its length. Corner pin keeps the edges
//! straight and Bezier warp bends only the edges; this bends the inside too.
//!
//! **Where the points live.** Each crossing is a pair of numbers in per cent of
//! the layer: 0, 0 is its upper left corner and 100, 100 its lower right. So the
//! same mesh fits a shape, a line of text, a solid or a 4K clip, at any preview
//! resolution, without knowing how large the layer is.
//!
//! **The points are the instance's own rows**, derived from Rows and Columns
//! ([`EffectDef::derived`]): each animates, wires and saves as any other row.
//! The grid's size is in every row's id, so a new grid starts flat, as After
//! Effects resets its mesh when Rows or Columns change, and the rows of the old
//! grid are left behind untouched. The panel shows them as one Distortion mesh
//! row; they are dragged in the Viewer.
//!
//! **Between the points** the picture follows a Catmull-Rom surface through
//! them: smooth across every line of the grid, and exactly flat while the grid
//! is. Rendering asks where each output pixel came from, so every pixel solves
//! the surface backwards by Newton's method, as Bezier warp does, from its own
//! place first and from the nearest crossing when that fails.

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

/// Mesh warp's controls. The crossings themselves are derived rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "mesh_warp",
    label = "Mesh warp",
    version = 1,
    category = Distortion,
    // A few Newton steps a pixel, each a sixteen-point surface and its slope.
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

/// The id of one crossing's `axis` ("x" or "y"), on a grid of `rows` by
/// `columns` cells, `r` lines down and `c` across.
#[must_use]
pub fn vertex_id(rows: u32, columns: u32, r: u32, c: u32, axis: &str) -> String {
    format!("mesh_{rows}x{columns}_r{r}c{c}_{axis}")
}

/// Whether `id` is one of a mesh's crossings, of any grid.
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

/// The crossings' rows for a grid, two to a crossing, row by row, built once
/// per size and kept until Lumit closes. At most 31 × 31 sizes, so the leak
/// is bounded.
#[must_use]
pub fn rows_for(rows: u32, columns: u32) -> &'static [ParamSchema] {
    if let Ok(map) = cache().read() {
        if let Some(hit) = map.get(&(rows, columns)) {
            return hit;
        }
    }
    let mut out = Vec::with_capacity(((rows + 1) * (columns + 1) * 2) as usize);
    for r in 0..=rows {
        for c in 0..=columns {
            let (x, y) = rest(rows, columns, r, c);
            for (axis, default, name) in [("x", x, "X"), ("y", y, "Y")] {
                out.push(ParamSchema {
                    id: Box::leak(vertex_id(rows, columns, r, c, axis).into_boxed_str()),
                    label: Box::leak(
                        format!("Row {} column {} {name}", r + 1, c + 1).into_boxed_str(),
                    ),
                    kind: ParamKind::Float {
                        default,
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
    /// Newton steps a pixel, 1..=12 (the Quality control).
    pub steps: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Where the frame sits on the raster, `[x, y, width, height]`. A zero
    /// width is the whole raster.
    pub frame: [f32; 4],
}

impl MeshWarpParams {
    /// Whether every crossing is where a flat grid has it, so the effect
    /// changes nothing.
    #[must_use]
    pub fn is_flat(&self) -> bool {
        let mut i = 0;
        for r in 0..=self.rows {
            for c in 0..=self.columns {
                let (x, y) = rest(self.rows, self.columns, r, c);
                let p = self.pts[i];
                if (f64::from(p[0]) - x / 100.0).abs() > 1e-6
                    || (f64::from(p[1]) - y / 100.0).abs() > 1e-6
                {
                    return false;
                }
                i += 1;
            }
        }
        true
    }
}

impl MeshWarp {
    /// The op whose parameters are `p`, as both kernels read it.
    #[must_use]
    pub fn packed_in(p: Params<'_>) -> MeshWarpParams {
        let s = Self::read(p);
        let (rows, columns) = clamp_grid(i64::from(s.rows), i64::from(s.columns));
        let mut pts = Vec::with_capacity(((rows + 1) * (columns + 1)) as usize);
        for r in 0..=rows {
            for c in 0..=columns {
                let (x, y) = rest(rows, columns, r, c);
                let read = |axis: &str, rest: f64| {
                    p.float(
                        ParamId::new(&vertex_id(rows, columns, r, c, axis)),
                        rest as f32,
                    ) / 100.0
                };
                pts.push([read("x", x), read("y", y)]);
            }
        }
        MeshWarpParams {
            rows,
            columns,
            pts,
            steps: s.quality.clamp(1, 12) as u32,
            mix: (s.mix / 100.0).clamp(0.0, 1.0),
            frame: crate::fx::schema::frame_rect(p).unwrap_or([0.0; 4]),
        }
    }
}

/// The four Catmull-Rom weights at `t`, and their slopes.
fn catmull(t: f32) -> ([f32; 4], [f32; 4]) {
    let t2 = t * t;
    let t3 = t2 * t;
    (
        [
            0.5 * (-t + 2.0 * t2 - t3),
            0.5 * (2.0 - 5.0 * t2 + 3.0 * t3),
            0.5 * (t + 4.0 * t2 - 3.0 * t3),
            0.5 * (-t2 + t3),
        ],
        [
            0.5 * (-1.0 + 4.0 * t - 3.0 * t2),
            0.5 * (-10.0 * t + 9.0 * t2),
            0.5 * (1.0 + 8.0 * t - 9.0 * t2),
            0.5 * (-2.0 * t + 3.0 * t2),
        ],
    )
}

/// Crossing `(r, c)`, with a line of crossings beyond each edge continued
/// straight on, so the surface stays flat at the edges of a flat grid.
fn vertex(p: &MeshWarpParams, r: i32, c: i32) -> [f32; 2] {
    let (rows, cols) = (p.rows as i32, p.columns as i32);
    let at = |r: i32, c: i32| p.pts[(r * (cols + 1) + c) as usize];
    let (r0, c0) = (r.clamp(0, rows), c.clamp(0, cols));
    let (rn, cn) = (
        if r < 0 {
            1
        } else if r > rows {
            rows - 1
        } else {
            r0
        },
        if c < 0 {
            1
        } else if c > cols {
            cols - 1
        } else {
            c0
        },
    );
    let base = at(r0, c0);
    // Reflected through the edge crossing: 2·edge − the one inside it.
    let mut out = base;
    if r != r0 {
        let inner = at(rn, c0);
        out = [2.0 * out[0] - inner[0], 2.0 * out[1] - inner[1]];
    }
    if c != c0 {
        let inner = at(r0, cn);
        let reflected = [2.0 * base[0] - inner[0], 2.0 * base[1] - inner[1]];
        out = [
            out[0] + reflected[0] - base[0],
            out[1] + reflected[1] - base[1],
        ];
    }
    out
}

/// The surface at `(u, v)` in 0..1 of the grid, and its two slopes: fractions
/// of the frame, for each step of `u` and `v` across the whole grid.
#[must_use]
pub fn surface(p: &MeshWarpParams, u: f32, v: f32) -> ([f32; 2], [f32; 2], [f32; 2]) {
    let (cols, rows) = (p.columns as f32, p.rows as f32);
    let gu = u * cols;
    let gv = v * rows;
    let i = (gu.floor() as i32).clamp(0, p.columns as i32 - 1);
    let j = (gv.floor() as i32).clamp(0, p.rows as i32 - 1);
    let (wu, du) = catmull(gu - i as f32);
    let (wv, dv) = catmull(gv - j as f32);
    let mut s = [0.0f32; 2];
    let mut su = [0.0f32; 2];
    let mut sv = [0.0f32; 2];
    for b in 0..4 {
        for a in 0..4 {
            let q = vertex(p, j - 1 + b as i32, i - 1 + a as i32);
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
        let d = ((q[0] - tx) * fw).powi(2) + ((q[1] - ty) * fh).powi(2);
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
    if p.pts.len() != ((p.rows + 1) * (p.columns + 1)) as usize || p.is_flat() {
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

    /// The crossings are dragged in the Viewer and shown as one Distortion
    /// mesh row, as in After Effects, rather than as a row each.
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

    fn flat(rows: u32, columns: u32) -> MeshWarpParams {
        let mut pts = Vec::new();
        for r in 0..=rows {
            for c in 0..=columns {
                let (x, y) = rest(rows, columns, r, c);
                pts.push([(x / 100.0) as f32, (y / 100.0) as f32]);
            }
        }
        MeshWarpParams {
            rows,
            columns,
            pts,
            steps: 8,
            mix: 1.0,
            frame: [0.0; 4],
        }
    }

    #[test]
    fn a_flat_grid_is_the_layer_itself() {
        let p = flat(3, 4);
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
    fn the_surface_passes_through_every_crossing() {
        let mut p = flat(2, 2);
        p.pts[4] = [0.6, 0.4];
        let (s, _, _) = surface(&p, 0.5, 0.5);
        assert!((s[0] - 0.6).abs() < 1e-5 && (s[1] - 0.4).abs() < 1e-5);
        assert!(!p.is_flat());
    }

    #[test]
    fn a_pixel_finds_where_it_came_from() {
        let mut p = flat(2, 2);
        p.pts[4] = [0.65, 0.5];
        let (s, _, _) = surface(&p, 0.5, 0.5);
        let (u, v) = source_of(&p, s[0], s[1], 100.0, 100.0).unwrap();
        assert!((u - 0.5).abs() < 1e-3 && (v - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_new_grid_size_names_new_rows() {
        let a = rows_for(2, 3);
        assert_eq!(a.len(), 3 * 4 * 2);
        assert_eq!(a[0].id, "mesh_2x3_r0c0_x");
        assert_eq!(a[a.len() - 1].id, "mesh_2x3_r2c3_y");
        assert!(rows_for(3, 3)
            .iter()
            .all(|r| !a.iter().any(|b| b.id == r.id)));
        assert_eq!(clamp_grid(0, 99), (1, MAX_CELLS));
    }
}
