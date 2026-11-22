#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Flicker's kernel against its CPU oracle, across light, tint, contrast and
//! opacity, and a dip deep enough to clamp. Mix 0 keeps the picture exactly.

use lumit_core::fx::effects::flicker::{flicker, FlickerParams};
use lumit_gpu::fx::common::{readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::flicker::FlickerOp;

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
            img[i + 1] = (0.05 + 0.6 * check) * a;
            img[i + 2] = (y as f32 / h as f32) * 0.4 * a;
            img[i + 3] = a;
        }
    }
    img
}

fn params(gain: [f32; 3], contrast: f32, opacity: f32, mix: f32) -> FlickerParams {
    FlickerParams {
        gain,
        contrast,
        opacity,
        mix,
    }
}

#[test]
fn wgsl_flicker_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 24u32);
    let tex = upload_linear_f32(&ctx, &picture(w, h), w, h);
    // The oracle reads what the kernel reads: the picture as half floats.
    let img = readback_linear_f32(&ctx, &tex, w, h).expect("readback");
    for (name, p) in [
        ("dip", params([0.4; 3], 1.0, 1.0, 1.0)),
        ("flash", params([1.7; 3], 1.0, 1.0, 1.0)),
        ("tint", params([1.2, 0.8, 1.05], 1.0, 1.0, 1.0)),
        ("contrast", params([1.0; 3], 2.5, 1.0, 1.0)),
        ("flat", params([0.9; 3], 0.3, 1.0, 1.0)),
        ("opacity", params([1.1; 3], 1.2, 0.35, 1.0)),
        ("mixed", params([0.5, 0.7, 0.6], 1.6, 0.8, 0.4)),
        ("mix zero", params([0.5; 3], 2.0, 0.5, 0.0)),
    ] {
        let mut cpu = img.clone();
        flicker(&mut cpu, &p);
        let op = FlickerOp {
            gain: p.gain,
            contrast: p.contrast,
            opacity: p.opacity,
            mix: p.mix,
        };
        let out = fx.flicker(&ctx, &tex, w, h, &op);
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
