#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Jolt's passes against their CPU oracle: the uneven blur, the scale and
//! slide under each Edges, the split, the colour, the light and Jolts only.
//! A frame with nothing hitting keeps the picture exactly.

use lumit_core::fx::effects::jolt::{jolt, JoltParams};
use lumit_gpu::fx::common::{readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::jolt::JoltOp;

/// A premultiplied picture with detail both ways and a soft alpha edge.
fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = if y < 4 { y as f32 / 4.0 } else { 1.0 };
            let check = ((x / 4 + y / 4) % 2) as f32;
            img[i] = (x as f32 / w as f32) * a;
            img[i + 1] = (0.2 + 0.6 * check) * a;
            img[i + 2] = (y as f32 / h as f32) * a;
            img[i + 3] = a;
        }
    }
    img
}

const STILL: JoltParams = JoltParams {
    blur: [0.0; 2],
    zoom: 1.0,
    offset: [0.0; 2],
    split: [0.0; 2],
    tint: [0.0; 4],
    gain: 1.0,
    opacity: 1.0,
    edge: 2,
    mix: 1.0,
};

#[test]
fn wgsl_jolt_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let tex = upload_linear_f32(&ctx, &picture(w, h), w, h);
    // The oracle reads what the kernel reads: the picture as half floats.
    let img = readback_linear_f32(&ctx, &tex, w, h).expect("readback");
    // Scaled by 1/0.8 about (20, 14), slid by (3.5, -2).
    let zoom = 0.8;
    let scaled = [20.0 - 20.0 * zoom - 3.5, 14.0 - 14.0 * zoom + 2.0];
    for (name, p) in [
        ("still", STILL),
        (
            "blur",
            JoltParams {
                blur: [5.0, 5.0],
                ..STILL
            },
        ),
        (
            "blur across",
            JoltParams {
                blur: [7.5, 0.0],
                ..STILL
            },
        ),
        (
            "blur down",
            JoltParams {
                blur: [0.0, 3.0],
                edge: 1,
                ..STILL
            },
        ),
        (
            "scale and slide",
            JoltParams {
                zoom,
                offset: scaled,
                ..STILL
            },
        ),
        (
            "transparent",
            JoltParams {
                zoom: 1.25,
                offset: [-4.0, 1.5],
                edge: 0,
                ..STILL
            },
        ),
        (
            "repeat",
            JoltParams {
                offset: [9.25, -6.5],
                edge: 1,
                ..STILL
            },
        ),
        (
            "split",
            JoltParams {
                offset: [2.0, 0.0],
                split: [3.0, 1.0],
                ..STILL
            },
        ),
        (
            "colour",
            JoltParams {
                tint: [0.9, 0.1, 0.4, 0.6],
                ..STILL
            },
        ),
        ("light", JoltParams { gain: 2.5, ..STILL }),
        (
            "jolts only",
            JoltParams {
                opacity: 0.35,
                ..STILL
            },
        ),
        (
            "everything",
            JoltParams {
                blur: [6.0, 2.0],
                zoom,
                offset: scaled,
                split: [-2.0, 1.5],
                tint: [0.2, 0.8, 0.9, 0.3],
                gain: 0.6,
                opacity: 0.8,
                edge: 2,
                mix: 0.7,
            },
        ),
    ] {
        let mut cpu = img.clone();
        jolt(&mut cpu, w, h, &p);
        let op = JoltOp {
            blur: p.blur,
            zoom: p.zoom,
            offset: p.offset,
            split: p.split,
            tint: p.tint,
            gain: p.gain,
            opacity: p.opacity,
            edge: p.edge,
            mix: p.mix,
        };
        let out = fx.jolt(&ctx, &tex, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).expect("readback");
        if name == "still" {
            assert_eq!(gpu, img, "{name}: must keep the picture exactly");
            assert_eq!(cpu, img, "{name}: must keep the picture exactly");
            continue;
        }
        // Half-float working textures between the passes.
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 4e-3, "{name}: off by {worst}");
        assert_ne!(cpu, img, "{name}: must change the picture");
    }
}
