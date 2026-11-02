#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Luma fade's kernel against its CPU oracle, on the layer's own picture and
//! through a matte. Completion 0 and Mix 0 keep the picture exactly, and 100
//! leaves nothing.

use lumit_core::fx::effects::luma_fade::{luma_fade, LumaFadeParams};
use lumit_gpu::fx::common::{readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::utility::LumaFadeOp;

/// A premultiplied picture with every brightness in it and a soft alpha edge.
fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = x as f32 / (w - 1) as f32;
            let a = if y < 4 { y as f32 / 4.0 } else { 1.0 };
            img[i] = g * g * a;
            img[i + 1] = g * a;
            img[i + 2] = (1.0 - g) * 0.3 * a;
            img[i + 3] = a;
        }
    }
    img
}

fn params(completion: f32, band: f32, invert: bool, mix: f32) -> LumaFadeParams {
    LumaFadeParams {
        completion,
        band,
        invert,
        mix,
    }
}

#[test]
fn wgsl_luma_fade_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (40u32, 24u32);
    let img = picture(w, h);
    // The matte runs the other way, top to bottom.
    let mut matte = vec![0.0f32; img.len()];
    for (i, px) in matte.chunks_exact_mut(4).enumerate() {
        let v = (i as u32 / w) as f32 / (h - 1) as f32;
        px.copy_from_slice(&[v, v, v, 1.0]);
    }
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let matte_tex = upload_linear_f32(&ctx, &matte, w, h);
    // The oracle reads what the kernel reads: both pictures as half floats.
    let img = readback_linear_f32(&ctx, &tex, w, h).expect("readback");
    let matte = readback_linear_f32(&ctx, &matte_tex, w, h).expect("readback");
    for (name, p, with_matte) in [
        ("hard", params(0.5, 1e-3, false, 1.0), false),
        ("soft", params(0.5, 0.3, false, 1.0), false),
        ("inverted", params(0.4, 0.1, true, 1.0), false),
        ("mixed", params(0.6, 0.2, false, 0.6), false),
        ("matte", params(0.5, 0.2, false, 1.0), true),
        ("matte inverted", params(0.3, 0.05, true, 1.0), true),
        ("none", params(0.0, 0.2, false, 1.0), false),
        ("all", params(1.0, 0.2, true, 1.0), true),
        ("mix zero", params(0.5, 1e-3, false, 0.0), false),
    ] {
        let mut cpu = img.clone();
        luma_fade(&mut cpu, &p, if with_matte { &matte } else { &[] });
        let op = LumaFadeOp {
            completion: p.completion,
            band: p.band,
            invert: p.invert,
            mix: p.mix,
        };
        let m = with_matte.then_some(&matte_tex);
        let out = fx.luma_fade(&ctx, &tex, w, h, m, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).expect("readback");
        // The working texture is half float; a hard edge may land a pixel
        // either side of it, so compare the soft cases tightly and count the
        // hard ones.
        let off = cpu
            .iter()
            .zip(&gpu)
            .filter(|(a, b)| (*a - *b).abs() > 2e-3)
            .count();
        assert!(off <= 4, "{name}: {off} values differ");
        match name {
            "none" | "mix zero" => assert_eq!(gpu, img, "{name}: must keep the picture exactly"),
            "all" => assert!(gpu.iter().all(|v| *v == 0.0), "{name}: must leave nothing"),
            _ => {}
        }
    }
}
