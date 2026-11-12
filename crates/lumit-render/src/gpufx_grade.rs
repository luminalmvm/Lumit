//! CDL's and the colour warper's GPU passes, kept beside the dispatch table
//! rather than in it.

use lumit_core::fx::effects::{cdl::Cdl, colour_warper::ColourWarper};
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct CdlFx;

impl GpuEffect for CdlFx {
    fn match_name(&self) -> &'static str {
        "cdl"
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
        let pk = Cdl::read(p).packed();
        if pk.neutral {
            return tex.clone();
        }
        fx.cdl(ctx, tex, w, h, &pk.words())
    }
}

pub(crate) struct ColourWarperFx;

impl GpuEffect for ColourWarperFx {
    fn match_name(&self) -> &'static str {
        "colour_warper"
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
        let pk = ColourWarper::read(p).packed();
        if pk.neutral {
            return tex.clone();
        }
        fx.colour_warper(ctx, tex, w, h, &pk.words())
    }
}
