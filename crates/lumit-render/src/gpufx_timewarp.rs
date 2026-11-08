//! Timewarp's GPU pass, kept beside the dispatch table rather than in it.
//!
//! The two moments of the layer arrive as its layer pictures, first first
//! (`lumit_core::fx::moments`). Between them it holds the first, mixes the two
//! or makes the frame from the motion it measures, then smears that along the
//! motion when the shutter is open.

use lumit_core::fx::effects::timewarp::{Timewarp, TimewarpMethod, TimewarpParams};
use lumit_core::fx::params::Params;
use lumit_gpu::fx::temporal::MotionBlurOp;
use lumit_gpu::fx::timewarp::TimewarpKernels;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

/// What Timewarp keeps on a device: the motion measurement and its own
/// kernels, made the first time it runs there and kept on the effects engine.
struct Engines {
    flow: lumit_flow::FlowEngine,
    kernels: TimewarpKernels,
}

/// The motion from `a` to `b` and back, each at the size it was measured at as
/// x, y and how far it is trusted. `None` without a card to measure on.
fn measure(
    ctx: &GpuContext,
    engines: &mut Engines,
    a: &wgpu::Texture,
    b: &wgpu::Texture,
    p: &TimewarpParams,
) -> Option<(wgpu::Texture, wgpu::Texture)> {
    use lumit_core::retime::{FlowParams, FlowResolution, VectorDetail};
    let set = crate::decode::flow_settings(&FlowParams {
        resolution: match p.divisor {
            1 => FlowResolution::Native,
            2 => FlowResolution::Half,
            _ => FlowResolution::Quarter,
        },
        detail: VectorDetail::from_code(p.detail).unwrap_or_default(),
        smoothness: f64::from(p.smoothness),
        ..FlowParams::default()
    });
    // The solver reads the two pictures on an encoder of its own, and a
    // command that has not been submitted has not run.
    ctx.flush();
    let (fwd, bwd) = engines.flow.flow_pair_textures(a, b, &set)?;
    // Each steadied against the other as measured, as the frame synthesis on
    // footage does. The kernels read them at this size.
    let (f, g) = (
        lumit_flow::borrow_uncertain(&fwd, &bwd),
        lumit_flow::borrow_uncertain(&bwd, &fwd),
    );
    let upload = |x: &lumit_flow::FlowField, y: &lumit_flow::FlowField| {
        let conf = lumit_flow::confidence(x, y);
        lumit_gpu::fx::common::upload_flow_field(ctx, &x.u, &x.v, &conf, x.w as u32, x.h as u32)
    };
    Some((upload(&f, &g), upload(&g, &f)))
}

pub(crate) struct TimewarpFx;

impl GpuEffect for TimewarpFx {
    fn match_name(&self) -> &'static str {
        "timewarp"
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &wgpu::Texture,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> wgpu::Texture {
        let p = Timewarp::packed_from(p);
        let pictures = aux.layer_inputs();
        let picture = |i: usize| pictures.get(i).and_then(|l| l.as_ref()).map(|l| &l.tex);
        // No moment to show is the frame as it is.
        let Some(a) = picture(0) else {
            return tex.clone();
        };
        let b = picture(1);
        let mut engines = fx.take_kept::<Engines>().unwrap_or_else(|| {
            Box::new(Engines {
                flow: lumit_flow::FlowEngine::with_context(ctx),
                kernels: TimewarpKernels::new(ctx),
            })
        });
        let out = {
            let engines = &mut *engines;
            let needs_motion =
                b.is_some() && (p.method == TimewarpMethod::PixelMotion || p.shutter > 0.0);
            let fields = match b {
                Some(b) if needs_motion => measure(ctx, engines, a, b, &p),
                _ => None,
            };
            let shown = match (p.method, b) {
                (TimewarpMethod::WholeFrames, _) | (_, None) => a.clone(),
                (_, Some(_)) if p.phase <= 0.0 => a.clone(),
                (TimewarpMethod::PixelMotion, Some(b)) => match &fields {
                    Some((f, g)) => engines.kernels.synth(ctx, a, b, f, g, p.phase),
                    // Without a measurement the frames are mixed.
                    None => engines.kernels.mix(ctx, a, b, p.phase),
                },
                (TimewarpMethod::FrameMix, Some(b)) => engines.kernels.mix(ctx, a, b, p.phase),
            };
            let shown = match &fields {
                Some((f, _)) if p.shutter > 0.0 => fx.motion_blur(
                    ctx,
                    &shown,
                    &engines.kernels.field(ctx, f, w, h),
                    w,
                    h,
                    None,
                    &MotionBlurOp {
                        shutter_frac: p.shutter,
                        samples: p.samples,
                        mix: 1.0,
                        view: 0,
                        quality: 0,
                        vector_scale: 1.0,
                    },
                ),
                _ => shown,
            };
            if p.mix >= 1.0 {
                shown
            } else {
                engines.kernels.mix(ctx, tex, &shown, p.mix)
            }
        };
        fx.keep(engines);
        out
    }
}
