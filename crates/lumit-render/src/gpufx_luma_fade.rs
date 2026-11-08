//! Luma fade's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::luma_fade::LumaFade;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::fx::schema::MATTE_INVERT_ID;
use lumit_gpu::fx::utility::LumaFadeOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct LumaFadeFx;

impl GpuEffect for LumaFadeFx {
    fn match_name(&self) -> &'static str {
        "luma_fade"
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
        let l = LumaFade::read(p).packed(p.bool(MATTE_INVERT_ID, false));
        let op = LumaFadeOp {
            completion: l.completion,
            band: l.band,
            invert: l.invert,
            mix: l.mix,
        };
        fx.luma_fade(ctx, tex, w, h, aux.matte(), &op)
    }
}
