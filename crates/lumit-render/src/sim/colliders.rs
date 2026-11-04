//! What a simulation's particles hit at one time, flattened for the card.

use std::sync::Arc;

use glam::Mat4;
use lumit_core::camera::Place;
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::effects::particulate::Particulate;
use lumit_core::fx::points::stepped::collider::Collider;
use lumit_core::model::{EffectValue, Layer, LayerKind};
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
    /// Each model's distances, and where they start in `buffer`.
    list: Vec<(uuid::Uuid, Arc<Distances>, u32)>,
    buffer: Option<wgpu::Buffer>,
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
fn space<'a>(src: &Source<'a>) -> &'a Layer {
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
    ) -> Option<(Arc<Distances>, u32)> {
        if let Some((_, d, at)) = self.list.iter().find(|m| m.0 == item) {
            return Some((Arc::clone(d), *at));
        }
        let d = distances::of(doc, item)?;
        let at = self
            .list
            .last()
            .map_or(0, |(_, d, at)| at + d.values.len() as u32);
        self.list.push((item, Arc::clone(&d), at));
        let all: Vec<f32> = self
            .list
            .iter()
            .flat_map(|(_, d, _)| d.values.iter().copied())
            .collect();
        self.buffer = Some(lumit_gpu::fx::particulate_sim::distances_buffer(ctx, &all));
        Some((d, at))
    }

    /// Everything the particles hit at comp time `tc`, and every edge they read.
    pub(super) fn at(
        &mut self,
        ctx: &GpuContext,
        src: &Source<'_>,
        tc: f64,
        context: &Arc<ExpressionContext>,
        rows_now: &Particulate,
    ) -> (Vec<SimCollider>, Vec<[f32; 4]>) {
        let (doc, comp) = (src.doc.as_ref(), src.comp);
        let owner = space(src);
        let mut out = Vec::new();
        let mut edges = Vec::new();
        let hits = lumit_core::fx::points::stepped::sim::hit_layers(comp, src.inst, src.layer.id)
            .filter(|l| l.id != owner.id);
        for target in hits.take(MAX_COLLIDERS) {
            let c = Collider::of_in_3d(doc, comp, owner, target, tc, context);
            if let LayerKind::Model { item, settings } = &target.kind {
                let Some(scene) = crate::model::scene(doc, *item) else {
                    continue;
                };
                let Some((d, first)) = self.add(ctx, doc, *item) else {
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
                out.push(SimCollider {
                    from: mat_rows(&to_layer.inverse()),
                    to: mat_rows(&to_layer),
                    lo: [0.0; 2],
                    hi: [0.0; 2],
                    kind: ColliderKind::Model as u32,
                    first: 0,
                    n: 0,
                    _pad: 0,
                    grid: [first, nx, ny, nz],
                });
                continue;
            }
            out.push(flat(&c, &mut edges));
        }
        if rows_now.ground && rows_now.ground_height.is_finite() {
            let floor = f64::from(comp.height) * 0.5 - f64::from(rows_now.ground_height);
            let c = Collider::ground(comp, owner, tc, context, floor);
            out.push(SimCollider {
                kind: ColliderKind::Floor as u32,
                ..flat(&c, &mut edges)
            });
        }
        (out, edges)
    }
}

/// A card or walls, its edges put with the rest.
fn flat(c: &Collider, edges: &mut Vec<[f32; 4]>) -> SimCollider {
    let first = edges.len() as u32;
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    let room = MAX_EDGES.saturating_sub(edges.len());
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
    SimCollider {
        from: rows(c.from_layer()),
        to: rows(c.to_layer()),
        lo,
        hi,
        kind: kind as u32,
        first,
        n,
        _pad: 0,
        grid: [0; 4],
    }
}
