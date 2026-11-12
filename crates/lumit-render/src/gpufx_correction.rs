//! The Correction's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::correction::cpu::MATTE_VIEW_GREY;
use lumit_core::fx::effects::correction::{combine, Correction, DERIVED_PX_SCALE};
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
        let out = fx.correction(ctx, tex, w, h, aux.matte(), &op);
        let view = combine::matte_view_of(p);
        if view == 0 {
            return out;
        }
        // The Viewer's matte view, which only a preview copy asks for.
        let key = fx.correction_key_picture(ctx, tex, w, h, aux.matte(), &op);
        let shown = matte_shown(fx, ctx, &out, key, w, h, view);
        if out != *tex {
            ctx.recycle(out);
        }
        shown
    }
}

/// The matte view drawn over `out`, the picture the node made, with its `key`
/// (white where nothing keys): the twin of `cpu::matte_shown`.
pub(crate) fn matte_shown(
    fx: &FxEngine,
    ctx: &GpuContext,
    out: &wgpu::Texture,
    key: wgpu::Texture,
    w: u32,
    h: u32,
    view: u32,
) -> wgpu::Texture {
    let shown = if view == 2 {
        fx.matte_prepare(ctx, &key, w, h, 0, false)
    } else {
        // The backdrop, as `cpu::matte_backdrop` draws it.
        let fill = |colour| lumit_gpu::fx::generate::FillOp { colour, mix: 1.0 };
        let back = match view {
            3 => fx.fill(ctx, out, w, h, &fill([0.0; 3])),
            4 => fx.fill(ctx, out, w, h, &fill([1.0; 3])),
            5 => fx.tint(
                ctx,
                out,
                w,
                h,
                &lumit_gpu::fx::colour::TintOp {
                    black: [0.0; 3],
                    white: [1.0; 3],
                    mix: 1.0,
                },
            ),
            _ => fx.fill(ctx, out, w, h, &fill([MATTE_VIEW_GREY; 3])),
        };
        let shown = fx.matte_mix(ctx, &back, out, &key, w, h, false);
        ctx.recycle(back);
        shown
    };
    ctx.recycle(key);
    shown
}
