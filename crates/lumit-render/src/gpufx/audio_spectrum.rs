//! Audio spectrum's GPU op: the reading and the path arrive beside the op, and
//! the host lays them out as the same pieces and tiles the CPU reference draws.

use lumit_core::fx::effects::audio_spectrum::{
    draw::{laid_out, Laid},
    levels_of, AudioSpectrum as S,
};
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::mask::MaskPolyline;
use lumit_gpu::fx::{audio_spectrum::AudioSpectrumOp, FxEngine};
use lumit_gpu::GpuContext;

use super::{AuxSlot, GpuEffect, Tex};

pub(super) struct AudioSpectrum;

impl GpuEffect for AudioSpectrum {
    fn match_name(&self) -> &'static str {
        "audio_spectrum"
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
        let s = S::read(p);
        let px_scale = p.float(S::DERIVED_PX_SCALE, 1.0);
        let none = MaskPolyline::default();
        let path = aux.mask_path_n(0).unwrap_or(&none);
        let levels = aux.mask_path_n(1).map(levels_of).unwrap_or_default();
        draw_on(
            fx,
            ctx,
            tex,
            w,
            h,
            &laid_out(&s, px_scale, &levels, path, w, h),
        )
    }
}

/// One frame's drawing on the card, from what the CPU reference draws from.
fn draw_on(fx: &FxEngine, ctx: &GpuContext, tex: &Tex, w: u32, h: u32, laid: &Laid) -> Tex {
    fx.audio_spectrum(
        ctx,
        tex,
        w,
        h,
        &AudioSpectrumOp {
            pieces: &laid.pieces,
            tiles: [laid.tiles.cols, laid.tiles.rows],
            starts: &laid.tiles.start,
            items: &laid.tiles.items,
            half: laid.half,
            edge: laid.edge,
            composite: laid.composite,
            mix: laid.mix,
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::expression::ExpressionContext;
    use lumit_core::fx::effects::audio_spectrum::{draw::render, SHAPE_LINE, SHAPE_RING};
    use lumit_core::fx::markers::MarkerContext;

    /// The card draws what the CPU reference draws, across the shapes, over a
    /// picture with some alpha, on and off the original, and with a soft edge.
    #[test]
    fn the_card_draws_what_the_cpu_reference_draws() {
        let Some(gpu) = lumit_gpu::test_support::lease() else {
            return; // no GPU here
        };
        let (w, h) = (160u32, 96u32);
        let e = lumit_core::fx::builtins::instantiate("audio_spectrum").unwrap();
        let ops = lumit_core::fx::resolved::resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            (w as f32).hypot(h as f32),
            1.0,
            &MarkerContext::NONE,
            std::sync::Arc::new(ExpressionContext::detached()),
        );
        let mut s = S::read(ops.get(0).unwrap().params);
        s.bands = 24;
        s.from_x = 20.0;
        s.from_y = 70.0;
        s.to_x = 140.0;
        s.to_y = 60.0;
        s.height = 50.0;
        s.thickness = 5.0;
        s.hue_spread = 200.0;
        let levels: Vec<f32> = (0..24).map(|i| (i as f32 * 0.7).sin().abs()).collect();
        let src: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                let a = 0.25 + 0.5 * x;
                [0.3 * y * a, 0.2 * a, 0.1 * a, a]
            })
            .collect();
        for (name, shape, soft, composite, mirror) in [
            ("bars", 0, 0.0, false, false),
            ("line", SHAPE_LINE, 40.0, true, false),
            ("ring", SHAPE_RING, 0.0, true, true),
        ] {
            s.shape = shape;
            s.softness = soft;
            s.composite_on_original = composite;
            s.mirror = mirror;
            let laid = laid_out(&s, 1.0, &levels, &MaskPolyline::default(), w, h);
            assert!(!laid.pieces.is_empty(), "{name}: something is drawn");
            let mut cpu = src.clone();
            render(&mut cpu, w, h, &laid);
            let tex = lumit_gpu::fx::common::upload_linear_f32(&gpu, &src, w, h);
            let out = draw_on(gpu.fx(), &gpu, &tex, w, h, &laid);
            let back = gpu.colour().readback_linear_f32(&gpu, &out).unwrap();
            let worst = cpu
                .iter()
                .zip(&back)
                .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-2, "{name}: worst difference {worst}");
            assert!(cpu != src, "{name}: the reference drew something");
        }
    }
}
