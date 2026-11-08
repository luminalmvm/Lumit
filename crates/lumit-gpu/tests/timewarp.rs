#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Timewarp's kernels against their CPU oracle: the crossfade, and the frame
//! made from the motion between two moments, on a square that moves.

use lumit_core::fx::effects::timewarp::{timewarp_mix, timewarp_synth};
use lumit_gpu::fx::common::{readback_linear_f32, upload_flow_field, upload_linear_f32};
use lumit_gpu::fx::timewarp::TimewarpKernels;

/// A soft-edged square at `x0` on a ramp.
fn frame(w: usize, h: usize, x0: f32) -> Vec<f32> {
    let mut img = vec![0.0f32; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let inside = ((x as f32 - x0).abs() < 6.0 && (y as f32 - 12.0).abs() < 6.0) as u8;
            let g = x as f32 / w as f32;
            img[i] = if inside == 1 { 1.0 } else { g * 0.5 };
            img[i + 1] = if inside == 1 { 0.6 } else { 0.2 };
            img[i + 2] = 1.0 - g;
            img[i + 3] = 1.0;
        }
    }
    img
}

#[test]
fn wgsl_timewarp_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let kernels = TimewarpKernels::new(&ctx);
    let (w, h) = (48usize, 24usize);
    let ta = upload_linear_f32(&ctx, &frame(w, h, 14.0), w as u32, h as u32);
    let tb = upload_linear_f32(&ctx, &frame(w, h, 22.0), w as u32, h as u32);
    // The oracle reads what the kernels read: the pictures as half floats.
    let a = readback_linear_f32(&ctx, &ta, w as u32, h as u32).expect("readback");
    let b = readback_linear_f32(&ctx, &tb, w as u32, h as u32).expect("readback");

    for phase in [0.0f32, 0.3, 0.75] {
        let mut cpu = a.clone();
        timewarp_mix(&mut cpu, &b, phase);
        let out = kernels.mix(&ctx, &ta, &tb, phase);
        let gpu = readback_linear_f32(&ctx, &out, w as u32, h as u32).expect("readback");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(worst < 2e-3, "mix at {phase}: worst {worst}");
        if phase == 0.0 {
            assert_eq!(gpu, a, "mix at 0 keeps the first moment exactly");
        }
    }

    // The square moves 8 px right, measured at half size so 4 there;
    // trusted everywhere but a column, so both the warp and the fallback are
    // exercised.
    let (fw, fh) = (w / 2, h / 2);
    let n = fw * fh;
    let field = |u: f32| {
        let mut f = vec![0.0f32; n * 4];
        for i in 0..n {
            let x = i % fw;
            f[i * 4] = if (4..15).contains(&x) { u } else { 0.0 };
            f[i * 4 + 2] = if x == 20 { 0.0 } else { 1.0 };
        }
        f
    };
    let (fwd, bwd) = (field(4.0), field(-4.0));
    let plane = |f: &[f32], k: usize| (0..n).map(|i| f[i * 4 + k]).collect::<Vec<f32>>();
    let tex = |f: &[f32]| {
        upload_flow_field(
            &ctx,
            &plane(f, 0),
            &plane(f, 1),
            &plane(f, 2),
            fw as u32,
            fh as u32,
        )
    };
    let (tf, tw) = (tex(&fwd), tex(&bwd));
    for phase in [0.25f32, 0.5] {
        let cpu = timewarp_synth(&a, &b, &fwd, &bwd, (w, h), (fw, fh), phase);
        let out = kernels.synth(&ctx, &ta, &tb, &tf, &tw, phase);
        let gpu = readback_linear_f32(&ctx, &out, w as u32, h as u32).expect("readback");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(worst < 4e-3, "synth at {phase}: worst {worst}");
        // Half way, the square sits half way: its centre is where neither
        // moment had it.
        if phase == 0.5 {
            let at = |x: usize| gpu[(12 * w + x) * 4];
            assert!(
                at(18) > 0.9,
                "the square is at 18 half way, read {}",
                at(18)
            );
        }
    }
}
