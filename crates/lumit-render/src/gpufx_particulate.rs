//! Particulate's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{path_of, stream_points, AuxSlot, GpuEffect};

type Tex = wgpu::Texture;

/// Particulate's GPU op, built from the resolved bag and the schedule threaded
/// beside it (docs/08 §3.86, points-stream.md §3.3).
///
/// Nothing here decides anything: `Particulate::points` and
/// `Particulate::draw_style` did the reducing, in the one expression the CPU
/// reference also reads, and this only lays the numbers out the way the kernel
/// wants them. The two things it *does* work out are the two the bag cannot
/// carry — the candidate window, and each recorded frame's distance from the
/// evaluation time, subtracted in `f64` so a two-second age never has to be
/// recovered from inside an hour-long clock.
#[allow(clippy::too_many_arguments)]
pub(crate) fn particulate_op<'a>(
    packed: &'a lumit_core::fx::points::PointsParams,
    style: lumit_core::fx::points::DrawStyle,
    sched: &lumit_core::fx::points::PointsSchedule,
    frames: &'a [[u32; 4]],
    curves: &'a [f32],
    path: &'a [[f32; 4]],
    path_total: f32,
    mode: u32,
    projection: Option<[[f32; 4]; 3]>,
    density: Option<&'a Tex>,
    blend: u32,
) -> lumit_gpu::fx::particulate::ParticulateOp<'a> {
    let e = &packed.emitter;
    let look = &packed.particle;
    let f = &packed.forces;
    let dt = sched.schedule.dt();
    lumit_gpu::fx::particulate::ParticulateOp {
        emitter_pos: e.position,
        emitter_wh: [e.width, e.height],
        emitter_depth: e.depth,
        emitter_angle_deg: e.angle_deg,
        direction_deg: e.direction_deg,
        direction_z_deg: e.direction_z_deg,
        spread_deg: e.spread_deg,
        spread_z_deg: e.spread_z_deg,
        speed: e.speed,
        speed_jitter: e.speed_jitter,
        shape: e.shape as u32,
        seed: packed.seed,
        cap: packed.cap,
        life: look.life,
        life_jitter: look.life_jitter,
        size: look.size,
        size_jitter: look.size_jitter,
        rotation_deg: look.rotation_deg,
        rotation_jitter_deg: look.rotation_jitter_deg,
        spin_deg: look.spin_deg,
        align_to_motion: look.align_to_motion,
        colour: look.colour,
        end_colour: look.end_colour,
        wind: f.wind,
        gravity: f.gravity,
        drag: f.drag,
        turbulence: f.turbulence,
        turbulence_scale: f.turbulence_scale,
        turbulence_speed: f.turbulence_speed,
        // The CPU reference's fixed central difference, whatever the raster.
        eps: ((dt * 0.5) as f32).max(1e-6),
        dt: dt as f32,
        frames,
        candidates: frames.last().map_or(0, |f| f[0]),
        curves,
        path,
        path_total,
        tail_seconds: style.streak_seconds,
        tail_lead: 0.0,
        feather: style.feather,
        mix: style.mix,
        mode,
        projection,
        edge: packed.edge as u32,
        margin: packed.margin,
        drives: [
            look.size_by as u32,
            look.opacity_by as u32,
            look.colour_by as u32,
        ],
        speed_range: look.speed_range,
        density,
        blend,
        sim: None,
    }
}

/// The per-frame table the kernel searches: `[birth offset, bits of
/// (t − that frame's start), the frame's first birth index low and high]`,
/// the steady flow's frames oldest first and then the bursts', with a closing
/// offset that is the candidate count.
pub(crate) fn particulate_frames(sched: &lumit_core::fx::points::PointsSchedule) -> Vec<[u32; 4]> {
    let s = &sched.schedule;
    let (dt, first) = (s.dt(), s.first_frame());
    let mut out: Vec<[u32; 4]> = Vec::with_capacity(s.counts().len() + s.bursts().len() + 1);
    let mut offset = 0u32;
    let mut row = |f: i64, base: u64, n: u32| {
        let rel = (sched.t - f as f64 * dt) as f32;
        out.push([offset, rel.to_bits(), base as u32, (base >> 32) as u32]);
        offset = offset.saturating_add(n);
    };
    let mut birth = s.first_birth();
    for (i, n) in s.counts().iter().enumerate() {
        row(first + i as i64, birth, *n);
        birth = birth.saturating_add(u64::from(*n));
    }
    for &(f, n) in s.bursts() {
        row(f, lumit_core::fx::points::burst_base(f), n);
    }
    out.push([offset, 0, 0, 0]);
    out
}

/// Particulate (docs/08 §3.86): evaluate, compact, draw.
///
/// **The Sprite fallback is decided here** rather than in the kernel: an unset
/// Sprite layer means the mode arrives as Disc, because a render mode must
/// always draw something (particulate.md §2) and one branch resolved host-side
/// cannot come to mean two things in two places.
///
/// **The degradation rung is [`lumit_gpu::fx::particulate::ParticulateOp::cap`]**:
/// halve it and the newest half is what draws, by the same rule the cap itself
/// applies. This pass always hands over the *declared* cap, which is why the
/// rung is never on the export path — there is nothing on this path that could
/// turn it on.
pub(crate) struct Particulate;
impl Particulate {
    /// The GPU op this frame's Particulate draws, handed to `f` with the
    /// sprite it stamps, if any.
    pub(crate) fn with_op<R>(
        p: Params<'_>,
        aux: AuxSlot<'_>,
        f: impl FnOnce(&lumit_gpu::fx::particulate::ParticulateOp<'_>, Option<&Tex>) -> R,
    ) -> R {
        use lumit_core::fx::effects::particulate::Particulate as P;
        use lumit_core::fx::points::RenderMode;
        let inst = P::read(p);
        let packed = inst.points();
        let style = inst.draw_style();
        // The pictures arrive in row order, Density layer's before Sprite
        // layer's.
        let picture = |id: &str| {
            let at = <P as EffectMetadata>::SCHEMA
                .layer_inputs()
                .position(|row| row == id)?;
            aux.layer_inputs().get(at)?.as_ref().map(|l| &l.tex)
        };
        let sprite = picture("sprite_layer");
        let mode = match style.mode {
            RenderMode::Disc => 0,
            // Unset draws discs — the documented deviation from
            // unset-is-identity, because a mode must draw something.
            RenderMode::Sprite if sprite.is_some() => 1,
            RenderMode::Sprite => 0,
            RenderMode::Streak => 2,
        };
        let default_schedule = lumit_core::fx::points::PointsSchedule::default();
        let sched = aux.schedule().unwrap_or(&default_schedule);
        let frames = particulate_frames(sched);
        let mut curves = Vec::with_capacity(packed.particle.size_curve.len() * 2);
        curves.extend_from_slice(&packed.particle.size_curve);
        curves.extend_from_slice(&packed.particle.opacity_curve);
        // The camera, in the raster this frame is drawn at: the projection
        // was built in px@comp, like the mask path beside it.
        let projection = sched
            .projection
            .map(|proj| proj.rescaled(P::px_scale_of(p)).m);
        // The polyline the emitter walks. A Mask path's arrives in px@comp and
        // takes the raster factor. An outline's is built from the packed
        // emitter, which the resolve step already rescaled.
        let poly = if packed.emitter.shape.is_outline() {
            lumit_core::fx::points::emitter::outline_polyline(&packed.emitter)
        } else {
            lumit_core::fx::points::scale_path(&path_of(aux), P::px_scale_of(p))
        };
        let path: Vec<[f32; 4]> = poly
            .points
            .iter()
            .zip(poly.arc.iter())
            .map(|(pt, a)| [pt[0], pt[1], *a, 0.0])
            .collect();
        let density = picture(inst.density_row());
        let op = particulate_op(
            &packed,
            style,
            sched,
            &frames,
            &curves,
            &path,
            poly.length(),
            mode,
            projection,
            density,
            inst.particle_blend,
        );
        f(&op, sprite.filter(|_| mode == 1))
    }

    /// The particles a collider walk got to, drawn through the shared points
    /// draw. `stream` is in px@comp.
    #[allow(clippy::too_many_arguments)]
    fn stepped(
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
        stream: &lumit_core::fx::points::PointsStream,
    ) -> Tex {
        use lumit_core::fx::effects::particulate::Particulate as P;
        let inst = P::read(p);
        let style = inst.draw_style();
        let px_scale = P::px_scale_of(p);
        let points = stream_points(&stream.rescaled(px_scale), style.streak_seconds);
        let sprite = <P as EffectMetadata>::SCHEMA
            .layer_inputs()
            .position(|row| row == "sprite_layer")
            .and_then(|at| aux.layer_inputs().get(at)?.as_ref().map(|l| &l.tex))
            .filter(|_| style.mode == lumit_core::fx::points::RenderMode::Sprite);
        let projection = aux
            .schedule()
            .and_then(|sched| sched.projection)
            .map(|proj| proj.rescaled(px_scale).m);
        fx.points_draw(
            ctx,
            tex,
            w,
            h,
            None,
            &lumit_gpu::fx::points_draw::PointsDrawOp {
                points: &points,
                feather: style.feather,
                mix: style.mix,
                projection,
                field: lumit_gpu::fx::points_draw::FieldTest::None,
                seed: inst.seed,
                mode: u32::from(sprite.is_some()),
                sprite,
            },
        )
    }
}

impl GpuEffect for Particulate {
    fn match_name(&self) -> &'static str {
        "particulate"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        if let Some(trails) = aux.schedule().and_then(|s| s.trails.as_ref()) {
            return crate::trails::draw(fx, ctx, tex, w, h, p, aux, trails);
        }
        if let Some(stream) = aux.schedule().and_then(|s| s.stepped.as_ref()) {
            return Particulate::stepped(fx, ctx, tex, w, h, p, aux, stream);
        }
        Particulate::with_op(p, aux, |op, sprite| {
            crate::realise::scene::draw(fx, ctx, tex, (w, h), sprite, op, p)
                .unwrap_or_else(|| fx.particulate(ctx, tex, w, h, sprite, op))
        })
    }

    /// Births that follow a Density layer depend on its picture, so the
    /// stream a wire reads is the one the card made, read back. Without one
    /// the driver walk made it already.
    fn points_out(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Option<lumit_core::fx::points::PointsStream> {
        use lumit_core::fx::effects::particulate::Particulate as P;
        // A walked stream was made before the render, by the driver walk, and
        // Trails draw from that walk too. A simulation's is only on the card.
        let walked = crate::sim::for_stream(fx, ctx, aux);
        if aux
            .schedule()
            .is_some_and(|s| s.stepped.is_some() || (s.trails.is_some() && walked.is_none()))
        {
            return None;
        }
        let px_scale = P::px_scale_of(p);
        let (count, words, projection) = Particulate::with_op(p, aux, |op, _| {
            let sim = walked.as_ref().map(|w| crate::sim::view(w, p));
            let op = &lumit_gpu::fx::particulate::ParticulateOp { sim, ..*op };
            sim.is_some().then_some(()).or(op.density.map(|_| ()))?;
            let (count, words) = fx.particulate_stream(ctx, tex, w, h, op)?;
            Some((count, words, op.projection))
        })?;
        let cap = P::read(p).points().cap as usize;
        let word = |i: usize| words.get(i).copied().unwrap_or(0);
        let f = |i: usize| f32::from_bits(word(i));
        // Half precision, two to a word, as the colour region is written.
        let half = |i: usize, hi: bool| {
            lumit_gpu::fx::common::f16_to_f32(if hi {
                (word(i) >> 16) as u16
            } else {
                word(i) as u16
            })
        };
        let mut s = lumit_core::fx::points::PointsStream {
            projection: projection.map_or(lumit_core::fx::points::Projection::FLAT, |m| {
                lumit_core::fx::points::Projection { m }
            }),
            ..Default::default()
        };
        for i in 0..(count as usize).min(cap) {
            s.position.push([f(i * 3), f(i * 3 + 1), f(i * 3 + 2)]);
            s.speed.push([
                f(3 * cap + i * 3),
                f(3 * cap + i * 3 + 1),
                f(3 * cap + i * 3 + 2),
            ]);
            s.age.push(f(6 * cap + i));
            s.life.push(f(7 * cap + i));
            s.size.push(f(8 * cap + i));
            s.rotation.push(f(9 * cap + i));
            let c = 10 * cap + i * 2;
            s.colour.push([
                half(c, false),
                half(c, true),
                half(c + 1, false),
                half(c + 1, true),
            ]);
            let id = 12 * cap + i * 2;
            s.id.push(u64::from(word(id)) | (u64::from(word(id + 1)) << 32));
        }
        Some(s.rescaled(1.0 / px_scale.max(1e-6)))
    }
}
