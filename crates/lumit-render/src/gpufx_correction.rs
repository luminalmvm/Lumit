//! The Correction's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::correction::{Correction, DERIVED_PX_SCALE};
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::correction::CorrectionOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct CorrectionFx;

impl GpuEffect for CorrectionFx {
    fn match_name(&self) -> &'static str {
        "correction"
    }

    // Every node draws as Serial here; its Combine row is the stack's to read.
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
        let pk = Correction::read(p).packed(p.float(DERIVED_PX_SCALE, 1.0));
        let words = pk.words(aux.matte().is_some());
        let op = CorrectionOp {
            words: &words,
            data: &pk.data,
            neutral: pk.neutral,
            qualifier_on: pk.qualifier_on(),
            windows_on: pk.windows_on(),
            denoise_px: pk.qual.denoise_px,
            blur_px: pk.qual.blur_px,
            grow_px: pk.qual.grow_px,
            detail_px: pk.detail_px,
            detail_on: pk.detail != 0.0,
        };
        fx.correction(ctx, tex, w, h, aux.matte(), &op)
    }
}
