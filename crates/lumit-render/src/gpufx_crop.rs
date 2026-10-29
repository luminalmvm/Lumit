//! Crop's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct Crop;

impl GpuEffect for Crop {
    fn match_name(&self) -> &'static str {
        "crop"
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
        let c = effects::crop::Crop::read(p).packed();
        let op = lumit_gpu::fx::utility::CropOp {
            edges: c.edges,
            band: c.band,
            mix: c.mix,
        };
        fx.crop(ctx, tex, w, h, &op)
    }
}
