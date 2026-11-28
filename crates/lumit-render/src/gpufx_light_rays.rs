use lumit_core::fx::effects::light_rays::{LightRays, LightRaysParams, PASSES, TAPS};
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::light_rays::LightRaysOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct LightRaysFx;

pub(crate) fn op(l: LightRaysParams) -> LightRaysOp {
    let weights = std::array::from_fn(|k| std::array::from_fn(|j| LightRaysParams::weight(k, j)));
    let scales = std::array::from_fn(|k| LightRaysParams::step(k, 1.0));
    debug_assert_eq!((PASSES, TAPS), (4, 8));
    LightRaysOp {
        source: l.source,
        lengths: l.lengths,
        threshold: l.threshold,
        mask: l.mask,
        colourise: l.colourise,
        based_on: l.based_on,
        edge: l.edge,
        stops: l.stops,
        at: l.at,
        count: l.count,
        gain: l.gain,
        source_opacity: l.source_opacity,
        rays_opacity: l.rays_opacity,
        blend: l.blend,
        mix: l.mix,
        weights,
        scales,
    }
}

impl GpuEffect for LightRaysFx {
    fn match_name(&self) -> &'static str {
        "light_rays"
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
        let l = LightRays::read(p).packed(w, h);
        if l.is_identity() {
            return tex.clone();
        }
        fx.light_rays(ctx, tex, w, h, &op(l))
    }
}
