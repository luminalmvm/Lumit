//! Visual proof for the colour and generate batches (docs/08 §3.30–§3.37):
//! render one real frame through each of the eight effects at two settings, so
//! the kernels can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or noise, or the input
//! unchanged, agrees with a reference that does the same wrong thing. This is the
//! sibling of `distort_proof.rs` and `utility_proof.rs` for the first two
//! batches, which shipped without one. Eight things have to be looked at:
//!
//! 1. **Curves** must lift the shadows and roll the highlights when the master
//!    knots move, and tint when only one channel's do — not merely brighten.
//! 2. **Levels** must crush and stretch the range, and the gamma pair must be
//!    visibly different from the black/white pair.
//! 3. **Brightness** must move the picture without the sign confusion AE's two
//!    sliders invite: positive Contrast pushes away from the neutral point, not
//!    towards it.
//! 4. **Hue and saturation** must rotate the whole picture on Master, and only
//!    one family of colours on a range — the range pair is the readable proof
//!    that the six bands are not all one band.
//! 5. **Fill** must flood the colour and keep the alpha it was given, so a
//!    partial Mix reads as a tint rather than a flat card.
//! 6. **Gradient** must ramp between the two points, and Radial must be round.
//! 7. **Noise** must grain rather than tint: mono at low Amount, colour at high.
//! 8. **Fractal noise** must be a field of blobs with structure at every
//!    Complexity, and must be *the same field* at the same Seed — the pair here
//!    changes Complexity only, so the large shapes have to stay put.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_CG_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_CG_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test colour_generate_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_CG_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    brightness::Brightness, curves::Curves, fill::Fill, fractal_noise::FractalNoise,
    gradient::Gradient, hue_saturation::HueSaturation, levels::Levels, noise::Noise,
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

#[test]
#[ignore = "harness: set LUMIT_CG_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_eight_colour_and_generate_effects() {
    let Ok(clips) = std::env::var("LUMIT_CG_PROOF_CLIPS") else {
        eprintln!("set LUMIT_CG_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir =
        std::env::var("LUMIT_CG_PROOF_OUT").unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_CG_PROOF_FRAME")
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
        let (fw, fh) = (w as f32, h as f32);

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.cg{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);

        // ---- Curves: a master S-curve, then a red-only lift (the per-channel
        // proof — a tint no master move can produce).
        let mut s_curve = Curves::read(Params::EMPTY);
        s_curve.master_shadows = 0.15;
        s_curve.master_midtones = 0.50;
        s_curve.master_highlights = 0.85;
        let (y, m, mix) = s_curve.packed();
        let mut out = lin.clone();
        cpu::curves(&mut out, y, m, mix);
        write("1-curves-s-curve", &to_srgb(&out));

        let mut red_lift = Curves::read(Params::EMPTY);
        red_lift.red_shadows = 0.45;
        red_lift.red_midtones = 0.70;
        red_lift.blue_shadows = 0.10;
        red_lift.blue_midtones = 0.35;
        let (y, m, mix) = red_lift.packed();
        let mut out = lin.clone();
        cpu::curves(&mut out, y, m, mix);
        write("2-curves-red-lift", &to_srgb(&out));

        // ---- Levels: a hard range crush, then a gamma-only move at the same
        // black and white, so the two controls cannot be confused for each other.
        let mut crush = Levels::read(Params::EMPTY);
        crush.master_in_black = 0.20;
        crush.master_in_white = 0.75;
        let (r, mix) = crush.packed();
        let mut out = lin.clone();
        cpu::levels(&mut out, r, mix);
        write("3-levels-crush", &to_srgb(&out));

        let mut gamma = Levels::read(Params::EMPTY);
        gamma.master_gamma = 2.2;
        let (r, mix) = gamma.packed();
        let mut out = lin.clone();
        cpu::levels(&mut out, r, mix);
        write("4-levels-gamma", &to_srgb(&out));

        // ---- Brightness: AE's two sliders, one each way.
        let mut up = Brightness::read(Params::EMPTY);
        up.brightness = 25.0;
        let (b, k, mix) = up.packed();
        let mut out = lin.clone();
        cpu::brightness(&mut out, b, k, mix);
        write("5-brightness-up", &to_srgb(&out));

        let mut punch = Brightness::read(Params::EMPTY);
        punch.contrast = 40.0;
        let (b, k, mix) = punch.packed();
        let mut out = lin.clone();
        cpu::brightness(&mut out, b, k, mix);
        write("6-brightness-contrast", &to_srgb(&out));

        // ---- Hue and saturation: the master rotation, then a single range, so
        // the six bands are proved to be six.
        let mut spun = HueSaturation::read(Params::EMPTY);
        spun.master_hue = 120.0;
        let (bands, mix) = spun.packed();
        let mut out = lin.clone();
        cpu::hue_saturation(&mut out, bands, mix);
        write("7-huesat-master-spun", &to_srgb(&out));

        let mut blues_only = HueSaturation::read(Params::EMPTY);
        blues_only.blues_saturation = 100.0;
        blues_only.blues_hue = 60.0;
        blues_only.reds_saturation = -100.0;
        let (bands, mix) = blues_only.packed();
        let mut out = lin.clone();
        cpu::hue_saturation(&mut out, bands, mix);
        write("8-huesat-ranges-only", &to_srgb(&out));

        // ---- Fill: the flood, then a half Mix, which must read as a tint of the
        // picture rather than a flat card at half brightness.
        let mut flood = Fill::read(Params::EMPTY);
        flood.colour = [0.9, 0.2, 0.1, 1.0];
        let (colour, mix) = flood.packed();
        let mut out = lin.clone();
        cpu::fill(&mut out, colour, mix);
        write("9-fill-flood", &to_srgb(&out));

        let mut tinted = Fill::read(Params::EMPTY);
        tinted.colour = [0.1, 0.4, 0.9, 1.0];
        tinted.mix = 45.0;
        let (colour, mix) = tinted.packed();
        let mut out = lin.clone();
        cpu::fill(&mut out, colour, mix);
        write("10-fill-half-mix", &to_srgb(&out));

        // ---- Gradient: a diagonal linear ramp, then a scattered radial.
        let mut linear = Gradient::read(Params::EMPTY);
        linear.start_x = 0.0;
        linear.start_y = 0.0;
        linear.end_x = fw;
        linear.end_y = fh;
        linear.start_colour = [0.02, 0.05, 0.30, 1.0];
        linear.end_colour = [1.0, 0.75, 0.25, 1.0];
        let mut out = lin.clone();
        cpu::gradient(&mut out, w, h, &linear.packed());
        write("11-gradient-linear", &to_srgb(&out));

        let mut radial = Gradient::read(Params::EMPTY);
        radial.shape = 1;
        radial.start_x = fw * 0.5;
        radial.start_y = fh * 0.5;
        radial.end_x = fw * 0.95;
        radial.end_y = fh * 0.5;
        radial.start_colour = [1.0, 1.0, 0.9, 1.0];
        radial.end_colour = [0.0, 0.0, 0.05, 1.0];
        radial.scatter = 25.0;
        radial.seed = 7;
        let mut out = lin.clone();
        cpu::gradient(&mut out, w, h, &radial.packed());
        write("12-gradient-radial-scatter", &to_srgb(&out));

        // ---- Noise: mono grain, then heavy colour grain.
        let mono = Noise::read(Params::EMPTY);
        let mut mono = mono;
        mono.amount = 12.0;
        mono.seed = 7;
        let (amount, gaussian, colour, seed, tick, mix) = mono.packed(0);
        let mut out = lin.clone();
        cpu::noise(&mut out, w, h, amount, gaussian, colour, seed, tick, mix);
        write("13-noise-mono", &to_srgb(&out));

        let mut heavy = Noise::read(Params::EMPTY);
        heavy.amount = 45.0;
        heavy.colour_noise = true;
        heavy.distribution = 1;
        heavy.seed = 7;
        let (amount, gaussian, colour, seed, tick, mix) = heavy.packed(0);
        let mut out = lin.clone();
        cpu::noise(&mut out, w, h, amount, gaussian, colour, seed, tick, mix);
        write("14-noise-colour-gaussian", &to_srgb(&out));

        // ---- Fractal noise: one octave, then six at the same Seed and Scale.
        // The large shapes have to stay put between the two — that is the proof
        // the octave sum is a sum and not a re-draw.
        let fractal = |complexity: i32, fractal_type: u32| {
            let mut f = FractalNoise::read(Params::EMPTY);
            f.scale = fw * 0.20;
            f.offset_x = fw * 0.5;
            f.offset_y = fh * 0.5;
            f.complexity = complexity;
            f.fractal_type = fractal_type;
            f.seed = 7;
            f
        };
        let mut out = lin.clone();
        cpu::fractal_noise(&mut out, w, h, &fractal(1, 0).packed());
        write("15-fractal-one-octave", &to_srgb(&out));

        let mut out = lin.clone();
        cpu::fractal_noise(&mut out, w, h, &fractal(6, 1).packed());
        write("16-fractal-six-turbulent", &to_srgb(&out));
    }
}
