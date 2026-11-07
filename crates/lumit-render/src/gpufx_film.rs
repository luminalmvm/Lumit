//! Film's GPU pass, kept beside the dispatch table rather than in it.

use lumit_core::fx::effects::film::{Film as Declared, FilmFrame};
use lumit_core::fx::film::{tables, Spread, Stage};
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_gpu::fx::film::{FilmOp, FilmStageOp};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::gpufx::{glow_shape, AuxSlot, GpuEffect};

pub(crate) struct Film;

/// A stage in the blurs Glow runs: a gaussian is Glow's at Falloff 0 with σ
/// half its radius.
fn stage(s: &Stage, w: u32, h: u32) -> FilmStageOp {
    FilmStageOp {
        keep: s.keep,
        terms: s
            .terms
            .iter()
            .map(|(how, weight)| {
                let shape = match *how {
                    Spread::Gauss(sigma) => glow_shape(2.0 * sigma, 0.0, w, h),
                    Spread::Exp { radius_px, falloff } => glow_shape(radius_px, falloff, w, h),
                };
                (shape, *weight)
            })
            .collect(),
    }
}

fn op<'a>(f: &'a FilmFrame, w: u32, h: u32) -> FilmOp<'a> {
    let k = &f.k;
    let mut grain_blur = Stage {
        keep: [0.0; 3],
        terms: Vec::new(),
    };
    grain_blur.gauss(f.grain_blur, [1.0; 3]);
    FilmOp {
        camera: k.camera,
        boost: k.boost,
        inverse: k.inverse,
        couplers: k.couplers,
        to_rec709: k.to_rec709,
        couplers_on: k.couplers_on,
        shown: k.shown as u32,
        positive: k.positive,
        gamma: k.gamma,
        top: k.top,
        grey: k.grey,
        slope: k.slope,
        grain: k.grain,
        grain_colour: k.grain_colour,
        seed: k.seed,
        tick: k.tick,
        print_gain: k.print_gain,
        print_gamma: k.print_gamma,
        scan_low: k.scan_low,
        scan_top: k.scan_top,
        weave: k.weave,
        mix: k.mix,
        bloom: stage(&f.bloom, w, h),
        scatter: stage(&f.scatter, w, h),
        halation: stage(&f.halation, w, h),
        held: stage(&f.couplers, w, h),
        grain_blur: stage(&grain_blur, w, h),
        tables: &f.tables,
        at: [
            tables::AT_CURVE,
            tables::AT_CURVE0,
            tables::AT_PRINT_CURVE,
            tables::AT_GRAIN,
            tables::AT_LUT_PRINT,
            tables::AT_LUT_SCAN,
        ]
        .map(|v| v as u32),
        sizes: [tables::CURVE, tables::GRAIN, tables::LUT].map(|v| v as u32),
        floor: tables::DENSITY_FLOOR,
    }
}

impl GpuEffect for Film {
    fn match_name(&self) -> &'static str {
        "film"
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
        match Declared::read(p).frame(p, w, h) {
            Some(frame) => fx.film(ctx, tex, w, h, &op(&frame, w, h)),
            // Only a broken stock file, which a test rules out: the picture as
            // it came.
            None => fx.blur(
                ctx,
                tex,
                w,
                h,
                None,
                &lumit_gpu::fx::blur::BlurOp {
                    radius_px: 0.0,
                    edge: 1,
                    mix: 0.0,
                },
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::fx::params::Params;

    /// A grey ramp with a few lights far brighter than white, so halation,
    /// bloom and the stock's shoulder all have something to do.
    fn picture(w: u32, h: u32) -> Vec<f32> {
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let t = x as f32 / w as f32;
                let lit = (x % 31 == 7 && y % 23 == 11) as u8 as f32 * 40.0;
                img[i] = 0.02 + t * 0.6 + lit;
                img[i + 1] = 0.02 + (y as f32 / h as f32) * 0.4 + lit * 0.8;
                img[i + 2] = 0.05 + (1.0 - t) * 0.3 + lit * 0.5;
                img[i + 3] = 1.0;
            }
        }
        img
    }

    /// **The Viewer's Film is the export's**: every part, on its own and
    /// together, lands within the working format's rounding of the CPU twin.
    #[test]
    fn wgsl_film_matches_the_cpu_oracle() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = ctx.fx();
        let (w, h) = (96u32, 64u32);
        let img = picture(w, h);
        let tex = lumit_gpu::fx::common::upload_linear_f32(&ctx, &img, w, h);
        let base = Declared::read(Params::EMPTY);
        let variants: Vec<(&str, Declared)> = vec![
            ("default", base),
            (
                "bloom and weave",
                Declared {
                    bloom: true,
                    weave: true,
                    mix: 60.0,
                    ..base
                },
            ),
            (
                "scan",
                Declared {
                    print: false,
                    format: 0,
                    ..base
                },
            ),
            ("slide", Declared { stock: 16, ..base }),
            (
                "own colour",
                Declared {
                    colour: false,
                    backing: 3,
                    ..base
                },
            ),
            (
                "paper",
                Declared {
                    stock: 5,
                    print_stock: 3,
                    red: 4,
                    push: 1.0,
                    contrast: 130.0,
                    highlights: 2.0,
                    ..base
                },
            ),
        ];
        for (name, film) in variants {
            let frame = film.frame(Params::EMPTY, w, h).expect("the stocks load");
            let mut cpu = img.clone();
            lumit_core::fx::film::cpu::film(&mut cpu, w, h, &frame);
            let out = fx.film(&ctx, &tex, w, h, &op(&frame, w, h));
            let gpu =
                lumit_gpu::fx::common::readback_linear_f32(&ctx, &out, w, h).expect("readback");
            let worst = cpu
                .iter()
                .zip(&gpu)
                .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
                .fold(0.0f32, f32::max);
            eprintln!("film {name}: worst relative |Δ| {worst:.3e}");
            assert!(worst < 1e-2, "{name}: worst |Δ| {worst}");
        }
    }
}
