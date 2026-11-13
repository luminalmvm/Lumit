//! Light leak's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::light_leak::LightLeak;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::light_leak::LightLeakOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct LightLeakFx;

impl GpuEffect for LightLeakFx {
    fn match_name(&self) -> &'static str {
        "light_leak"
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
        let l = LightLeak::read(p).packed(p, w, h);
        // The ends of a cut have no light on them, and cost nothing.
        if l.is_identity() {
            return tex.clone();
        }
        let op = LightLeakOp {
            normal: l.normal,
            centre: l.centre,
            inv_h: l.inv_h,
            z: l.z,
            colour: l.colour,
            colour_2: l.colour_2,
            seed: l.seed,
            mix: l.mix,
        };
        fx.light_leak(ctx, tex, w, h, &op)
    }
}
