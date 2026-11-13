#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Slices' kernel against its CPU oracle, both ways across, both sides, in
//! order and at random, with and without the smear. The ends keep the picture
//! exactly or leave nothing.

use lumit_core::fx::effects::slices::{slices, SlicesParams};
use lumit_gpu::fx::common::{readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::slices::SlicesOp;

/// A premultiplied picture with a gradient, a check, a value past 1 and a soft
/// alpha edge.
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

fn params(t: f32, vertical: bool, incoming: bool, random: bool, smear: f32) -> SlicesParams {
    SlicesParams {
        t,
        count: 7,
        vertical,
        alternate: true,
        random,
        stagger: 0.4,
        ease: 2,
        incoming,
        smear,
        seed: 11,
        mix: 1.0,
    }
}

#[test]
fn wgsl_slices_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (64u32, 36u32);
    let tex = upload_linear_f32(&ctx, &picture(w, h), w, h);
    // The oracle reads what the kernel reads: the picture as half floats.
    let img = readback_linear_f32(&ctx, &tex, w, h).expect("readback");
    let mut smooth = params(0.3, false, false, false, 0.0);
    smooth.ease = 1;
    smooth.count = 2;
    let mut mixed = params(0.5, false, true, true, 0.1);
    mixed.mix = 0.4;
    for (name, p) in [
        ("across", params(0.45, false, false, false, 0.075)),
        ("across incoming", params(0.45, false, true, false, 0.075)),
        ("down", params(0.6, true, false, false, 0.075)),
        ("down incoming random", params(0.6, true, true, true, 0.15)),
        ("random sharp", params(0.25, false, false, true, 0.0)),
        ("smooth two", smooth),
        ("mixed", mixed),
        ("start", params(0.0, false, false, true, 0.15)),
        ("start incoming", params(0.0, true, true, true, 0.15)),
        ("end incoming", params(1.0, false, true, false, 0.15)),
    ] {
        let mut cpu = img.clone();
        slices(&mut cpu, w, h, &p);
        let op = SlicesOp {
            t: p.t,
            count: p.count,
            vertical: p.vertical,
            alternate: p.alternate,
            random: p.random,
            stagger: p.stagger,
            ease: p.ease,
            incoming: p.incoming,
            smear: p.smear,
            seed: p.seed,
            mix: p.mix,
        };
        let out = fx.slices(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).expect("readback");
        match name {
            "start" | "end incoming" => {
                assert_eq!(cpu, img, "{name}: the oracle must keep the picture");
                assert_eq!(gpu, img, "{name}: the kernel must keep the picture");
                continue;
            }
            "start incoming" => {
                assert!(cpu.iter().all(|v| *v == 0.0), "{name}: must be empty");
                assert!(gpu.iter().all(|v| *v == 0.0), "{name}: must be empty");
                continue;
            }
            _ => {}
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
