//! Jolt's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::jolt::Jolt;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::jolt::JoltOp;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxSlot, GpuEffect};

pub(crate) struct JoltFx;

impl GpuEffect for JoltFx {
    fn match_name(&self) -> &'static str {
        "jolt"
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
        let j = Jolt::read(p).packed(p, w, h);
        // A time hit: the layer at another moment, in place of its input.
        let moved = aux.layer_inputs().first().and_then(|l| l.as_ref());
        // Most frames have no jolt on them, and cost nothing.
        if j.is_identity() && moved.is_none() {
            return tex.clone();
        }
        let op = JoltOp {
            blur: j.blur,
            zoom: j.zoom,
            offset: j.offset,
            split: j.split,
            tint: j.tint,
            gain: j.gain,
            opacity: j.opacity,
            edge: j.edge,
            mix: j.mix,
        };
        fx.jolt(ctx, moved.map_or(tex, |m| &m.tex), tex, w, h, &op)
    }
}
