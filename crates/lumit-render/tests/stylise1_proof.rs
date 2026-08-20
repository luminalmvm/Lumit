//! Visual proof for Wave 2's Stylise I batch (docs/08 §3.58–§3.63): render one
//! real frame through each of the six effects at two settings, so the kernels
//! can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or flat, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Six things
//! here have to be looked at:
//!
//! 1. **Posterize** must show flat bands with the steps spread across the whole
//!    range, not piled into the highlights.
//! 2. **Threshold** must be a clean two-tone stencil at 50, and must move the
//!    right way when the Level does.
//! 3. **Tritone** must read as a duotone print — coloured shadows, coloured
//!    highlights, no grey left in the middle.
//! 4. **Photo filter** must warm or cool the picture *without* changing its
//!    exposure while Preserve luminosity is on, and must darken it with the
//!    switch off.
//! 5. **Black and white** must be grey, and a red filter must visibly darken a
//!    sky against the same picture's default conversion.
//! 6. **Shadow highlight** must open the dark regions and hold the bright ones,
//!    without haloing and without softening anything.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_STYLISE1_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_STYLISE1_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test stylise1_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_STYLISE1_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    black_and_white::BlackAndWhite, photo_filter::PhotoFilter, posterize::Posterize,
    shadow_highlight::ShadowHighlight, threshold::Threshold, tritone::Tritone,
};
use lumit_core::fx::{EffectMetadata, Params};

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

/// The mean Rec. 709 luma of a linear frame — what the Photo filter and Shadow
/// highlight claims are read against, printed beside each render so the
/// judgement is not purely by eye.
fn mean_luma(lin: &[f32]) -> f32 {
    let n = (lin.len() / 4) as f32;
    lin.chunks_exact(4)
        .map(|p| p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722)
        .sum::<f32>()
        / n
}

#[test]
#[ignore = "harness: set LUMIT_STYLISE1_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_six_stylise_one_effects() {
    let Ok(clips) = std::env::var("LUMIT_STYLISE1_PROOF_CLIPS") else {
        eprintln!("set LUMIT_STYLISE1_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_STYLISE1_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_STYLISE1_PROOF_FRAME")
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
        let diag = ((w * w + h * h) as f32).sqrt();

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.s1{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);
        eprintln!("source mean luma {:.4}", mean_luma(&lin));

        // ---- Posterize: the poster and the two-tone print. The bands must be
        // spread across the range, which is what the perceptual spacing buys.
        for (tag, levels) in [("1-posterize-8", 8), ("2-posterize-3", 3)] {
            let mut q = Posterize::read(Params::EMPTY);
            q.levels = levels;
            let (n, mix) = q.packed();
            let mut out = lin.clone();
            cpu::posterize(&mut out, n, mix);
            write(tag, &to_srgb(&out));
        }

        // ---- Threshold: the stencil at mid-grey, and a soft one placed low.
        for (tag, level, softness) in [
            ("3-threshold-50", 50.0, 0.0),
            ("4-threshold-30-soft", 30.0, 25.0),
        ] {
            let mut t = Threshold::read(Params::EMPTY);
            t.level = level;
            t.softness = softness;
            let (lv, hw, mix) = t.packed();
            let mut out = lin.clone();
            cpu::threshold(&mut out, lv, hw, mix);
            write(tag, &to_srgb(&out));
        }

        // ---- Tritone: the shipped default (a split-toned print), and a
        // cyanotype.
        let mut cyan = Tritone::read(Params::EMPTY);
        cyan.shadows = [0.0, 0.03, 0.18, 1.0];
        cyan.midtones = [0.05, 0.30, 0.60, 1.0];
        cyan.highlights = [0.85, 0.97, 1.0, 1.0];
        for (tag, t) in [
            ("5-tritone-default", Tritone::read(Params::EMPTY)),
            ("6-tritone-cyanotype", cyan),
        ] {
            let mut out = lin.clone();
            cpu::tritone(&mut out, &t.packed());
            write(tag, &to_srgb(&out));
        }

        // ---- Photo filter: a warming 85 at AE's density with the exposure
        // held, and a deep blue at full density with it let go.
        for (tag, filter, density, preserve) in [
            ("7-photofilter-warming-85", 0u32, 25.0, true),
            ("8-photofilter-deep-blue-unpreserved", 16, 100.0, false),
        ] {
            let mut f = PhotoFilter::read(Params::EMPTY);
            f.filter = filter;
            f.density = density;
            f.preserve_luminosity = preserve;
            let mut out = lin.clone();
            cpu::photo_filter(&mut out, &f.packed());
            eprintln!("{tag}: mean luma {:.4}", mean_luma(&out));
            write(tag, &to_srgb(&out));
        }

        // ---- Black and white: AE's default weights, and a red-filter
        // conversion that must darken blues and lift reds against it.
        let mut red = BlackAndWhite::read(Params::EMPTY);
        red.reds = 200.0;
        red.yellows = 140.0;
        red.blues = -40.0;
        red.cyans = -20.0;
        let mut sepia = BlackAndWhite::read(Params::EMPTY);
        sepia.tint = true;
        for (tag, b) in [
            ("9-blackwhite-default", BlackAndWhite::read(Params::EMPTY)),
            ("10-blackwhite-red-filter", red),
            ("11-blackwhite-sepia", sepia),
        ] {
            let mut out = lin.clone();
            cpu::black_and_white(&mut out, &b.packed());
            write(tag, &to_srgb(&out));
        }

        // ---- Shadow highlight: AE's default pair, and a hard rescue. The
        // Radius is % diag; the resolve step would have made it pixels.
        for (tag, sa, ha, radius, cc) in [
            ("12-shadowhighlight-default", 25.0, 25.0, 0.015, 20.0),
            ("13-shadowhighlight-rescue", 90.0, 70.0, 0.03, 40.0),
        ] {
            let mut s = ShadowHighlight::read(Params::EMPTY);
            s.shadow_amount = sa;
            s.highlight_amount = ha;
            s.radius = radius * diag;
            s.colour_correction = cc;
            let mut out = lin.clone();
            cpu::shadow_highlight(&mut out, w, h, &s.packed());
            eprintln!("{tag}: mean luma {:.4}", mean_luma(&out));
            write(tag, &to_srgb(&out));
        }
    }
}
