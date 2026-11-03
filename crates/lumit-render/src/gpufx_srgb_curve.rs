//! sRGB curve's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::srgb_curve::SrgbCurve;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::utility::SrgbCurveOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct SrgbCurveFx;

impl GpuEffect for SrgbCurveFx {
    fn match_name(&self) -> &'static str {
        "srgb_curve"
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &wgpu::Texture,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> wgpu::Texture {
        let (decode, mix) = SrgbCurve::read(p).packed();
        fx.srgb_curve(ctx, tex, w, h, &SrgbCurveOp { decode, mix })
    }
}
