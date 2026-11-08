//! What a simulation's particles hit at one time, flattened for the card.

use std::sync::Arc;

use glam::Mat4;
use lumit_core::camera::Place;
use lumit_core::fx::effects::particulate::Particulate;
use lumit_core::fx::points::stepped::collider::Collider;
use lumit_core::model::{EffectValue, Layer, LayerKind};
use lumit_core::model_layer::ModelSettings;
use lumit_gpu::fx::particulate_sim::{ColliderKind, SimCollider};
use lumit_gpu::GpuContext;

use super::distances::{self, Distances};
use super::Source;

/// The most layers a simulation hits.
const MAX_COLLIDERS: usize = 32;

/// The most edges all its colliders' shapes hold between them.
const MAX_EDGES: usize = 1 << 14;

/// The models a walk hits, with their distances once on the card.
#[derive(Default)]
pub(super) struct Models {
    /// Each model and take's distances, and where they start in `buffer`.
    list: Vec<(uuid::Uuid, Option<String>, Arc<Distances>, u32)>,
    buffer: Option<wgpu::Buffer>,
    /// What was hit at the last time asked, and when, which is where the next
    /// frame's sweep starts.
    last: Option<(f64, Vec<SimCollider>)>,
    /// The surface a Layer emitter gives birth on, gathered once a walk.
    surface: Option<Option<Arc<[[f32; 4]]>>>,
}

fn rows(p: &Place) -> [[f32; 4]; 3] {
    std::array::from_fn(|r| {
        let m = p.m[r];
        let t = [p.t.0, p.t.1, p.t.2][r];
        [m[0] as f32, m[1] as f32, m[2] as f32, t as f32]
    })
}

fn mat_rows(m: &Mat4) -> [[f32; 4]; 3] {
    let r = m.transpose().to_cols_array_2d();
    [r[0], r[1], r[2]]
}

fn place_mat(p: &Place) -> Mat4 {
    let [a, b, c] = rows(p);
    Mat4::from_cols_array_2d(&[a, b, c, [0.0, 0.0, 0.0, 1.0]]).transpose()
}

/// The layer the particles' pixels are in: the Model layer a Layer emitter names,
/// when it has them born on that model's surface, or their own.
pub(super) fn space<'a>(src: &Source<'a>) -> &'a Layer {
    let named = matches!(src.inst.param("shape"), Some(EffectValue::Choice(9)))
        .then(|| src.inst.layer_ref("emitter_layer"))
        .flatten()
        .and_then(|id| src.comp.layers.iter().find(|l| l.id == id))
        .filter(|l| matches!(l.kind, LayerKind::Model { .. }));
    named
        .filter(|_| lumit_core::fx::effects::particulate::rows::is_scene(src.inst))
        .unwrap_or(src.layer)
}

impl Models {
    pub(super) fn surface(&mut self, src: &Source<'_>) -> Option<Arc<[[f32; 4]]>> {
        self.surface
            .get_or_insert_with(|| crate::realise::scene::surface_of(src.doc, src.comp, src.layer))
            .clone()
    }

    pub(super) fn distances(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// `item`'s distances, put with the rest. They only ever grow by a model the
    /// walk has not met yet, which its name already says it reads.
    fn add(
        &mut self,
        ctx: &GpuContext,
        doc: &lumit_core::model::Document,
        item: uuid::Uuid,
        settings: &ModelSettings,
        scene: &lumit_scene::ModelScene,
    ) -> Option<(Arc<Distances>, u32)> {
        let take = crate::model_motion::clip_of(settings, scene).map(|clip| clip.name.clone());
        if let Some((.., d, at)) = self.list.iter().find(|m| m.0 == item && m.1 == take) {
            return Some((Arc::clone(d), *at));
        }
        let d = distances::of(doc, item, settings)?;
        let at = self
            .list
            .last()
            .map_or(0, |(.., d, at)| at + d.values.len() as u32);
        self.list.push((item, take, Arc::clone(&d), at));
        let all: Vec<f32> = self
            .list
            .iter()
            .flat_map(|(.., d, _)| d.values.iter().copied())
            .collect();
        self.buffer = Some(lumit_gpu::fx::particulate_sim::distances_buffer(ctx, &all));
        Some((d, at))
    }

    /// Everything the particles hit at layer time `lt`, and every edge they read,
    /// each collider's pose at the frame's start the one the frame before ended on.
    pub(super) fn at(
        &mut self,
        ctx: &GpuContext,
        src: &Source<'_>,
        start: f64,
        lt: f64,
        rows_now: &Particulate,
    ) -> (Vec<SimCollider>, Vec<u32>) {
        let offset = src.layer.start_offset.0.to_f64();
        let (mut out, edges) = self.posed(ctx, src, lt + offset, rows_now);
        let was = match self.last.take() {
            Some((at, poses)) if at == start => poses,
            // A walk that picked up here poses the start with the rows at the start.
            _ => {
                let then = src.inputs(start);
                let rows = then.as_ref().map_or(rows_now, |i| &i.rows);
                self.posed(ctx, src, start + offset, rows).0
            }
        };
        // A layer that comes or goes stands still for the frame.
        if was.len() == out.len() {
            for (c, w) in out.iter_mut().zip(&was) {
                c.was_from = w.from;
                c.was_to = w.to;
            }
        }
        self.last = Some((lt, out.clone()));
        (out, edges)
    }

    fn posed(
        &mut self,
        ctx: &GpuContext,
        src: &Source<'_>,
        tc: f64,
        rows_now: &Particulate,
    ) -> (Vec<SimCollider>, Vec<u32>) {
        let context = &src.context(tc);
        let (doc, comp) = (src.doc.as_ref(), src.comp);
        let owner = space(src);
        let mut out = Vec::new();
        let mut edges: Vec<[f32; 4]> = Vec::new();
        let mut grids: Vec<u32> = Vec::new();
        let hits = lumit_core::fx::points::stepped::sim::hit_layers(comp, src.inst, src.layer.id)
            .filter(|l| l.id != owner.id);
        for target in hits.take(MAX_COLLIDERS) {
            let c = Collider::of_in_3d(doc, comp, owner, target, tc, context);
            if let LayerKind::Model { item, settings } = &target.kind {
                let Some(scene) = crate::model::scene(doc, *item) else {
                    continue;
                };
                let Some((d, first)) = self.add(ctx, doc, *item, settings, &scene) else {
                    continue;
                };
                let size = (comp.width as f32, comp.height as f32);
                let to_layer = place_mat(c.to_layer())
                    * Mat4::from_cols_array_2d(&crate::model::model_to_comp(
                        settings,
                        &scene.bounds,
                        size,
                    ))
                    * d.cell_to_model;
                let [nx, ny, nz] = d.cells;
                let (from, to) = (mat_rows(&to_layer.inverse()), mat_rows(&to_layer));
                out.push(SimCollider {
                    from,
                    to,
                    was_from: from,
                    was_to: to,
                    lo: [0.0; 2],
                    hi: [0.0; 2],
                    kind: ColliderKind::Model as u32,
                    first: 0,
                    n: 0,
                    cell: 0.0,
                    grid: [first, nx, ny, nz],
                });
                continue;
            }
            out.push(flat(&c, &mut edges, &mut grids));
        }
        if rows_now.ground && rows_now.ground_height.is_finite() {
            let floor = f64::from(comp.height) * 0.5 - f64::from(rows_now.ground_height);
            let c = Collider::ground(comp, owner, tc, context, floor);
            out.push(SimCollider {
                kind: ColliderKind::Floor as u32,
                ..flat(&c, &mut edges, &mut grids)
            });
        }
        // The grids come after every edge, and say where they are from there.
        let base = edges.len() as u32 * 4;
        for c in out
            .iter_mut()
            .filter(|c| c.kind == ColliderKind::Walls as u32)
        {
            c.grid[0] += base;
            c.grid[1] += base;
        }
        let mut words: Vec<u32> = edges.iter().flatten().map(|v| v.to_bits()).collect();
        words.append(&mut grids);
        (out, words)
    }
}

/// A card or walls, its edges put with the rest, and walls' grid too.
fn flat(c: &Collider, edges: &mut Vec<[f32; 4]>, grids: &mut Vec<u32>) -> SimCollider {
    let first = edges.len() as u32;
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    let room = MAX_EDGES.saturating_sub(edges.len());
    // An edge the room turned away must not stay in a grid.
    let whole = c.edges().len() <= room;
    for [a, b] in c.edges().iter().take(room) {
        edges.push([a[0], a[1], b[0], b[1]]);
        for k in 0..2 {
            lo[k] = lo[k].min(a[k]).min(b[k]);
            hi[k] = hi[k].max(a[k]).max(b[k]);
        }
    }
    let n = edges.len() as u32 - first;
    let square = n == 4
        && edges[first as usize..]
            .iter()
            .all(|e| e[0] == e[2] || e[1] == e[3]);
    let kind = match (c.is_surface(), square) {
        (false, _) => ColliderKind::Walls,
        (true, true) => ColliderKind::Rect,
        (true, false) => ColliderKind::Shapes,
    };
    let (from, to) = (rows(c.from_layer()), rows(c.to_layer()));
    let mut made = SimCollider {
        from,
        to,
        was_from: from,
        was_to: to,
        lo,
        hi,
        kind: kind as u32,
        first,
        n,
        cell: 1.0,
        grid: [0; 4],
    };
    if kind == ColliderKind::Walls && whole {
        let (origin, cell, size, starts, items) = c.grid();
        made.lo = origin;
        made.cell = cell;
        let at = grids.len() as u32;
        made.grid = [at, at + starts.len() as u32, size[0] as u32, size[1] as u32];
        grids.extend_from_slice(starts);
        grids.extend_from_slice(items);
    } else if kind == ColliderKind::Walls {
        made.n = 0;
    }
    made
}
