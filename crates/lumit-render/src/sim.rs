//! Particulate's simulation on the card: a frame is the walk from the layer's
//! in point. Rows, births and what is hit are read at each frame's own time,
//! the particles only ever live on the card, and where the walk got to is kept
//! there every [`KEEP_EVERY`] frames, so playing on is one frame of steps and
//! scrubbing back a few. A kept state is only a shortcut: a frame reached from
//! one is the frame walked from the start, which is what lets an export match
//! the Viewer.

pub mod bake;
mod colliders;
mod distances;

use std::sync::{Arc, Mutex, Weak};

use lumit_budget::{Reservation, Tier};
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::effects::particulate::Particulate as P;
use lumit_core::fx::params::Params;
use lumit_core::fx::points::stepped::sim::{physics, Ledger, Steady};
use lumit_core::fx::points::stepped::Inputs;
use lumit_core::fx::points::trails::Trails;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::{Composition, Document, EffectInstance, Layer};
use lumit_gpu::fx::particulate::ParticulateOp;
use lumit_gpu::fx::particulate_sim::{SimFrame, SimStep, SimView, SimWork};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::AuxSlot;

type Tex = wgpu::Texture;

/// How often a walk keeps where it got to, in frames.
pub(crate) const KEEP_EVERY: i64 = 8;

/// The most bytes kept states hold between them, and at most a quarter of what
/// the governor gives the card. Under pressure no more are kept at all.
const KEPT_BYTES: u64 = 4 << 30;

/// The most bytes walks hold in working buffers between them. The walk in hand
/// always has its own.
const WALK_BYTES: u64 = 1 << 30;

/// A state kept for the frames after it.
struct Kept {
    buffer: wgpu::Buffer,
    ledger: Ledger,
}

impl lumit_cache::ByteSized for Kept {
    fn byte_size(&self) -> usize {
        self.buffer.size() as usize
    }
}

/// A walk's working buffers and how far they have got.
struct Live {
    name: [u8; 32],
    work: Arc<SimWork>,
    ledger: Ledger,
    /// The frame's own state, while `work` holds a time between frames, and what
    /// the ledger has granted for it.
    between: Option<(wgpu::Buffer, Option<Reservation>)>,
    colliders: colliders::Models,
    /// What the ledger has granted for `work`.
    _held: Option<Reservation>,
}

/// Kept states and working buffers, for the one card they are on.
struct Store {
    ledger: Weak<lumit_budget::Ledger>,
    kept: lumit_cache::ByteLru<u128, Kept>,
    /// Latest used first.
    live: Vec<Live>,
}

/// One store a card, so a Viewer and an export don't take each other's walks.
static STORES: Mutex<Vec<Store>> = Mutex::new(Vec::new());

/// Runs `f` on the store for `ctx`'s card, and lets go of a card that has gone.
fn with_store<R>(ctx: &GpuContext, f: impl FnOnce(&mut Store) -> R) -> Option<R> {
    let mut stores = STORES.lock().ok()?;
    stores.retain(|s| s.ledger.strong_count() > 0);
    let at = Arc::as_ptr(ctx.ledger());
    if !stores.iter().any(|s| s.ledger.as_ptr() == at) {
        let budget = ctx.ledger().budget(Tier::Vram) / 4;
        let mut kept = lumit_cache::ByteLru::new(KEPT_BYTES.min(budget) as usize);
        kept.account_against(Arc::clone(ctx.ledger()), Tier::Vram);
        stores.push(Store {
            ledger: Arc::downgrade(ctx.ledger()),
            kept,
            live: Vec::new(),
        });
    }
    stores.iter_mut().find(|s| s.ledger.as_ptr() == at).map(f)
}

fn state_name(walk: &[u8; 32], frame: i64) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(walk);
    h.update(&frame.to_le_bytes());
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    u128::from_le_bytes(out)
}

/// The furthest state kept of `walk` from `from` to `frame`.
fn furthest_kept(ctx: &GpuContext, walk: &[u8; 32], frame: i64, from: i64) -> Option<Kept> {
    with_store(ctx, |s| {
        let mut k = frame.div_euclid(KEEP_EVERY) * KEEP_EVERY;
        while k >= from {
            if let Some(kept) = s.kept.get(&state_name(walk, k)) {
                return Some(Kept {
                    buffer: kept.buffer.clone(),
                    ledger: kept.ledger,
                });
            }
            k -= KEEP_EVERY;
        }
        None
    })
    .flatten()
}

/// A walk's name: the rows and all it reads, and which clone it is.
fn walk_name(carried: &Trails) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("lumit card walk 1");
    h.update(&carried.key);
    h.update(&carried.clone.0.to_le_bytes());
    h.update(&carried.clone.1.to_le_bytes());
    *h.finalize().as_bytes()
}

/// Everything a walk needs of the document, found once.
pub(crate) struct Source<'a> {
    pub doc: &'a Arc<Document>,
    pub comp: &'a Composition,
    pub layer: &'a Layer,
    pub inst: &'a EffectInstance,
    pub clone: (u32, u32),
    pub dt: f64,
}

impl<'a> Source<'a> {
    pub(crate) fn find(
        doc: &'a Arc<Document>,
        comp: uuid::Uuid,
        layer: uuid::Uuid,
        effect: uuid::Uuid,
        clone: (u32, u32),
    ) -> Option<Self> {
        let comp = doc.comp(comp)?;
        let layer = comp.layers.iter().find(|l| l.id == layer)?;
        let inst = layer.effects.iter().find(|e| e.id == effect)?;
        Some(Source {
            doc,
            comp,
            layer,
            inst,
            clone,
            dt: 1.0 / comp.frame_rate.fps().max(1.0),
        })
    }

    fn context(&self, tc: f64) -> Arc<ExpressionContext> {
        Arc::new(ExpressionContext {
            document: Arc::clone(self.doc),
            comp: Some(self.comp.id),
            layer: Some(self.layer.id),
            comp_time: tc,
            current_depth: 0,
            inputs: None,
        })
    }

    /// The rows at layer time `lt`, in px@comp.
    fn inputs(&self, lt: f64) -> Option<Inputs> {
        let tc = lt + self.layer.start_offset.0.to_f64();
        let audio = crate::audio_tap::DocumentAudio::new(self.doc, self.comp, tc);
        lumit_core::fx::drivers::walk::rows_at(
            &self.layer.graph,
            self.inst.id,
            lt,
            self.context(tc),
            Some(&audio),
            self.clone,
        )
    }
}

/// The steps of one frame, or the part of one `h` seconds long a time between
/// frames asks for. `frame` is the frame it starts from.
fn steps_of(then: &Inputs, steady: &Steady, frame: i64, h: f64, dt: f64) -> Vec<SimStep> {
    let phys = physics(&then.rows);
    let whole = h >= dt;
    let count = if whole { phys.steps.max(1) } else { 1 };
    let each = h / f64::from(count);
    // The flow's push is sized so its top speed at this Drag is about the row.
    let curl = then.rows.turbulence_amount.max(0.0) * then.rows.drag.max(0.5) * 2.0;
    (0..count)
        .map(|k| {
            let end = f64::from(k + 1) * each;
            let (a, b) = steady.between(f64::from(k) * each, end);
            SimStep {
                h: each as f32,
                clock: (frame as f64 * dt + end) as f32,
                steady_first: steady.first + u64::from(a),
                steady_n: b - a,
                off0: steady.off(a) as f32,
                off_step: (steady.dt / f64::from(steady.n.max(1))) as f32,
                end: end as f32,
                burst_first: 0,
                burst_n: 0,
                contact: phys.contact,
                keep: phys.keep,
                slip: phys.slip,
                sparks: phys.sparks,
                spark_speed: phys.spark_speed,
                spark_life: phys.spark_life,
                sparks_from: phys.sparks_from,
                axis: phys.axis,
                attractor: phys.attractor,
                vortex: phys.vortex,
                field: phys.field,
                sphere: phys.sphere,
                curl,
                flock_radius: phys.flock_radius,
                apart: phys.apart,
                along: phys.along,
                together: phys.together,
                pose0: (f64::from(k) / f64::from(count)) as f32,
                pose1: (f64::from(k + 1) / f64::from(count)) as f32,
                spark_inherit: phys.spark_inherit,
            }
        })
        .collect()
}

/// One lot of steps of `work`, from rows `then`.
#[allow(clippy::too_many_arguments)]
fn run_steps(
    fx: &FxEngine,
    ctx: &GpuContext,
    src: &Source<'_>,
    work: &SimWork,
    models: &mut colliders::Models,
    then: &Inputs,
    (from_lt, lt): (f64, f64),
    steps: &[SimStep],
) {
    let packed = then.rows.points();
    let style = then.rows.draw_style();
    let sched = lumit_core::fx::points::PointsSchedule::default();
    let frames = [[0u32; 4]];
    let curves = {
        let look = &packed.particle;
        let mut c = Vec::with_capacity(look.size_curve.len() * 2);
        c.extend_from_slice(&look.size_curve);
        c.extend_from_slice(&look.opacity_curve);
        c
    };
    let surface = models.surface(src);
    let poly = if packed.emitter.shape.is_outline() {
        lumit_core::fx::points::emitter::outline_polyline(&packed.emitter)
    } else {
        then.path().clone()
    };
    let path: Vec<[f32; 4]> = match &surface {
        Some(tris) => tris.to_vec(),
        None => poly
            .points
            .iter()
            .zip(poly.arc.iter())
            .map(|(pt, a)| [pt[0], pt[1], *a, 0.0])
            .collect(),
    };
    let op = crate::gpufx_particulate::particulate_op(
        &packed,
        style,
        &sched,
        &frames,
        &curves,
        &path,
        poly.length(),
        0,
        None,
        None,
        0,
    );
    let op = ParticulateOp {
        shape: if surface.is_some() { 10 } else { op.shape },
        ..op
    };
    let frame = then.frame();
    let offset = src.layer.start_offset.0.to_f64();
    let (rows, edges) = models.at(ctx, src, from_lt + offset, lt + offset, &then.rows);
    fx.sim_steps(
        ctx,
        work,
        &SimFrame {
            op: &op,
            frame: (frame[0].max(1.0) as u32, frame[1].max(1.0) as u32),
            colliders: &rows,
            edges: &edges,
            distances: models.distances(),
        },
        steps,
    );
}

/// A kept state put back, and whatever the walk had to the side of it.
fn start_from(
    fx: &FxEngine,
    ctx: &GpuContext,
    src: &Source<'_>,
    name: &[u8; 32],
    live: &mut Live,
    target: i64,
    from: i64,
) -> Option<()> {
    if let Some((state, _held)) = live.between.take() {
        fx.sim_restore(ctx, &live.work, &state);
    }
    let here = Some(live.ledger.frame).filter(|f| *f <= target && *f >= from);
    let kept =
        furthest_kept(ctx, name, target, from).filter(|k| here.is_none_or(|f| k.ledger.frame > f));
    if let Some(kept) = kept {
        fx.sim_restore(ctx, &live.work, &kept.buffer);
        live.ledger = kept.ledger;
    }
    let reached = kept_frame(here, live, target);
    // A walk already within a few frames needs no file, which is what keeps
    // playing on from reading the disk every so often.
    if reached.is_none_or(|r| target - r > KEEP_EVERY) {
        if let Some(baked) = bake::nearest(src.inst.id, name, target, reached) {
            if fx.sim_write(ctx, &live.work, &baked.bytes) {
                live.ledger = baked.ledger;
                return Some(());
            }
        }
    }
    if reached.is_none() {
        // From the in point, with the burst it opens on.
        fx.sim_clear(ctx, &live.work);
        live.ledger = Ledger {
            frame: from,
            ..Ledger::default()
        };
        let now = src.inputs(from as f64 * src.dt)?;
        let (first, n) = live.ledger.burst(&now.rows);
        if n > 0 {
            let steady = live.ledger.steady(0.0, src.dt, live.work.cap);
            let mut steps = steps_of(&now, &steady, from, src.dt, src.dt);
            steps.truncate(1);
            if let Some(s) = steps.first_mut() {
                s.h = 0.0;
                s.burst_first = first;
                s.burst_n = n.min(live.work.cap);
            }
            let (work, lt) = (Arc::clone(&live.work), from as f64 * src.dt);
            run_steps(
                fx,
                ctx,
                src,
                &work,
                &mut live.colliders,
                &now,
                (lt, lt),
                &steps,
            );
        }
    }
    Some(())
}

fn kept_frame(here: Option<i64>, live: &Live, target: i64) -> Option<i64> {
    let f = live.ledger.frame;
    (here.is_some() || f <= target && f > 0).then_some(f)
}

#[cfg(feature = "test-fixtures")]
thread_local! {
    static WALKED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many frames this thread has walked, for a test to see a change draw again
/// rather than walk again.
#[cfg(feature = "test-fixtures")]
#[must_use]
pub fn walked_here() -> u64 {
    WALKED.with(std::cell::Cell::get)
}

/// One whole frame on from `live`'s, whose rows at the frame's start are `now`.
/// Hands back the rows at its end.
fn frame_on(
    fx: &FxEngine,
    ctx: &GpuContext,
    src: &Source<'_>,
    live: &mut Live,
    now: &Inputs,
) -> Option<Inputs> {
    #[cfg(feature = "test-fixtures")]
    WALKED.with(|w| w.set(w.get() + 1));
    let f = live.ledger.frame;
    let lt = (f + 1) as f64 * src.dt;
    let then = src.inputs(lt)?;
    let rate = now.rows.emit_rate;
    let steady = live.ledger.steady(rate, src.dt, live.work.cap);
    let mut steps = steps_of(&then, &steady, f, src.dt, src.dt);
    live.ledger.advance(rate, &steady);
    let (first, n) = live.ledger.burst(&then.rows);
    if let Some(last) = steps.last_mut() {
        last.burst_first = first;
        last.burst_n = n.min(live.work.cap);
    }
    let work = Arc::clone(&live.work);
    let from = f as f64 * src.dt;
    run_steps(
        fx,
        ctx,
        src,
        &work,
        &mut live.colliders,
        &then,
        (from, lt),
        &steps,
    );
    Some(then)
}

/// Particle steps between handing work to the card and waiting for it, so a
/// stop lands soon and no one submission gets near the card's limit on a slow
/// one (docs/13 section 5).
const WORK_PER_SUBMIT: u64 = 2_000_000;

/// Counts `more` particle steps of work, and hands them over once they add up.
pub(crate) fn hand_over(ctx: &GpuContext, work: &mut u64, more: u64) {
    *work += more;
    if *work >= WORK_PER_SUBMIT {
        *work = 0;
        ctx.flush();
        ctx.device.poll(wgpu::Maintain::Wait);
    }
}

/// Keeps where `live` has got to, for the frames after it, unless the card is
/// short: a walk then only goes slower, and its frames are the same.
fn keep(fx: &FxEngine, ctx: &GpuContext, name: &[u8; 32], live: &Live) {
    if !matches!(ctx.vram_pressure(), lumit_budget::Pressure::Easy) {
        return;
    }
    let buffer = fx.sim_keep(ctx, &live.work);
    let at = state_name(name, live.ledger.frame);
    let ledger = live.ledger;
    let _ = with_store(ctx, |s| s.kept.insert(at, Kept { buffer, ledger }));
}

/// `name`'s working buffers, or new ones for `cap` particles with none born.
fn take_live(fx: &FxEngine, ctx: &GpuContext, name: &[u8; 32], cap: u32) -> Option<Live> {
    let found = with_store(ctx, |s| {
        s.live
            .iter()
            .position(|l| l.name == *name && l.work.cap == cap)
            .map(|at| s.live.remove(at))
    })?;
    if found.is_some() {
        return found;
    }
    Some(bake::fresh(fx, ctx, *name, cap))
}

/// `live` back in the store, with the walks used longest ago let go once they
/// hold too much.
fn give_back(ctx: &GpuContext, live: Live) {
    let _ = with_store(ctx, |s| {
        s.live.insert(0, live);
        let mut bytes = 0;
        let mut first = true;
        s.live.retain(|l| {
            bytes += l.work.bytes();
            let stays = first || bytes <= WALK_BYTES;
            first = false;
            stays
        });
    });
}

/// The particles at layer time `t`, on the card. `None` before the in point,
/// when a row cannot be read, or when the frame is no longer wanted.
fn walked(
    fx: &FxEngine,
    ctx: &GpuContext,
    src: &Source<'_>,
    name: &[u8; 32],
    t: f64,
) -> Option<Arc<SimWork>> {
    if t.is_nan() || t < 0.0 {
        return None;
    }
    let dt = src.dt;
    let target = (t / dt + 1e-6).floor() as i64;
    let from = target
        .saturating_sub(lumit_core::fx::points::MAX_SCAN_FRAMES)
        .max(0);
    // A walk that started late is not the walk from the in point.
    let keeps = from == 0;
    let first = src.inputs(from as f64 * dt)?;
    let cap = first.rows.points().cap;
    let mut live = take_live(fx, ctx, name, cap)?;
    if start_from(fx, ctx, src, name, &mut live, target, from).is_none() {
        give_back(ctx, live);
        return None;
    }
    let mut work = 0u64;
    let mut now = src.inputs(live.ledger.frame as f64 * dt)?;
    while live.ledger.frame < target {
        // A frame nobody wants any more stops here, and the walk is left where
        // it got to for the next.
        if crate::cancel::requested() {
            crate::cancel::cut_short();
            give_back(ctx, live);
            return None;
        }
        let Some(then) = frame_on(fx, ctx, src, &mut live, &now) else {
            give_back(ctx, live);
            return None;
        };
        now = then;
        if keeps && live.ledger.frame % KEEP_EVERY == 0 {
            keep(fx, ctx, name, &live);
        }
        // The last frame's steps go with the draw.
        if live.ledger.frame < target {
            hand_over(
                ctx,
                &mut work,
                u64::from(cap) * u64::from(physics(&now.rows).steps),
            );
        }
    }
    // A time between frames is the frame before, moved on by the part of a
    // frame it is past it, with the frame's own state kept to the side.
    let h = t - target as f64 * dt;
    if h > 1e-9 {
        let held = ctx
            .ledger()
            .reserve(Tier::Vram, live.work.state.size())
            .ok();
        live.between = Some((fx.sim_keep(ctx, &live.work), held));
        let then = src.inputs(t)?;
        let steady = live.ledger.steady(now.rows.emit_rate, dt, cap);
        let (a, b) = steady.between(0.0, h);
        let part = Steady {
            first: steady.first + u64::from(a),
            n: b - a,
            from: 0,
            dt: h,
        };
        let steps = steps_of(&then, &part, target, h, dt);
        let work = Arc::clone(&live.work);
        let from = target as f64 * dt;
        run_steps(
            fx,
            ctx,
            src,
            &work,
            &mut live.colliders,
            &then,
            (from, t),
            &steps,
        );
    }
    let work = Arc::clone(&live.work);
    give_back(ctx, live);
    Some(work)
}

/// Lets go of every walk and kept state, on a project going away.
pub fn let_go() {
    if let Ok(mut stores) = STORES.lock() {
        stores.clear();
    }
}

/// Lets go of the kept states, with the rest of the card's caches. A walk then
/// starts from where it is, or from the start.
pub fn clear_kept() {
    if let Ok(mut stores) = STORES.lock() {
        for s in stores.iter_mut() {
            s.kept.clear();
        }
    }
}

/// How long one step of `effect`'s walk takes on the card, in milliseconds, the
/// fastest of `runs` frames after `warm` to fill it. `None` with no card.
#[must_use]
pub fn timed_steps(
    doc: &Arc<Document>,
    comp: uuid::Uuid,
    layer: uuid::Uuid,
    effect: uuid::Uuid,
    warm: i64,
    runs: i64,
) -> Option<f64> {
    let src = Source::find(doc, comp, layer, effect, lumit_core::fx::drivers::NO_CLONE)?;
    let ctx = GpuContext::headless().ok()?;
    let fx = FxEngine::new(&ctx);
    let mut now = src.inputs(0.0)?;
    let cap = now.rows.points().cap;
    let steps = f64::from(physics(&now.rows).steps);
    let mut live = bake::fresh(&fx, &ctx, [0; 32], cap);
    fx.sim_clear(&ctx, &live.work);
    live.ledger.frame = 0;
    let mut best = f64::INFINITY;
    for k in 0..warm + runs {
        let begun = std::time::Instant::now();
        now = frame_on(&fx, &ctx, &src, &mut live, &now)?;
        ctx.device.poll(wgpu::Maintain::Wait);
        if k >= warm {
            best = best.min(begun.elapsed().as_secs_f64() * 1e3 / steps);
        }
    }
    Some(best)
}

/// The particles a simulation's op got to, for a wire to read. `None` for any
/// other op, and when the frame is no longer wanted.
pub(crate) fn for_stream(
    fx: &FxEngine,
    ctx: &GpuContext,
    aux: AuxSlot<'_>,
) -> Option<Arc<SimWork>> {
    let sched = aux.schedule()?;
    let carried = sched.trails.as_ref().filter(|t| t.sim)?;
    let (effect, _) = aux.op();
    let src = Source::find(
        &carried.doc,
        carried.comp,
        carried.layer,
        effect,
        carried.clone,
    )?;
    walked(fx, ctx, &src, &walk_name(carried), sched.t)
}

/// How `work`'s particles are drawn with the rows `p`.
pub(crate) fn view<'a>(work: &'a SimWork, p: Params<'_>) -> SimView<'a> {
    SimView {
        state: &work.state,
        px: P::px_scale_of(p),
        spark_size: physics(&P::read(p)).spark_size,
    }
}

/// A simulated Particulate over `input`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    fx: &FxEngine,
    ctx: &GpuContext,
    input: &Tex,
    w: u32,
    h: u32,
    p: Params<'_>,
    aux: AuxSlot<'_>,
    carried: &Trails,
) -> Tex {
    let (effect, _) = aux.op();
    let t = aux.schedule().map_or(0.0, |s| s.t);
    let source = Source::find(
        &carried.doc,
        carried.comp,
        carried.layer,
        effect,
        carried.clone,
    );
    let work = source.and_then(|src| walked(fx, ctx, &src, &walk_name(carried), t));
    let Some(work) = work else {
        return input.clone();
    };
    crate::gpufx_particulate::Particulate::with_op(p, aux, |op, sprite| {
        let op = ParticulateOp {
            sim: Some(view(&work, p)),
            ..*op
        };
        crate::realise::scene::draw(fx, ctx, input, (w, h), sprite, &op, p)
            .unwrap_or_else(|| fx.particulate(ctx, input, w, h, sprite, &op))
    })
}
