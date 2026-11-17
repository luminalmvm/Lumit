#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Light leak's kernel against its CPU oracle, across angles, sizes, times
//! and a strength past 100. Mix 0 keeps the picture exactly.

use lumit_core::fx::effects::light_leak::{light_leak, LightLeakParams};
use lumit_gpu::fx::common::{readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::light_leak::LightLeakOp;

/// A premultiplied picture with dark and bright parts, a value past 1 and a
/// soft alpha edge.
fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = if y < 4 { y as f32 / 4.0 } else { 1.0 };
            let check = ((x / 4 + y / 4) % 2) as f32;
            img[i] = (x as f32 / w as f32) * 1.5 * a;
            img[i + 1] = (0.1 + 0.6 * check) * a;
            img[i + 2] = (y as f32 / h as f32) * 0.4 * a;
            img[i + 3] = a;
        }
    }
    img
}

fn params(w: u32, h: u32, angle: f32, size: f32, z: f32, amount: f32, mix: f32) -> LightLeakParams {
    let (sin, cos) = angle.to_radians().sin_cos();
    LightLeakParams {
        normal: [cos / size, sin / size],
        centre: [w as f32 * 0.5, h as f32 * 0.5],
        inv_h: 1.0 / h as f32,
        z: [z, z * 1.7],
        colour: [amount, 0.45 * amount, 0.12 * amount],
        colour_2: [amount, 0.1 * amount, 0.25 * amount],
        seed: 7,
        mix,
    }
}

#[test]
fn wgsl_light_leak_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (64u32, 36u32);
    let tex = upload_linear_f32(&ctx, &picture(w, h), w, h);
    // The oracle reads what the kernel reads: the picture as half floats.
    let img = readback_linear_f32(&ctx, &tex, w, h).expect("readback");
    for (name, p) in [
        ("default", params(w, h, 20.0, 0.8, 0.0, 0.6, 1.0)),
        ("narrow", params(w, h, 0.0, 0.2, 1.25, 1.0, 1.0)),
        ("turned", params(w, h, 115.0, 0.5, 3.7, 0.8, 1.0)),
        ("strong", params(w, h, -40.0, 1.4, 12.5, 2.0, 1.0)),
        ("mixed", params(w, h, 20.0, 0.8, 0.4, 0.6, 0.4)),
        ("mix zero", params(w, h, 20.0, 0.8, 0.0, 0.6, 0.0)),
    ] {
        let mut cpu = img.clone();
        light_leak(&mut cpu, w, &p);
        let op = LightLeakOp {
            normal: p.normal,
            centre: p.centre,
            inv_h: p.inv_h,
            z: p.z,
            colour: p.colour,
            colour_2: p.colour_2,
            seed: p.seed,
            mix: p.mix,
        };
        let out = fx.light_leak(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).expect("readback");
        if name == "mix zero" {
            assert_eq!(gpu, img, "{name}: must keep the picture exactly");
            assert_eq!(cpu, img, "{name}: must keep the picture exactly");
            continue;
        }
        // The output is half float, so the error grows with the value.
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs() / a.abs().max(1.0))
            .fold(0.0f32, f32::max);
        assert!(worst <= 2e-3, "{name}: off by {worst}");
        assert_ne!(cpu, img, "{name}: must change the picture");
    }
}
