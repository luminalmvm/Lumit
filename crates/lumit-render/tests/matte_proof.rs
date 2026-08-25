//! Visual proof for the K-395 deeper-meaning matte overrides (docs/08 §2.6):
//! render the same real frame with the same matte three ways per effect, so the
//! claim "the override is visibly a different picture from the generic
//! dissolve" can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the GPU and the CPU agree, and the `run_ops` tests
//! prove the matte reaches the right effect exactly once. Neither can prove the
//! override was *worth* having — that is a question about what the picture looks
//! like, and it has to be looked at:
//!
//! 1. **Gaussian blur.** The generic dissolve blurs the whole frame at the full
//!    Radius and then fades that back where the matte is dark, so every pixel
//!    was still gathered from the full radius away: a sharp picture with a wide
//!    veil over it. The override scales the radius per pixel, so where the matte
//!    is grey the blur is genuinely *narrower*. Look at how detail dies out
//!    across the matte ramp: a dissolve loses contrast evenly, a radius ramp
//!    loses fine detail first and coarse detail last, the way a lens does.
//! 2. **Glow.** The generic dissolve makes the halo stop dead at the matte's
//!    edge — a glow "on the sign only" that does not light the wall beside it.
//!    The override gates the *seed*, so only the lit part of the matte blooms
//!    but its halo spreads outward across the dark matte as light does. Look
//!    just outside the matte's edge: dissolve leaves it untouched, the gate
//!    spills into it.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_MATTE_PROOF_CLIPS="C:/tmp/lumit-flow-clips/cartoon.mp4" \
//! LUMIT_MATTE_PROOF_OUT="C:/tmp/lumit-flow-clips" \
//!   cargo test -p lumit-render --release --test matte_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_MATTE_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;

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

/// The generic strength semantic, for comparison: the effect run in full, then
/// dissolved back towards its input by the matte. This is what every effect that
/// does *not* override gets, and what the two exemplars would have got had they
/// not claimed their matte — so it is the "before" picture here.
fn dissolved(input: &[f32], processed: &mut [f32], matte: &[f32]) {
    cpu::matte_mix(processed, input, matte, false);
}

/// A left-to-right ramp of matte, full at the left edge and black at the right,
/// with a hard vertical step down the middle third so both a gradient and an
/// edge are in one picture. Opaque, since a matte's alpha is not what is read.
fn ramp_matte(w: u32, h: u32) -> Vec<f32> {
    let mut m = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let t = x as f32 / (w - 1) as f32;
            // A smooth ramp over the outer thirds, a hard edge in the middle:
            // the gradient shows the blur's width changing, the edge shows what
            // each semantic does at a boundary.
            let k = if (0.45..0.55).contains(&t) {
                f32::from(t < 0.5)
            } else {
                (1.0 - t).clamp(0.0, 1.0)
            };
            m[i] = k;
            m[i + 1] = k;
            m[i + 2] = k;
            m[i + 3] = 1.0;
        }
    }
    m
}

#[test]
#[ignore = "harness: set LUMIT_MATTE_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_dissolve_and_the_override() {
    let Ok(clips) = std::env::var("LUMIT_MATTE_PROOF_CLIPS") else {
        eprintln!("set LUMIT_MATTE_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_MATTE_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-flow-clips".to_owned());
    let frame: usize = std::env::var("LUMIT_MATTE_PROOF_FRAME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

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
        let Ok(a) = dec.frame_rgba(frame, None) else {
            eprintln!("{stem}: could not decode frame {frame}");
            continue;
        };
        let (w, h) = (a.width, a.height);
        let lin = to_linear(&a.rgba);
        let matte = ramp_matte(w, h);

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.m{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);
        write("1-matte", &to_srgb(&matte));

        // ---- Gaussian blur: radius modulation vs dissolving a full blur.
        let radius = (w as f32 * 0.02).max(4.0);
        let mut full = lin.clone();
        cpu::blur_gaussian(&mut full, w, h, radius, 1, 1.0);
        write("2-blur-dissolved", &{
            let mut d = full.clone();
            dissolved(&lin, &mut d, &matte);
            to_srgb(&d)
        });
        let mut varied = lin.clone();
        cpu::blur_gaussian_matted(&mut varied, w, h, radius, 1, 1.0, &matte);
        write("3-blur-radius", &to_srgb(&varied));

        // ---- Glow: seed gate vs dissolving a finished glow.
        // A deliberately strong glow: the point of the pair is to see WHERE the
        // light goes, and a subtle bloom makes that a matter of opinion.
        let (g_radius, threshold, knee, intensity) = (radius * 3.0, 0.25f32, 0.4f32, 3.0f32);
        let mut glowed = lin.clone();
        cpu::glow(
            &mut glowed,
            w,
            h,
            g_radius,
            threshold,
            knee,
            intensity,
            [1.0; 4],
            1.0,
            &[],
        );
        write("4-glow-dissolved", &{
            let mut d = glowed.clone();
            dissolved(&lin, &mut d, &matte);
            to_srgb(&d)
        });
        let mut seeded = lin.clone();
        cpu::glow(
            &mut seeded,
            w,
            h,
            g_radius,
            threshold,
            knee,
            intensity,
            [1.0; 4],
            1.0,
            &matte,
        );
        write("5-glow-seeded", &to_srgb(&seeded));
    }
}
