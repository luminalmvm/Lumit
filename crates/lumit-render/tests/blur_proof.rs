//! Visual proof for the K-390 Guertin-class motion blur
//! (docs/impl/optical-flow.md §4.5 item 3): render the same real frame three
//! ways — untouched, the way v1 blurred it, and the way v2 does — so the two
//! claims can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The numbers in the oracle tests prove the GPU and the CPU agree. They cannot
//! prove the *look* is better, and the two things this stage changed are both
//! things you have to see:
//!
//! 1. **Scatter.** v1 smeared each pixel along its own motion, so a fast object
//!    never spilled over the still background it passed. Compare the leading
//!    edge of anything moving in the v1 and v2 pictures.
//! 2. **Graceful low confidence.** v1 shortened the streak by confidence, so
//!    pixels the flow could not vouch for came out sharp in the middle of a
//!    blurred frame. In v2 they borrow their neighbourhood's motion instead.
//!    The confidence view shows where those places are; compare them across the
//!    two blurred pictures.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_BLUR_PROOF_CLIPS="C:/tmp/lumit-flow-clips/gameplay-pov.mp4" \
//! LUMIT_BLUR_PROOF_OUT="C:/tmp/lumit-flow-clips" \
//!   cargo test -p lumit-render --release --test blur_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_BLUR_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`) rather than PNG: nothing in the workspace encodes PNG,
//! and a throwaway encoder written for a diagnostic is exactly the code that
//! should not exist. The runner converts them.

use lumit_core::fx::{cpu, MbQuality, MbView};

/// v1's blur, kept only here and only to be the "before" picture: each pixel
/// smeared along **its own** vector, the streak scaled to nothing by low
/// confidence, a fixed tap count and a plain box weight. This is the behaviour
/// K-390 replaced; it is not reachable from the shipping engine any more.
#[allow(clippy::too_many_arguments)] // the shape v1 had; changing it would not be v1
fn motion_blur_v1(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    u: &[f32],
    v: &[f32],
    conf: &[f32],
    shutter_frac: f32,
    samples: i32,
) {
    let original = rgba.to_vec();
    let n = samples.max(1);
    let nf = n as f32;
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let c = conf[idx];
            let sv = (u[idx] * shutter_frac * c, v[idx] * shutter_frac * c);
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let mut acc = [0.0f32; 4];
            for k in 0..n {
                let t = (k as f32 + 0.5) / nf - 0.5;
                let s = bilinear(&original, w, h, pos.0 + t * sv.0, pos.1 + t * sv.1);
                for cc in 0..4 {
                    acc[cc] += s[cc];
                }
            }
            let i = idx * 4;
            for cc in 0..4 {
                rgba[i + cc] = acc[cc] / nf;
            }
        }
    }
}

/// The clamp-addressed bilinear `cpu::motion_blur` uses, duplicated because it
/// is private to that module and this file is a diagnostic, not a second engine.
fn bilinear(a: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> [f32; 4] {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let at = |cx: i32, cy: i32| {
        let cx = cx.clamp(0, w as i32 - 1) as u32;
        let cy = cy.clamp(0, h as i32 - 1) as u32;
        let i = ((cy * w + cx) * 4) as usize;
        [a[i], a[i + 1], a[i + 2], a[i + 3]]
    };
    let (xi, yi) = (x0 as i32, y0 as i32);
    let (c00, c10, c01, c11) = (
        at(xi, yi),
        at(xi + 1, yi),
        at(xi, yi + 1),
        at(xi + 1, yi + 1),
    );
    let mut out = [0.0f32; 4];
    for c in 0..4 {
        let top = c00[c] * (1.0 - tx) + c10[c] * tx;
        let bottom = c01[c] * (1.0 - tx) + c11[c] * tx;
        out[c] = top * (1.0 - ty) + bottom * ty;
    }
    out
}

fn to_linear(rgba: &[u8]) -> Vec<f32> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            [
                lumit_core::pixels::srgb_decode(p[0]),
                lumit_core::pixels::srgb_decode(p[1]),
                lumit_core::pixels::srgb_decode(p[2]),
                f32::from(p[3]) / 255.0,
            ]
        })
        .collect()
}

fn to_srgb(lin: &[f32]) -> Vec<u8> {
    lin.chunks_exact(4)
        .flat_map(|p| {
            [
                lumit_core::pixels::srgb_encode(p[0]),
                lumit_core::pixels::srgb_encode(p[1]),
                lumit_core::pixels::srgb_encode(p[2]),
                (p[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]
        })
        .collect()
}

#[test]
#[ignore = "harness: set LUMIT_BLUR_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_before_and_after_pictures() {
    let Ok(clips) = std::env::var("LUMIT_BLUR_PROOF_CLIPS") else {
        eprintln!("set LUMIT_BLUR_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_BLUR_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-flow-clips".to_owned());
    let frame: usize = std::env::var("LUMIT_BLUR_PROOF_FRAME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // 180 degrees, the film-standard default, and the schema's default tap cap.
    let (shutter_frac, samples) = (0.5f32, 32i32);

    for clip in clips.split(';').filter(|s| !s.trim().is_empty()) {
        let path = std::path::Path::new(clip.trim());
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let Ok(index) = lumit_media::index::build_frame_index(path) else {
            eprintln!("{stem}: could not index");
            continue;
        };
        let Ok(mut dec) = lumit_media::decode::VideoDecoder::open(path, index) else {
            eprintln!("{stem}: could not open");
            continue;
        };
        let (Ok(a), Ok(b)) = (dec.frame_rgba(frame, None), dec.frame_rgba(frame + 1, None)) else {
            eprintln!("{stem}: could not decode frames {frame} and {}", frame + 1);
            continue;
        };
        let (w, h) = (a.width, a.height);
        let (wu, hu) = (w as usize, h as usize);

        // Exactly what the decode worker measures for this effect (decode.rs):
        // half resolution, engine defaults, the forward pair plus confidence,
        // scaled back to the frame's own size.
        let set = lumit_flow::FlowSettings {
            divisor: 2,
            ..lumit_flow::FlowSettings::default()
        };
        let (ga, gb, _) = lumit_flow::flow_grays(&a.rgba, &b.rgba, wu, hu, &set);
        let (fwd, bwd) = lumit_flow::FlowEngine::new_auto().flow_pair_with(&ga, &gb, &set);
        let conf_half = lumit_flow::confidence(&fwd, &bwd);
        let (u, v, conf) = lumit_flow::field_to_size(&fwd, &conf_half, wu, hu);

        let lin = to_linear(&a.rgba);
        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.f{frame}.{tag}.{w}x{h}.raw");
            if let Err(e) = std::fs::write(&name, px) {
                eprintln!("could not write {name}: {e}");
            } else {
                eprintln!("wrote {name}");
            }
        };
        write("0-source", &a.rgba);

        let mut v1 = lin.clone();
        motion_blur_v1(&mut v1, w, h, &u, &v, &conf, shutter_frac, samples);
        write("1-v1", &to_srgb(&v1));

        for (tag, quality) in [
            ("2-v2-normal", MbQuality::Normal),
            ("3-v2-high", MbQuality::High),
        ] {
            let mut px = lin.clone();
            cpu::motion_blur(
                &mut px,
                w,
                h,
                &u,
                &v,
                &conf,
                shutter_frac,
                samples,
                1.0,
                MbView::Rendered,
                quality,
            );
            write(tag, &to_srgb(&px));
        }

        // The diagnostics that say *where* to look: confidence marks the places
        // v1 froze, and the dominant-motion view shows what v2 hands them.
        for (tag, view) in [
            ("4-confidence", MbView::Confidence),
            ("5-dominant", MbView::TileMax),
        ] {
            let mut px = lin.clone();
            cpu::motion_blur(
                &mut px,
                w,
                h,
                &u,
                &v,
                &conf,
                shutter_frac,
                samples,
                1.0,
                view,
                MbQuality::Normal,
            );
            write(tag, &to_srgb(&px));
        }

        // A one-line summary of the field, so a picture that looks wrong can be
        // told apart from a clip that simply is not moving.
        let n = u.len().max(1);
        let mean_speed: f64 = u
            .iter()
            .zip(&v)
            .map(|(a, b)| f64::from((a * a + b * b).sqrt()))
            .sum::<f64>()
            / n as f64;
        let mean_conf: f64 = conf.iter().map(|c| f64::from(*c)).sum::<f64>() / n as f64;
        let low = conf.iter().filter(|c| **c < 0.5).count() as f64 / n as f64;
        eprintln!(
            "{stem} f{frame}: {w}x{h}, mean speed {mean_speed:.2} px/frame, \
             mean confidence {mean_conf:.3}, {:.1}% below 0.5",
            low * 100.0
        );
    }
}

/// What a frame of flow and a frame of blur cost on real footage at each tier,
/// printed. Not a gate — the number is whatever the machine running it can do —
/// so it is `#[ignore]`d and run by hand, the same discipline as
/// `lens_flare_frame_cost`:
///
/// ```text
/// LUMIT_BLUR_PROOF_CLIPS="C:/tmp/lumit-flow-clips/gameplay-pov.mp4" \
///   cargo test -p lumit-render --release --test blur_proof -- \
///   flow_and_blur_frame_cost --ignored --nocapture
/// ```
///
/// Flow is measured the way the decode worker measures it (half res, K-390's
/// census cost), blur at the frame's own size. GPU work is only finished when
/// something reads it back, so each blur figure is timed with a readback and
/// the readback's own cost — measured on the same texture immediately after —
/// is subtracted, leaving the kernel.
#[test]
#[ignore = "a measurement, not a gate: prints times, asserts nothing"]
fn flow_and_blur_frame_cost() {
    use lumit_core::retime::VectorDetail;
    use lumit_gpu::fx::{readback_linear_f32, upload_flow_field, upload_linear_f32, MotionBlurOp};
    use std::time::Instant;

    let Ok(clips) = std::env::var("LUMIT_BLUR_PROOF_CLIPS") else {
        eprintln!("set LUMIT_BLUR_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let frame: usize = std::env::var("LUMIT_BLUR_PROOF_FRAME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
        eprintln!("no GPU adapter; nothing to measure");
        return;
    };
    let fx = lumit_gpu::fx::FxEngine::new(&ctx);
    let runs = 5u32;

    for clip in clips.split(';').filter(|s| !s.trim().is_empty()) {
        let path = std::path::Path::new(clip.trim());
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let Ok(index) = lumit_media::index::build_frame_index(path) else {
            eprintln!("{stem}: could not index");
            continue;
        };
        let Ok(mut dec) = lumit_media::decode::VideoDecoder::open(path, index) else {
            eprintln!("{stem}: could not open");
            continue;
        };
        let (Ok(a), Ok(b)) = (dec.frame_rgba(frame, None), dec.frame_rgba(frame + 1, None)) else {
            eprintln!("{stem}: could not decode frames {frame} and {}", frame + 1);
            continue;
        };
        // `LUMIT_COST_SCALE=2` pixel-doubles the pair before measuring, which is
        // how a 4k figure comes out of a set that has no 4k clip in it (the
        // "4k" cinematic is a 1920×816 export of a game rendered at 4k). Cost
        // here is per-pixel and per-pyramid-level, so a doubled frame costs what
        // a real one of that size costs; only the *content* is not 4k detail.
        let scale: u32 = std::env::var("LUMIT_COST_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
            .max(1);
        let grow = |f: &lumit_media::decode::DecodedFrame| -> (Vec<u8>, u32, u32) {
            let (w, h) = (f.width * scale, f.height * scale);
            let mut out = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let s = ((y / scale * f.width + x / scale) * 4) as usize;
                    let d = ((y * w + x) * 4) as usize;
                    out[d..d + 4].copy_from_slice(&f.rgba[s..s + 4]);
                }
            }
            (out, w, h)
        };
        let (a_rgba, w, h) = if scale > 1 {
            grow(&a)
        } else {
            (a.rgba.clone(), a.width, a.height)
        };
        let (b_rgba, _, _) = if scale > 1 {
            grow(&b)
        } else {
            (b.rgba.clone(), b.width, b.height)
        };
        let (a, b) = (a_rgba, b_rgba);
        let (wu, hu) = (w as usize, h as usize);

        // --- flow, per Vector detail tier (Medium is the shipped default) ---
        let mut last = None;
        for (tier, detail) in [
            ("Normal (Medium)", VectorDetail::Medium),
            ("High", VectorDetail::High),
        ] {
            let set = lumit_flow::FlowSettings {
                divisor: 2,
                iterations: detail.iterations(),
                min_level_dim: detail.min_level_dim(),
                refine_iters: detail.refine_iters(),
                ..lumit_flow::FlowSettings::default()
            };
            let (ga, gb, _) = lumit_flow::flow_grays(&a, &b, wu, hu, &set);
            let mut eng = lumit_flow::FlowEngine::new_auto();
            let warm = eng.flow_pair_with(&ga, &gb, &set); // shaders, plan, buffers
            let started = Instant::now();
            for _ in 0..runs {
                let _ = eng.flow_pair_with(&ga, &gb, &set);
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            eprintln!(
                "{stem} {w}x{h}  flow {tier:<16} {each:8.2} ms/frame  ({})",
                eng.backend()
            );
            last = Some((warm, set));
        }

        // --- blur, per Quality tier, on the field the effect actually gets ---
        let Some(((fwd, bwd), _set)) = last else {
            continue;
        };
        let conf_half = lumit_flow::confidence(&fwd, &bwd);
        let (u, v, conf) = lumit_flow::field_to_size(&fwd, &conf_half, wu, hu);
        let src = upload_linear_f32(&ctx, &to_linear(&a), w, h);
        let flow_t = upload_flow_field(&ctx, &u, &v, &conf, w, h);
        for (tier, quality) in [("Normal", MbQuality::Normal), ("High", MbQuality::High)] {
            let op = MotionBlurOp {
                shutter_frac: 0.5,
                samples: 32,
                mix: 1.0,
                view: MbView::Rendered.code(),
                quality: quality.code(),
            };
            // Waiting on the device — not reading the picture back — is what
            // isolates the kernel: a 4k readback costs an order of magnitude
            // more than the blur, so timing one against the other measures the
            // copy and calls it the shader.
            let warm = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
            drop(readback_linear_f32(&ctx, &warm, w, h)); // shaders, pipeline
            let blurs = 20u32;
            let started = Instant::now();
            for _ in 0..blurs {
                let _ = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
                ctx.device.poll(wgpu::Maintain::Wait);
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(blurs);
            eprintln!("{stem} {w}x{h}  blur {tier:<16} {each:8.2} ms/frame");
        }
    }
}
