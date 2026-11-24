use lumit_core::fx::{
    effects::turbiniu::Turbiniu as Controls, params::Params, registry::EffectMetadata,
};
use lumit_core::mask::MaskPolyline;
use lumit_gpu::{fx::FxEngine, GpuContext};

use super::{AuxSlot, GpuEffect, Tex};

pub(super) struct Turbiniu;

impl GpuEffect for Turbiniu {
    fn match_name(&self) -> &'static str {
        "turbiniu"
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let controls = Controls::read(p);
        let scale = p.float(Controls::PX_SCALE, 1.0);
        let empty = MaskPolyline::default();
        let first = aux.mask_path_n(0).unwrap_or(&empty);
        let second = aux.mask_path_n(1).unwrap_or(&empty);
        if controls.mix <= 0.0 || first.length() <= 0.0 || second.length() <= 0.0 {
            return tex.clone();
        }
        let vertices = controls.geometry(first, second, scale, [w, h]);
        fx.turbiniu(
            ctx,
            tex,
            &vertices,
            controls.blending == 1,
            controls.multisampling == 1,
            (controls.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}
