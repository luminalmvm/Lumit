#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Crop's kernel against its CPU oracle, hard-edged and feathered.

use lumit_core::fx::effects::crop::{crop, CropParams};
use lumit_gpu::fx::common::{readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::utility::CropOp;

fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = (x + y) as f32 / (w + h) as f32;
            img[i] = g;
            img[i + 1] = 1.0 - g;
            img[i + 2] = 0.25;
            img[i + 3] = 1.0;
        }
    }
    img
}

#[test]
fn wgsl_crop_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (40u32, 30u32);
    let img = picture(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    for (name, p) in [
        (
            "none",
            CropParams {
                edges: [0.0; 4],
                band: 1e-3,
                mix: 1.0,
            },
        ),
        (
            "hard",
            CropParams {
                edges: [0.1, 0.2, 0.25, 0.05],
                band: 1e-3,
                mix: 1.0,
            },
        ),
        (
            "feathered",
            CropParams {
                edges: [0.3, 0.1, 0.1, 0.3],
                band: 6.0,
                mix: 1.0,
            },
        ),
        (
            "half mix",
            CropParams {
                edges: [0.2, 0.2, 0.2, 0.2],
                band: 2.0,
                mix: 0.5,
            },
        ),
    ] {
        let mut cpu = img.clone();
        crop(&mut cpu, w, h, &p);
        let op = CropOp {
            edges: p.edges,
            band: p.band,
            mix: p.mix,
        };
        let out = fx.crop(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).expect("readback");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        // The working texture is half float.
        assert!(worst < 2e-3, "{name}: worst difference {worst}");
    }
}
