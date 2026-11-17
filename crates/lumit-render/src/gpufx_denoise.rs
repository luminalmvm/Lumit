//! Denoise's GPU pass, kept beside the dispatch table rather than in it.
//!
//! The neighbour frames and the motion measured to each arrive whole
//! ([`AuxKind::FlowFields`]). A neighbour is averaged in only when a field came
//! with it: the decode files none for the other side of a cut, for the same
//! source frame again (a held end, a slowed clip) or for a measurement that
//! failed. With Follow motion off the field is one of no motion, so the
//! neighbour is read where it is.

use lumit_core::fx::denoise::DenoiseParams;
use lumit_core::fx::effects::denoise::Denoise;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::denoise::{DenoiseKernels, DenoiseOp, Neighbour};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{AuxData, AuxKind, AuxSlot, GpuEffect};

/// The kernels' copy of the resolved numbers.
fn op_of(p: &DenoiseParams) -> DenoiseOp {
    DenoiseOp {
        view: p.view,
        luma_sigma: p.luma_sigma,
        chroma_sigma: p.chroma_sigma,
        tone: p.tone,
        luma_bands: p.luma_bands,
        chroma_bands: p.chroma_bands,
        frames: p.frames,
        t0: p.t0,
        t1: p.t1,
        strength: p.strength,
        luma_amount: p.luma_amount,
        chroma_amount: p.chroma_amount,
        band_amounts: p.band_amounts,
        detail: p.detail,
        sharpen: p.sharpen,
        grain: p.grain,
        mix: p.mix,
    }
}

pub(crate) struct DenoiseFx;

impl GpuEffect for DenoiseFx {
    fn match_name(&self) -> &'static str {
        "denoise"
    }

    fn aux(&self) -> AuxKind {
        AuxKind::FlowFields
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
        let d = Denoise::read(p).packed();
        let reach = d.frames as i32;
        let fields = match aux.data {
            AuxData::FlowFields(fields, _) => fields,
            _ => &[],
        };
        // In offset order, so the sums are made in the order the CPU twin
        // makes them whatever order the frames were decoded in.
        let mut near: Vec<&(i32, wgpu::Texture)> = aux
            .neighbours()
            .iter()
            .filter(|(o, _)| *o != 0 && o.abs() <= reach)
            .collect();
        near.sort_by_key(|(o, _)| *o);
        near.dedup_by_key(|(o, _)| *o);
        let neighbours: Vec<Neighbour<'_>> = near
            .into_iter()
            .filter_map(|(o, picture)| {
                let (_, field) = fields.iter().find(|(f, _)| f == o)?;
                Some(Neighbour {
                    picture,
                    field: d.motion.then_some(field),
                })
            })
            .collect();
        let kernels = fx
            .take_kept::<DenoiseKernels>()
            .filter(|k| k.working() == ctx.working())
            .unwrap_or_else(|| Box::new(DenoiseKernels::new(ctx)));
        let out = kernels.run(ctx, tex, w, h, &neighbours, &op_of(&d), aux.matte());
        fx.keep(kernels);
        out
    }
}
