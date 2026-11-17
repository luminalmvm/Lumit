//! Denoise's kernels against `lumit_core::fx::denoise`, the whole pipeline at
//! once: into Y'CbCr, the neighbours moved along a field and averaged in, the
//! four bands shrunk, Grain and back.

use super::common::*;
use super::denoise::*;
use lumit_core::fx::denoise::{self as cpu, temporal::Field, DenoiseParams};

/// A small deterministic noise, -0.5..0.5.
fn noise(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x as f32 / u32::MAX as f32) - 0.5
}

/// A picture with edges, a ramp, a soft alpha edge and noise, at fp16, its
/// content moved `shift` pixels, with noise seeded by `seed`.
fn picture(w: u32, h: u32, shift: [f32; 2], seed: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x as f32 - shift[0], y as f32 - shift[1]);
            let base = 0.2 + 0.3 * (sx * 0.31).sin() * (sy * 0.17).cos() + 0.01 * sx;
            let a = if x as f32 > w as f32 * 0.8 { 0.5 } else { 1.0 };
            let i = seed.wrapping_mul(100_003).wrapping_add(y * w + x) * 3;
            let px = [
                (base + 0.03 * noise(i)).max(0.0),
                (base * 0.8 + 0.03 * noise(i + 1)).max(0.0),
                (0.25 + 0.03 * noise(i + 2)).max(0.0),
            ];
            out.extend_from_slice(&[px[0] * a, px[1] * a, px[2] * a, a]);
        }
    }
    out.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

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

fn params(frames: u32, view: u32) -> DenoiseParams {
    DenoiseParams {
        view,
        luma_sigma: 6.0 / 255.0,
        chroma_sigma: 5.0 / 255.0,
        tone: [1.2, 1.0, 0.8],
        luma_bands: [1.0, 0.6, 0.35, 0.2],
        chroma_bands: [1.0, 0.8, 0.6, 0.45],
        frames,
        motion: true,
        t0: 1.5,
        t1: 2.5,
        strength: 1.0,
        luma_amount: 0.7,
        chroma_amount: 1.0,
        band_amounts: [1.0, 1.0, 1.0, 0.5],
        detail: 0.25,
        sharpen: 0.5,
        grain: 0.3,
        mix: 0.9,
    }
}

/// The per-frame shift of the content, in picture pixels.
const STEP: [f32; 2] = [1.5, -0.75];

#[test]
fn wgsl_denoise_matches_the_cpu_oracle_and_is_bit_stable() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let kernels = DenoiseKernels::new(&ctx);
    let (w, h) = (48u32, 40u32);
    let (fw, fh) = (w / 2, h / 2);
    let current = picture(w, h, [0.0, 0.0], 0);
    let cur_t = upload_linear_f32(&ctx, &current, w, h);
    // Four neighbours, each the content moved on by its offset, and a field
    // at half size that says so, trusted a little less towards the right.
    let offsets = [-2i32, -1, 1, 2];
    let frames: Vec<Vec<f32>> = offsets
        .iter()
        .map(|&o| {
            picture(
                w,
                h,
                [STEP[0] * o as f32, STEP[1] * o as f32],
                (7 + o) as u32,
            )
        })
        .collect();
    let fields: Vec<(Vec<f32>, Vec<f32>, Vec<f32>)> = offsets
        .iter()
        .map(|&o| {
            let n = (fw * fh) as usize;
            let u = vec![STEP[0] * o as f32 * 0.5; n];
            let v = vec![STEP[1] * o as f32 * 0.5; n];
            let conf = (0..n)
                .map(|i| 1.0 - 0.3 * (i as u32 % fw) as f32 / fw as f32)
                .collect();
            (u, v, conf)
        })
        .collect();
    let pictures: Vec<wgpu::Texture> = frames
        .iter()
        .map(|f| upload_linear_f32(&ctx, f, w, h))
        .collect();
    let field_textures: Vec<wgpu::Texture> = fields
        .iter()
        .map(|(u, v, c)| upload_flow_field(&ctx, u, v, c, fw, fh))
        .collect();
    let aligned: Vec<Vec<f32>> = frames
        .iter()
        .zip(&fields)
        .map(|(f, (u, v, conf))| {
            cpu::temporal::warp(
                f,
                w,
                h,
                Some(Field {
                    u,
                    v,
                    conf,
                    w: fw,
                    h: fh,
                }),
            )
        })
        .collect();
    let aligned_refs: Vec<&[f32]> = aligned.iter().map(Vec::as_slice).collect();
    let neighbours: Vec<Neighbour<'_>> = pictures
        .iter()
        .zip(&field_textures)
        .map(|(picture, field)| Neighbour {
            picture,
            field: Some(field),
        })
        .collect();
    // Follow motion off: every neighbour read where it is.
    let still: Vec<Vec<f32>> = frames
        .iter()
        .map(|f| cpu::temporal::warp(f, w, h, None))
        .collect();
    let still_refs: Vec<&[f32]> = still.iter().map(Vec::as_slice).collect();
    let in_place: Vec<Neighbour<'_>> = pictures
        .iter()
        .map(|picture| Neighbour {
            picture,
            field: None,
        })
        .collect();
    // A matte that runs from black on the left to white on the right, with a
    // tint so each channel's weight counts.
    let matte: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let t = (i % w) as f32 / (w - 1) as f32;
            [t, t * 0.9, (t * 1.2).min(1.0), 1.0]
        })
        .collect();
    let matte_t = upload_linear_f32(&ctx, &matte, w, h);

    // (frames, view, motion, matte)
    let cases = [
        (0u32, 0u32, true, false),
        (2, 0, true, false),
        (2, 1, true, false),
        (2, 2, true, false),
        (2, 0, false, false),
        (2, 0, true, true),
        (2, 1, true, true),
    ];
    for (frames, view, motion, matted) in cases {
        let p = params(frames, view);
        let (used, given): (&[&[f32]], &[Neighbour<'_>]) = match (frames, motion) {
            (0, _) => (&[], &[]),
            (_, true) => (&aligned_refs, &neighbours),
            (_, false) => (&still_refs, &in_place),
        };
        let want = cpu::run(&current, w, h, used, &p, if matted { &matte } else { &[] });
        let run = || {
            let m = matted.then_some(&matte_t);
            let out = kernels.run(&ctx, &cur_t, w, h, given, &op_of(&p), m);
            readback_linear_f32(&ctx, &out, w, h).unwrap()
        };
        let got = run();
        let worst = want
            .iter()
            .zip(&got)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        let mean = want
            .iter()
            .zip(&got)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / want.len() as f32;
        let case = format!("frames {frames} view {view} motion {motion} matte {matted}");
        // The Removed noise view draws the difference four times over.
        let gain = if view == 1 { 4.0 } else { 1.0 };
        assert!(
            worst < 1.5e-2 * gain && mean < 1e-3 * gain,
            "{case}: worst {worst}, mean {mean}"
        );
        // The same frame twice is the same bits.
        let again = run();
        assert!(
            got.iter()
                .zip(&again)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{case} is not bit-stable"
        );
    }
}
