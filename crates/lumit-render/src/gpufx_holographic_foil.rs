//! Holographic foil's GPU pass, kept beside the dispatch table rather than in
//! it.

use lumit_core::fx::effects::holographic_foil::HolographicFoil;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::holographic_foil::HolographicFoilOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct HolographicFoilFx;

impl GpuEffect for HolographicFoilFx {
    fn match_name(&self) -> &'static str {
        "holographic_foil"
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
        let foil = HolographicFoil::read(p);
        let f = foil.packed(p, w, h);
        if f.is_identity() {
            return tex.clone();
        }
        // The layer rows' pictures arrive in row order.
        let picture = |id: &str| {
            let at = <HolographicFoil as EffectMetadata>::SCHEMA
                .layer_inputs()
                .position(|row| row == id)?;
            aux.layer_inputs().get(at)?.as_ref().map(|l| &l.tex)
        };
        let op = HolographicFoilOp {
            material: foil.material,
            rows: f.blocks,
            matte: aux.matte(),
            height_map: picture("height_map"),
            foil_mask: picture("foil_mask"),
            direction_map: picture("direction_map"),
            pitch_map: picture("pitch_map"),
            environment: picture("environment"),
        };
        fx.holographic_foil(ctx, tex, w, h, &op)
    }
}
