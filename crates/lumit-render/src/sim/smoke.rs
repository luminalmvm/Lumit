//! Smoke and fire's walk: the same kind of walk as the particles', beside them in
//! the card's store. A frame is the walk from the layer's in point, kept every
//! [`KEEP_EVERY`] frames under the same ledger, and baked to the same folder. A
//! kept or baked state is only ever a shortcut.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use glam::{Mat4, Vec3};
use lumit_budget::{Reservation, Tier};
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::drivers::NO_CLONE;
use lumit_core::fx::effects::smoke::{self, Grid, Smoke};
use lumit_core::fx::points::stepped::sim::Ledger;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::{Document, Layer, LayerKind};
use lumit_gpu::fx::smoke::{EmitKind, SmokeEmit, SmokeSolid, SmokeStep, SmokeWork};
use lumit_gpu::fx::volumetric::{VolumeLook, VolumeOp};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;
use uuid::Uuid;

use super::{
    bake, colliders::space, distances, hand_over, state_name, with_store, Kept, Source, KEEP_EVERY,
    WALK_BYTES,
};
use crate::track::{AnalysisError, Progress};

/// The most layers and models the gas goes round.
const MAX_SOLIDS: usize = 32;

/// A kind the kernel emits nothing from: a row that names no layer.
const EMITS_NOTHING: u32 = 3;

/// One box of smoke in a frame's draw list: which effect it is, what time of its
/// walk this frame shows, and how it looks.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone)]
pub(crate) struct VolumeDraw {
    pub doc: Arc<Document>,
    pub comp: Uuid,
    pub layer: Uuid,
    pub effect: Uuid,
    /// The walk's time, seconds from the in point, and the comp's.
    pub t: f64,
    pub tc: f64,
    pub look: VolumeLook,
    /// `look`'s thickness and flame across the box's longest side, as a file's
    /// box is only known once its frame is read.
    pub thick_across: f32,
    pub flame_across: f32,
}

/// A walk's working buffers and how far they have got.
pub(super) struct Live {
    name: [u8; 32],
    work: Arc<SmokeWork>,
    ledger: Ledger,
    /// The models it reads, with where each one's distances start in `work`.
    models: Vec<(Uuid, Arc<distances::Distances>, u32)>,
    _held: Option<Reservation>,
}

fn context(src: &Source<'_>, tc: f64) -> Arc<ExpressionContext> {
    Arc::new(ExpressionContext {
        document: Arc::clone(src.doc),
        comp: Some(src.comp.id),
        layer: Some(src.layer.id),
        comp_time: tc,
        current_depth: 0,
        inputs: None,
    })
}

/// The rows at layer time `lt`.
fn rows_at(src: &Source<'_>, lt: f64) -> Option<Smoke> {
    let tc = lt + src.layer.start_offset.0.to_f64();
    let diag = f64::from(src.comp.width).hypot(f64::from(src.comp.height)) as f32;
    let audio = crate::audio_tap::DocumentAudio::new(src.doc, src.comp, tc);
    let drivers = lumit_core::fx::drivers::resolve_drivers(
        &src.layer.graph,
        lt,
        context(src, tc),
        Some(&audio),
    );
    let stack = lumit_core::fx::resolved::resolve_instance(
        src.inst,
        &drivers,
        lt,
        diag,
        1.0,
        &lumit_core::fx::markers::MarkerContext::NONE,
        context(src, tc),
    );
    Some(Smoke::read(stack.get(0)?.params))
}

/// The box's nought to one to comp pixels at comp time `tc`.
fn box_place(src: &Source<'_>, tc: f64) -> Mat4 {
    let size = smoke::box_size(src.doc, src.comp, src.layer);
    let world = crate::placement::world_placement(src.comp, src.layer, tc, context(src, tc));
    Mat4::from_cols_array_2d(&world)
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -size[2] * 0.5))
        * Mat4::from_scale(Vec3::from(size))
}

fn rows_of(m: Mat4) -> [[f32; 4]; 3] {
    let r = m.transpose().to_cols_array_2d();
    [r[0], r[1], r[2]]
}

/// How far in from a walked box's sides its gas thins out, a share of the box.
const EDGE_FADE: f32 = 0.12;

/// The boxes of smoke `comp` shows at comp time `t`.
pub(crate) fn volumes(
    doc: &Arc<Document>,
    comp: &lumit_core::model::Composition,
    t: f64,
) -> Vec<VolumeDraw> {
    comp.layers
        .iter()
        .filter(|l| {
            l.switches.visible
                && t >= l.in_point.0.to_f64()
                && t < l.out_point.0.to_f64()
                && smoke::on_layer(l).is_some()
                && (!matches!(l.kind, LayerKind::Model { .. })
                    || super::volume_file_path(doc, l).is_some())
        })
        .filter_map(|l| {
            let inst = smoke::on_layer(l)?;
            let src = Source::find(doc, comp.id, l.id, inst.id, NO_CLONE)?;
            let lt = t - l.start_offset.0.to_f64();
            let rows = rows_at(&src, lt)?;
            let place = box_place(&src, t);
            let size = [place.x_axis, place.y_axis, place.z_axis].map(|a| a.truncate().length());
            let longest = size[0].max(size[1]).max(size[2]);
            let opacity = l.transform.opacity.value_at(lt) as f32 / 100.0;
            let mut look = smoke::look(&rows, longest, opacity);
            // A file's density is the file's, and this row thins or thickens it.
            if super::volume_file_path(doc, l).is_some() {
                look.thickness *= rows.density.max(0.0) / 100.0;
            }
            let corner = |c: glam::Vec4| c.truncate().to_array();
            Some(VolumeDraw {
                doc: Arc::clone(doc),
                comp: comp.id,
                layer: l.id,
                effect: inst.id,
                t: l.source_time_at(lt),
                tc: t,
                thick_across: look.thickness * longest,
                flame_across: look.fire_gain * longest,
                look: VolumeLook {
                    from_box: [
                        corner(place.x_axis),
                        corner(place.y_axis),
                        corner(place.z_axis),
                        corner(place.w_axis),
                    ],
                    thickness: look.thickness,
                    smoke: look.smoke,
                    shadow: look.shadow,
                    fire_low: look.fire_low,
                    fire_high: look.fire_high,
                    fire_gain: look.fire_gain,
                    fire_start: smoke::FLAME_STARTS,
                    fire_glow: look.fire_glow,
                    detail: look.detail,
                    clock: l.source_time_at(lt) as f32,
                    fade: EDGE_FADE,
                },
            })
        })
        .collect()
}

/// A step's numbers from the rows at its time.
fn step_of(rows: &Smoke, grid: &Grid, kind: u32, h: f32, clock: f32) -> SmokeStep {
    let p = smoke::physics(rows, grid);
    let [nx, ny, nz] = grid.cells.map(|c| c as f32);
    let m = p.patch_middle;
    let r = p.patch_reach;
    SmokeStep {
        cells: [nx, ny, nz, h],
        wind: [p.wind[0], p.wind[1], p.wind[2], p.settle],
        emit: [p.emit[0], p.emit[1], p.emit[2], kind as f32],
        emit_middle: [m[0], m[1], m[2], clock],
        emit_reach: [r[0], r[1], r[2], p.kick],
        motion: [p.push, p.pull, p.curl, p.thin],
        burn: [p.burn_rate, p.burn_heat, p.burn_soot, p.cool],
        fire: [p.ignite, p.swell, 1.5, 0.0],
        to_model: [[0.0; 4]; 3],
        model_cells: [1.0, 1.0, 1.0, 0.0],
        extra: [p.turbulence, p.patch, 0.0, 0.0],
        more: [p.radiate, p.tongues, 0.0, 0.0],
        drift: [0.0; 4],
    }
}

/// What a walk is emitting from, found by its rows.
enum Emits<'a> {
    Patch,
    Nothing,
    Particles(&'a Layer, Uuid),
    Model(&'a Layer, uuid::Uuid),
}

fn emits<'a>(src: &Source<'a>) -> Emits<'a> {
    use lumit_core::model::EffectValue;
    let source = smoke::emit_layer(src.comp, src.inst);
    match src.inst.param("emit_from") {
        Some(EffectValue::Choice(1)) => source
            .and_then(|l| {
                let e = l
                    .effects
                    .iter()
                    .find(|e| e.enabled && e.effect.match_name == "particulate")?;
                Some(Emits::Particles(l, e.id))
            })
            .unwrap_or(Emits::Nothing),
        Some(EffectValue::Choice(2)) => match source.map(|l| (l, &l.kind)) {
            Some((l, LayerKind::Model { item, .. })) => Emits::Model(l, *item),
            _ => Emits::Nothing,
        },
        _ => Emits::Patch,
    }
}

/// Working buffers for a walk with nothing in them, reserved first. `None`, with
/// the Bake line saying why, when the card can't hold them even with its kept
/// states gone. A walk then draws no smoke, and never crashes.
pub(super) fn fresh(fx: &FxEngine, ctx: &GpuContext, src: &Source<'_>) -> Option<Live> {
    let grid = smoke::grid(src.doc, src.comp, src.layer, src.inst);
    let what = emits(src);
    // Every model the walk reads, one table on the card. One not read yet is a
    // walk not started, which the next frame tries again.
    let mut items: Vec<Uuid> = Vec::new();
    if let Emits::Model(_, item) = &what {
        items.push(*item);
    }
    let hits: Vec<&Layer> = smoke::solid_layers(src.comp, src.inst, src.layer.id)
        .take(MAX_SOLIDS)
        .collect();
    for l in &hits {
        if let LayerKind::Model { item, .. } = &l.kind {
            if !items.contains(item) {
                items.push(*item);
            }
        }
    }
    let mut models = Vec::with_capacity(items.len());
    let mut table = Vec::new();
    for item in items {
        let d = distances::of(src.doc, item)?;
        models.push((item, Arc::clone(&d), table.len() as u32));
        table.extend_from_slice(&d.values);
    }
    let bytes = lumit_gpu::fx::smoke::work_bytes(grid.cells);
    let device_most = u64::from(ctx.device.limits().max_storage_buffer_binding_size)
        .min(ctx.device.limits().max_buffer_size);
    let fits = lumit_gpu::fx::smoke::state_bytes(grid.cells) <= device_most;
    let held = fits
        .then(|| {
            ctx.ledger().reserve(Tier::Vram, bytes).ok().or_else(|| {
                super::clear_kept();
                ctx.ledger().reserve(Tier::Vram, bytes).ok()
            })
        })
        .flatten();
    let Some(held) = held else {
        refused(src.inst.id);
        return None;
    };
    let work = fx.smoke_work(
        ctx,
        grid.cells,
        &SmokeEmit {
            distances: Some(table.as_slice()),
            particles: matches!(what, Emits::Particles(..)),
            solids: !hits.is_empty(),
        },
    );
    fx.smoke_clear(ctx, &work);
    accepted(src.inst.id);
    Some(Live {
        name: smoke::definition(src.doc, src.comp, src.layer, src.inst),
        work: Arc::new(work),
        ledger: Ledger {
            frame: i64::MAX,
            ..Ledger::default()
        },
        models,
        _held: Some(held),
    })
}

/// Says on the Bake line that this grid is too much for the card.
fn refused(effect: Uuid) {
    if !matches!(
        crate::track::progress(effect),
        Some(Progress::Failed(AnalysisError::TooBig))
    ) {
        crate::track::finish(effect, Some(Progress::Failed(AnalysisError::TooBig)));
    }
}

/// Takes that back once a grid fits, a smaller one or a card with room again.
fn accepted(effect: Uuid) {
    if matches!(
        crate::track::progress(effect),
        Some(Progress::Failed(AnalysisError::TooBig))
    ) {
        crate::track::finish(effect, None);
    }
}

/// The grid's cells to comp pixels at comp time `tc`.
fn cells_place(src: &Source<'_>, live: &Live, tc: f64) -> Mat4 {
    let cells = Vec3::from(live.work.cells.map(|c| c as f32));
    box_place(src, tc) * Mat4::from_scale(Vec3::ONE / cells.max(Vec3::ONE))
}

/// A model's cells of distances to comp pixels at comp time `tc`.
fn model_place(src: &Source<'_>, layer: &Layer, d: &distances::Distances, tc: f64) -> Option<Mat4> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return None;
    };
    let scene = crate::model::scene(src.doc, *item)?;
    let size = (src.comp.width as f32, src.comp.height as f32);
    let own = Arc::new(ExpressionContext {
        layer: Some(layer.id),
        ..(*context(src, tc)).clone()
    });
    let world = crate::placement::world_placement(src.comp, layer, tc, own);
    Some(
        Mat4::from_cols_array_2d(&world)
            * Mat4::from_cols_array_2d(&crate::model::model_to_comp(settings, &scene.bounds, size))
            * d.cell_to_model,
    )
}

/// What the frame after `frame` emits: particles laid in the grid, and the
/// model's cells in a step's own numbers. `None` when the frame is no longer
/// wanted, which must stop the walk here: a step on what was laid for another
/// frame would put wrong gas in every frame after.
fn prepare_emit(
    fx: &FxEngine,
    ctx: &GpuContext,
    src: &Source<'_>,
    live: &Live,
    frame: i64,
    step: &mut SmokeStep,
) -> Option<()> {
    let tc = (frame + 1) as f64 * src.dt + src.layer.start_offset.0.to_f64();
    let cells_to_comp = cells_place(src, live, tc);
    match emits(src) {
        Emits::Patch => {}
        Emits::Nothing => step.emit[3] = EMITS_NOTHING as f32,
        Emits::Particles(layer, effect) => {
            step.emit[3] = EmitKind::Particles as u32 as f32;
            let from = Source::find(src.doc, src.comp.id, layer.id, effect, NO_CLONE)?;
            let carried =
                crate::trails::carriage(src.doc, src.comp, layer, from.inst, NO_CLONE, [0.0; 2]);
            let lt = tc - layer.start_offset.0.to_f64();
            // Before that layer's in point there is nothing to lay.
            if lt < 0.0 {
                step.emit[3] = EMITS_NOTHING as f32;
                return Some(());
            }
            let parts = super::walked(fx, ctx, &from, &super::walk_name(&carried), lt)?;
            let room = space(&from);
            let world = crate::placement::world_placement(src.comp, room, tc, context(&from, tc));
            let to_cells = cells_to_comp.inverse() * Mat4::from_cols_array_2d(&world);
            fx.smoke_splat(ctx, &live.work, &parts.state, parts.cap, rows_of(to_cells));
        }
        Emits::Model(layer, item) => {
            let (_, d, at) = live.models.iter().find(|m| m.0 == item)?;
            let cell_to_comp = model_place(src, layer, d, tc)?;
            step.emit[3] = EmitKind::Model as u32 as f32;
            step.to_model = rows_of(cell_to_comp.inverse() * cells_to_comp);
            let [x, y, z] = d.cells.map(|c| c as f32);
            step.model_cells = [x, y, z, *at as f32];
        }
    }
    Some(())
}

/// What the gas goes round in the frame ending at comp time `tc`.
fn solids_at(src: &Source<'_>, live: &Live, tc: f64, grid: &Grid) -> Vec<SmokeSolid> {
    use lumit_core::fx::points::stepped::collider::Collider;
    let then = tc - src.dt;
    let (now_cells, was_cells) = (cells_place(src, live, tc), cells_place(src, live, then));
    let was_cells_inv = was_cells.inverse();
    smoke::solid_layers(src.comp, src.inst, src.layer.id)
        .take(MAX_SOLIDS)
        .filter_map(|l| {
            let (now, was, lo, hi, cells, kind) = if let LayerKind::Model { item, .. } = &l.kind {
                let (_, d, at) = live.models.iter().find(|m| m.0 == *item)?;
                let [x, y, z] = d.cells;
                (
                    model_place(src, l, d, tc)?,
                    model_place(src, l, d, then)?,
                    [0.0; 4],
                    [0.0; 4],
                    [*at, x, y, z],
                    1,
                )
            } else {
                let at = |t: f64| {
                    let w = crate::placement::world_placement(src.comp, l, t, context(src, t));
                    Mat4::from_cols_array_2d(&w)
                };
                let c = Collider::of_in_3d(src.doc, src.comp, src.layer, l, tc, &context(src, tc));
                if !c.is_surface() || c.edges().is_empty() {
                    return None;
                }
                let mut lo = [f32::INFINITY; 2];
                let mut hi = [f32::NEG_INFINITY; 2];
                for p in c.edges().iter().flatten() {
                    for k in 0..2 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                let now = at(tc);
                // A card has no thickness, so it is a slab a cell or two thick.
                let thick = 1.5 * grid.cell_px / now.z_axis.truncate().length().max(1e-3);
                (
                    now,
                    at(then),
                    [lo[0], lo[1], -thick, 0.0],
                    [hi[0], hi[1], thick, 0.0],
                    [0; 4],
                    0,
                )
            };
            Some(SmokeSolid {
                from_cell: rows_of(now.inverse() * now_cells),
                was: rows_of(was_cells_inv * was * now.inverse() * now_cells),
                lo,
                hi,
                grid: cells,
                kind: [kind, 0, 0, 0],
            })
        })
        .collect()
}

/// One whole frame on from `live`'s.
fn frame_on(fx: &FxEngine, ctx: &GpuContext, src: &Source<'_>, live: &mut Live) -> Option<()> {
    let f = live.ledger.frame;
    let lt = (f + 1) as f64 * src.dt;
    let rows = rows_at(src, lt)?;
    let grid = smoke::grid(src.doc, src.comp, src.layer, src.inst);
    let p = smoke::physics(&rows, &grid);
    let h = src.dt / f64::from(p.steps);
    let mut first = step_of(&rows, &grid, p.emit_kind, h as f32, 0.0);
    prepare_emit(fx, ctx, src, live, f, &mut first)?;
    let tc = lt + src.layer.start_offset.0.to_f64();
    let solids = solids_at(src, live, tc, &grid);
    if rows.trail {
        // Where the box's middle was a frame ago, in today's cells: the gas stays
        // put in the comp while the box moves through it.
        let middle = Vec3::from(live.work.cells.map(|c| c as f32)) * 0.5;
        let was = cells_place(src, live, tc).inverse() * cells_place(src, live, tc - src.dt);
        let by = (was.transform_point3(middle) - middle) / src.dt as f32;
        first.drift = [by.x, by.y, by.z, 0.0];
    }
    first.extra[2] = solids.len() as f32;
    first.extra[3] = src.dt as f32;
    if !solids.is_empty() {
        fx.smoke_solids(ctx, &live.work, &solids, &first);
    }
    let steps: Vec<SmokeStep> = (0..p.steps)
        .map(|k| {
            let mut s = first;
            s.emit_middle[3] = (f as f64 * src.dt + f64::from(k) * h) as f32;
            s
        })
        .collect();
    fx.smoke_steps(ctx, &live.work, &steps);
    live.ledger.frame = f + 1;
    Some(())
}

fn take_live(fx: &FxEngine, ctx: &GpuContext, src: &Source<'_>, name: &[u8; 32]) -> Option<Live> {
    let found = with_store(ctx, |s| {
        s.smoke
            .iter()
            .position(|l| l.name == *name)
            .map(|at| s.smoke.remove(at))
    })?;
    if found.is_some() {
        return found;
    }
    fresh(fx, ctx, src)
}

/// `live` back in the store, with the walks used longest ago let go once they
/// hold too much.
fn give_back(ctx: &GpuContext, live: Live) {
    let _ = with_store(ctx, |s| {
        s.smoke.insert(0, live);
        let mut bytes = 0;
        let mut first = true;
        s.smoke.retain(|l| {
            bytes += l.work.bytes();
            let stays = first || bytes <= WALK_BYTES;
            first = false;
            stays
        });
    });
}

/// Puts a kept or baked state back if one is nearer than where `live` is, or
/// empties it to start from the in point.
fn start_from(fx: &FxEngine, ctx: &GpuContext, src: &Source<'_>, live: &mut Live, target: i64) {
    let here = Some(live.ledger.frame).filter(|f| *f <= target);
    let kept = super::furthest_kept(ctx, &live.name, target, 0)
        .filter(|k| here.is_none_or(|f| k.ledger.frame > f));
    if let Some(kept) = kept {
        fx.smoke_restore(ctx, &live.work, &kept.buffer);
        live.ledger = kept.ledger;
    }
    let reached = Some(live.ledger.frame).filter(|f| *f <= target);
    if reached.is_none_or(|r| target - r > KEEP_EVERY) {
        if let Some(baked) = bake::nearest(src.inst.id, &live.name, target, reached) {
            if fx.smoke_write(ctx, &live.work, &baked.bytes) {
                live.ledger = baked.ledger;
                return;
            }
        }
    }
    if reached.is_none() {
        fx.smoke_clear(ctx, &live.work);
        live.ledger = Ledger::default();
    }
}

/// Grid work between waits for the card: particle steps, as `hand_over` counts.
fn step_work(live: &Live, steps: u32) -> u64 {
    lumit_gpu::fx::smoke::cell_count(live.work.cells) * u64::from(steps) * 8
}

/// Keeps where `live` has got to, unless the card is short: a walk then only goes
/// slower, and its frames are the same.
fn keep(fx: &FxEngine, ctx: &GpuContext, live: &Live) {
    if !matches!(ctx.vram_pressure(), lumit_budget::Pressure::Easy) {
        return;
    }
    let buffer = fx.smoke_keep(ctx, &live.work);
    let at = state_name(&live.name, live.ledger.frame);
    let ledger = live.ledger;
    let _ = with_store(ctx, |s| s.kept.insert(at, Kept { buffer, ledger }));
}

/// The gas at walk time `t`. `None` before the in point, when it can't fit, when
/// the frame is no longer wanted, or with Simulate off and nothing to show.
fn walked(fx: &FxEngine, ctx: &GpuContext, src: &Source<'_>, t: f64) -> Option<Arc<SmokeWork>> {
    if t.is_nan() || t < 0.0 {
        return None;
    }
    let target = (t / src.dt + 1e-6).floor() as i64;
    let name = smoke::definition(src.doc, src.comp, src.layer, src.inst);
    let mut live = take_live(fx, ctx, src, &name)?;
    start_from(fx, ctx, src, &mut live, target);
    let simulates = rows_at(src, t)?.simulate;
    let mut work = 0u64;
    let (began, from) = (web_time::Instant::now(), live.ledger.frame);
    while simulates && live.ledger.frame < target {
        if began.elapsed().as_millis() >= super::SAY_WALKING_AFTER_MS {
            let done = (live.ledger.frame - from) as f32;
            crate::profile::report_walk(done / (target - from).max(1) as f32);
        }
        if crate::cancel::requested() {
            crate::cancel::cut_short();
            give_back(ctx, live);
            return None;
        }
        if frame_on(fx, ctx, src, &mut live).is_none() {
            give_back(ctx, live);
            return None;
        }
        if live.ledger.frame % KEEP_EVERY == 0 {
            keep(fx, ctx, &live);
        }
        if live.ledger.frame < target {
            hand_over(ctx, &mut work, step_work(&live, 1));
        }
    }
    // Simulate off, with no state at or before this frame, is nothing to show.
    if !simulates && live.ledger.frame == 0 {
        give_back(ctx, live);
        return None;
    }
    let out = Arc::clone(&live.work);
    fx.smoke_draw(ctx, &out);
    give_back(ctx, live);
    Some(out)
}

/// A volume file's frame as the marcher reads it: its field, in its own box.
#[cfg(not(target_arch = "wasm32"))]
fn file_volume(
    ctx: &GpuContext,
    src: &Source<'_>,
    d: &VolumeDraw,
    path: &std::path::Path,
) -> Option<VolumeOp> {
    let LayerKind::Model { item, settings } = &src.layer.kind else {
        return None;
    };
    let frame = (d.t / src.dt + 1e-6).floor() as i64;
    let Ok((fields, bounds)) = super::volume_file::field(ctx, path, frame)? else {
        refused(src.inst.id);
        return None;
    };
    accepted(src.inst.id);
    let scene = crate::model::scene(src.doc, *item)?;
    let tc = d.tc;
    let world = crate::placement::world_placement(src.comp, src.layer, tc, context(src, tc));
    let size = (src.comp.width as f32, src.comp.height as f32);
    let (lo, hi) = (Vec3::from(bounds.min), Vec3::from(bounds.max));
    let place = Mat4::from_cols_array_2d(&world)
        * Mat4::from_cols_array_2d(&crate::model::model_to_comp(settings, &scene.bounds, size))
        * Mat4::from_translation(lo)
        * Mat4::from_scale((hi - lo).max(Vec3::splat(1e-6)));
    let longest = [place.x_axis, place.y_axis, place.z_axis]
        .map(|a| a.truncate().length())
        .into_iter()
        .fold(0.0, f32::max);
    let corner = |c: glam::Vec4| c.truncate().to_array();
    let mut look = d.look;
    look.fade = 0.0;
    look.from_box = [
        corner(place.x_axis),
        corner(place.y_axis),
        corner(place.z_axis),
        corner(place.w_axis),
    ];
    // The file's own density, as thick across as the rows say a simulated one is.
    look.thickness = d.thick_across / longest.max(1.0);
    look.fire_gain = d.flame_across / longest.max(1.0);
    Some(VolumeOp { fields, look })
}

/// The boxes of `draws` as the marcher reads them, each walked to its frame.
pub(crate) fn walked_volumes(
    fx: &FxEngine,
    ctx: &GpuContext,
    draws: &[VolumeDraw],
) -> Vec<VolumeOp> {
    draws
        .iter()
        .filter_map(|d| {
            let src = Source::find(&d.doc, d.comp, d.layer, d.effect, NO_CLONE)?;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = super::volume_file::path_of(&d.doc, src.layer) {
                return file_volume(ctx, &src, d, &path);
            }
            let work = walked(fx, ctx, &src, d.t)?;
            Some(VolumeOp {
                fields: work.fields().clone(),
                look: d.look,
            })
        })
        .collect()
}

/// A bake's walk: the frames the layer shows, from the start, with a state
/// written every so often.
pub(super) fn bake_walk(
    doc: &Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    effect: Uuid,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(), AnalysisError> {
    let src = Source::find(doc, comp, layer, effect, NO_CLONE).ok_or(AnalysisError::Unreadable)?;
    let (first, end) = bake::frames_shown(&src);
    // A volume file has nothing to walk.
    if end <= first || super::volume_file_path(doc, src.layer).is_some() {
        return Err(AnalysisError::NoFrames);
    }
    let total = (end - first) as usize;
    report(Progress::Tracking { done: 0, total });
    let ctx = GpuContext::headless().map_err(|_| AnalysisError::Unreadable)?;
    let fx = FxEngine::new(&ctx);
    let mut live = fresh(&fx, &ctx, &src).ok_or(AnalysisError::TooBig)?;
    let name = live.name;
    let dir = bake::start_dir(effect, &name).ok_or(AnalysisError::Unreadable)?;
    fx.smoke_clear(&ctx, &live.work);
    live.ledger = Ledger::default();
    let mut writer = bake::Writer::default();
    let mut work = 0u64;
    while live.ledger.frame < end {
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        report(Progress::Tracking {
            done: (live.ledger.frame - first).max(0) as usize,
            total,
        });
        frame_on(&fx, &ctx, &src, &mut live).ok_or(AnalysisError::Unreadable)?;
        hand_over(&ctx, &mut work, step_work(&live, 1));
        let frame = live.ledger.frame;
        if frame % KEEP_EVERY == 0 && frame + KEEP_EVERY > first {
            let bytes = fx
                .smoke_read(&ctx, &live.work)
                .ok_or(AnalysisError::Unreadable)?;
            if writer.write(&dir, &live.ledger, &bytes)? {
                break;
            }
        }
    }
    report(Progress::Tracking { done: total, total });
    Ok(())
}

/// Whether a grid for `effect` fits on `ctx`'s card, as a walk finds out, with the
/// Bake line saying so if not.
#[must_use]
pub fn fits(
    fx: &FxEngine,
    ctx: &GpuContext,
    doc: &Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    effect: Uuid,
) -> bool {
    Source::find(doc, comp, layer, effect, NO_CLONE)
        .is_some_and(|src| fresh(fx, ctx, &src).is_some())
}

/// How long one step and one march of `effect`'s box take on the card, in
/// milliseconds, each the fastest of `runs` after `warm` frames to fill it. `None`
/// with no card or no room.
#[must_use]
pub fn timed(
    doc: &Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    effect: Uuid,
    warm: i64,
    runs: i64,
) -> Option<(f64, f64)> {
    use std::time::Instant;
    let src = Source::find(doc, comp, layer, effect, NO_CLONE)?;
    let ctx = GpuContext::headless().ok()?;
    let fx = FxEngine::new(&ctx);
    let mut live = fresh(&fx, &ctx, &src)?;
    live.ledger.frame = 0;
    let mut step = f64::INFINITY;
    for k in 0..warm + runs {
        let begun = Instant::now();
        frame_on(&fx, &ctx, &src, &mut live)?;
        ctx.flush();
        ctx.device.poll(wgpu::Maintain::Wait);
        if k >= warm {
            step = step.min(begun.elapsed().as_secs_f64() * 1e3);
        }
    }
    fx.smoke_draw(&ctx, &live.work);
    let draw = volumes(doc, src.comp, (warm + runs) as f64 * src.dt)
        .into_iter()
        .find(|d| d.effect == effect)?;
    let (w, h) = (src.comp.width, src.comp.height);
    let pose = lumit_core::camera::default_pose(f64::from(w), f64::from(h));
    let [right, up, forward] = lumit_core::camera::axes(pose.rotation_deg);
    let v = |p: (f64, f64, f64)| [p.0 as f32, p.1 as f32, p.2 as f32];
    let op = lumit_gpu::fx::volumetric::HazeOp {
        eye: v(pose.position),
        right: v(right),
        up: v(up),
        forward: v(forward),
        zoom: pose.zoom as f32,
        comp: (w as f32, h as f32),
        scale: 1.0,
        near: 0.0,
        centre: [0.0; 3],
        radius: 1.0,
        lights: Vec::new(),
        volumes: vec![VolumeOp {
            fields: live.work.fields().clone(),
            look: draw.look,
        }],
        no_shadow: None,
    };
    let depth = fx.far_depth(&ctx, w, h);
    let mut march = f64::INFINITY;
    for _ in 0..runs {
        let begun = Instant::now();
        let _ = fx.haze_scatter(&ctx, &depth, &op, None, None);
        ctx.flush();
        ctx.device.poll(wgpu::Maintain::Wait);
        march = march.min(begun.elapsed().as_secs_f64() * 1e3);
    }
    Some((step, march))
}
