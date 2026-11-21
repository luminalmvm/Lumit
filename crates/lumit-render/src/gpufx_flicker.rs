//! Flicker's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::flicker::Flicker;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::flicker::FlickerOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct FlickerFx;

impl GpuEffect for FlickerFx {
    fn match_name(&self) -> &'static str {
        "flicker"
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
        let f = Flicker::read(p).packed(p);
        // A frame where nothing moves costs nothing.
        if f.is_identity() {
            return tex.clone();
        }
        let op = FlickerOp {
            gain: f.gain,
            contrast: f.contrast,
            opacity: f.opacity,
            mix: f.mix,
        };
        fx.flicker(ctx, tex, w, h, &op)
    }
}
