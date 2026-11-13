//! Slices' GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::slices::Slices;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::slices::SlicesOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct SlicesFx;

impl GpuEffect for SlicesFx {
    fn match_name(&self) -> &'static str {
        "slices"
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
        let s = Slices::read(p).packed();
        if s.is_identity() {
            return tex.clone();
        }
        let op = SlicesOp {
            t: s.t,
            count: s.count,
            vertical: s.vertical,
            alternate: s.alternate,
            random: s.random,
            stagger: s.stagger,
            ease: s.ease,
            incoming: s.incoming,
            smear: s.smear,
            seed: s.seed,
            mix: s.mix,
        };
        fx.slices(ctx, tex, w, h, &op)
    }
}
