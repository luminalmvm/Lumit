//! Hologram's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::hologram::Hologram;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::hologram::HologramOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct HologramFx;

impl GpuEffect for HologramFx {
    fn match_name(&self) -> &'static str {
        "hologram"
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
        let f = Hologram::read(p).packed(p, w, h);
        if f.is_identity() {
            return tex.clone();
        }
        // The layer rows' pictures arrive in row order.
        let picture = |id: &str| {
            let at = <Hologram as EffectMetadata>::SCHEMA
                .layer_inputs()
                .position(|row| row == id)?;
            aux.layer_inputs().get(at)?.as_ref().map(|l| &l.tex)
        };
        let op = HologramOp {
            rows: f.blocks,
            matte: aux.matte(),
            height_map: picture("height_map"),
            environment: picture("environment"),
            depth_map: picture("depth_map"),
            backdrop: picture("backdrop_layer"),
        };
        fx.hologram(ctx, tex, w, h, &op)
    }
}
