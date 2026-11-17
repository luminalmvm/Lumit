#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The Correction's passes against its CPU oracle, across every group of rows.

use lumit_core::fx::effects::correction::{packed::HUE_FLAT, Correction};
use lumit_core::fx::params::{CurvePoints, Params};
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::fx::shape_points::ShapePoints;
use lumit_gpu::fx::common::{f16_bits, f16_to_f32, readback_linear_f32, upload_linear_f32};
use lumit_gpu::fx::correction::CorrectionOp;

/// Colour across, light down, a half-clear band, a clear corner and an HDR
/// spike, already fp16 as the card holds it.
fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let s = x as f32 / w as f32;
            let l = 0.05 + 0.95 * y as f32 / h as f32;
            let rgb = [
                l * (0.5 + 0.5 * (6.0 * s).sin()),
                l * (0.5 + 0.5 * (6.0 * s + 2.1).sin()),
                l * (0.5 + 0.5 * (6.0 * s + 4.2).sin()),
            ];
            let a = if x >= w - 6 && y < 6 {
                0.0
            } else if (20..24).contains(&x) {
                0.5
            } else {
                1.0
            };
            img[i..i + 4].copy_from_slice(&[rgb[0] * a, rgb[1] * a, rgb[2] * a, a]);
        }
    }
    let spike = ((10 * w + 30) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);
    img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

/// A prepared matte: a grey ramp down the picture.
fn matte(w: u32, h: u32) -> Vec<f32> {
    let mut m = Vec::new();
    for y in 0..h {
        for _ in 0..w {
            let k = f16_to_f32(f16_bits(y as f32 / (h - 1) as f32));
            m.extend_from_slice(&[k, k, k, 1.0]);
        }
    }
    m
}

/// A Correction with every row at rest.
fn rest() -> Correction {
    let mut c = Correction::read(Params::EMPTY);
    let flat = CurvePoints::sanitised(&HUE_FLAT);
    for curve in [
        &mut c.hue_hue,
        &mut c.hue_sat,
        &mut c.hue_lum,
        &mut c.lum_sat,
        &mut c.sat_sat,
        &mut c.sat_lum,
    ] {
        *curve = flat;
    }
    c
}

fn wheels(space: u32) -> Correction {
    let mut c = rest();
    c.space = space;
    c.lift = [0.05, -0.02, 0.0, 1.0];
    c.gamma = [0.1, 0.0, -0.15, 1.0];
    c.gain = [1.2, 1.0, 0.9, 1.0];
    c.offset = [0.01, 0.0, -0.02, 1.0];
    c.shadow = [0.0, 0.0, 0.05, 1.0];
    c.midtone = [0.03, 0.0, 0.0, 1.0];
    c.highlight = [-0.02, 0.02, 0.0, 1.0];
    c.exposure = 0.5;
    c.temperature = 30.0;
    c.tint = -10.0;
    c.contrast = 120.0;
    c.saturation = 130.0;
    c.hue = 20.0;
    c.lum_mix = 60.0;
    c.colour_boost = 40.0;
    c.shadows = 30.0;
    c.highlights = -25.0;
    c.mixer_red = [0.9, 0.1, 0.0, 1.0];
    c.mixer_green = [0.05, 0.9, 0.05, 1.0];
    c.mixer_blue = [0.0, 0.2, 0.8, 1.0];
    c.mixer_keep_luma = true;
    c
}

fn cases(w: u32, h: u32) -> Vec<(&'static str, Correction, bool)> {
    let mut log = wheels(1);
    log.midtone_detail = 60.0;

    let mut curves = rest();
    curves.curve_y = CurvePoints::sanitised(&[[0.0, 0.05], [0.5, 0.55], [1.0, 0.95]]);
    curves.curve_r = CurvePoints::sanitised(&[[0.0, 0.0], [0.4, 0.5], [1.0, 1.0]]);
    curves.soft_high = 60.0;
    curves.soft_low = 30.0;
    curves.hue_hue = CurvePoints::sanitised(&[[0.0, 0.5], [0.3, 0.6], [0.7, 0.45], [1.0, 0.5]]);
    curves.hue_sat = CurvePoints::sanitised(&[[0.1, 0.7], [0.6, 0.3]]);
    curves.hue_lum = CurvePoints::sanitised(&[[0.0, 0.6], [0.5, 0.4], [1.0, 0.6]]);
    curves.lum_sat = CurvePoints::sanitised(&[[0.0, 0.3], [1.0, 0.6]]);
    curves.sat_sat = CurvePoints::sanitised(&[[0.0, 0.5], [1.0, 0.4]]);
    curves.sat_lum = CurvePoints::sanitised(&[[0.0, 0.5], [1.0, 0.6]]);

    let mut hsl = rest();
    hsl.gain = [1.5, 0.8, 0.8, 1.0];
    hsl.saturation = 150.0;
    hsl.qual_mode = 1;
    hsl.qual_hue = 40.0;
    hsl.qual_hue_width = 90.0;
    hsl.qual_hue_soft = 30.0;
    hsl.qual_hue_sym = 30.0;
    hsl.qual_sat_low = 10.0;
    hsl.qual_lum_low = 15.0;
    hsl.qual_lum_high = 90.0;
    hsl.qual_denoise = 60.0;
    hsl.qual_clean_black = 20.0;
    hsl.qual_clean_white = 10.0;
    hsl.qual_white_clip = 90.0;
    hsl.qual_blur = 3.0;
    hsl.qual_grow = 1.5;
    hsl.qual_invert = true;

    let mut rgb = wheels(0);
    rgb.qual_mode = 2;
    rgb.qual_red_low = 30.0;
    rgb.qual_green_high = 70.0;
    rgb.qual_rgb_soft = 15.0;
    rgb.qual_grow = -1.0;
    rgb.qual_blur = 2.0;

    let mut windows = wheels(2);
    windows.win1_shape = 1;
    windows.win1_x = w as f32 * 0.3;
    windows.win1_y = h as f32 * 0.4;
    windows.win1_w = 20.0;
    windows.win1_h = 12.0;
    windows.win1_angle = 25.0;
    windows.win1_soft = 4.0;
    windows.win1_soft_in = 2.0;
    windows.win2_shape = 2;
    windows.win2_x = w as f32 * 0.7;
    windows.win2_y = h as f32 * 0.6;
    windows.win2_w = 16.0;
    windows.win2_h = 14.0;
    windows.win2_round = 40.0;
    windows.win2_soft = 3.0;
    windows.win2_opacity = 70.0;
    windows.win3_shape = 3;
    windows.win3_x = w as f32 * 0.5;
    windows.win3_y = h as f32 * 0.5;
    windows.win3_w = 18.0;
    windows.win3_h = 18.0;
    windows.win3_points =
        ShapePoints::sanitised(&[[-1.0, -0.6], [0.8, -1.0], [1.0, 0.7], [-0.3, 1.0]]);
    windows.win3_subtract = true;
    windows.win3_soft = 2.0;
    windows.win4_shape = 4;
    windows.win4_x = w as f32 * 0.5;
    windows.win4_y = h as f32 * 0.8;
    windows.win4_angle = -30.0;
    windows.win4_soft = 6.0;
    windows.win4_invert = true;

    let mut luma = curves;
    luma.qual_mode = 3;
    luma.qual_lum_low = 30.0;
    luma.qual_lum_high = 70.0;
    luma.qual_lum_soft = 20.0;
    luma.win1_shape = 3;
    luma.win1_x = w as f32 * 0.5;
    luma.win1_y = h as f32 * 0.5;
    luma.win1_w = w as f32 * 0.8;
    luma.win1_h = h as f32 * 0.8;
    luma.win1_smooth = false;
    luma.win1_soft = 3.0;
    luma.win1_points = ShapePoints::sanitised(&[[-1.0, -1.0], [1.0, -0.5], [0.0, 1.0]]);
    luma.mix = 60.0;

    let mut hdr = rest();
    hdr.hdr_falloff = 1.5;
    hdr.hdr_balance = [0.05, 0.0, -0.05, 1.0];
    hdr.hdr_black_offset = 0.01;
    hdr.hdr_black_saturation = 50.0;
    hdr.hdr_shadow_exposure = 0.7;
    hdr.hdr_shadow_balance = [0.0, 0.1, 0.2, 1.0];
    hdr.hdr_light_exposure = -1.2;
    hdr.hdr_light_saturation = 140.0;
    hdr.hdr_specular_range = 2.0;
    hdr.hdr_specular_exposure = -2.0;

    // Kept round an orange and a teal, dropped round a dark red; Y, Cb, Cr
    // as the video space reads them.
    let mut d3 = wheels(0);
    d3.qual_mode = 4;
    d3.qual_3d_keep = ShapePoints::sanitised(&[[-0.4, 0.3], [0.2, -0.3]]);
    d3.qual_3d_keep_luma = ShapePoints::sanitised(&[[0.6, 0.0], [0.5, 0.0]]);
    d3.qual_3d_drop = ShapePoints::sanitised(&[[-0.1, 0.4]]);
    d3.qual_3d_drop_luma = ShapePoints::sanitised(&[[0.3, 0.0]]);
    d3.qual_3d_size = 30.0;
    d3.qual_3d_soft = 40.0;
    d3.qual_blur = 1.5;

    vec![
        ("at rest", rest(), false),
        ("3d qualifier", d3, false),
        ("hdr zones", hdr, false),
        ("wheels in video", wheels(0), false),
        ("wheels in log with detail", log, false),
        ("curves and hue curves", curves, false),
        ("hsl qualifier with finesse", hsl, false),
        ("rgb qualifier under a matte", rgb, true),
        ("four windows in linear", windows, false),
        ("luma qualifier and a straight polygon", luma, false),
    ]
}

#[test]
fn wgsl_correction_matches_the_cpu_oracle() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = picture(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let m = matte(w, h);
    let mtex = upload_linear_f32(&ctx, &m, w, h);
    for (name, c, matted) in cases(w, h) {
        let pk = c.packed(1.0);
        let mut cpu = img.clone();
        lumit_core::fx::effects::correction::cpu::correction(
            &mut cpu,
            w,
            h,
            &pk,
            if matted { &m } else { &[] },
        );
        let words = pk.words(matted);
        let op = CorrectionOp {
            words: &words,
            data: &pk.data,
            neutral: pk.neutral,
            qualifier_on: pk.qualifier_on(),
            windows_on: pk.windows_on(),
            denoise_px: pk.qual.denoise_px,
            blur_px: pk.qual.blur_px,
            grow_px: pk.qual.grow_px,
            detail_px: pk.detail_px,
            detail_on: pk.detail != 0.0,
        };
        let matte = matted.then_some(&mtex);
        let run = || {
            let out = fx.correction(&ctx, &tex, w, h, matte, &op);
            readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };
        let gpu = run();
        if name == "at rest" {
            assert!(pk.neutral, "a Correction at rest must be neutral");
            assert_eq!(gpu, img, "at rest the picture goes through untouched");
            assert_eq!(cpu, img, "at rest the CPU leaves the picture alone");
            continue;
        }
        assert!(!pk.neutral, "{name}: graded rows must not read as rest");
        assert!(
            cpu.iter().zip(&img).any(|(a, b)| (a - b).abs() > 1e-2),
            "{name}: the grade must change the picture"
        );
        // fp16 out, and fp16 keys and blurs in between: judged relative to
        // the value, with a floor for the dark end.
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs() / a.abs().max(1.0))
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-2, "{name}: worst difference {worst}");
        assert!(
            gpu.iter().all(|v| v.is_finite()),
            "{name}: a value is not finite"
        );
        assert_eq!(gpu, run(), "{name}: the GPU Correction must be bit-stable");
    }
}

/// CDL and the colour warper against their CPU twins: a graded case each,
/// and at rest the picture untouched.
#[test]
fn wgsl_cdl_and_warper_match_the_cpu_oracle() {
    use lumit_core::fx::effects::{cdl::Cdl, colour_warper::ColourWarper};
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = picture(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let mut cdl = Cdl::read(Params::EMPTY);
    cdl.space = 1;
    cdl.slope = [1.1, 0.95, 1.05, 1.0];
    cdl.offset = [0.02, -0.01, 0.0, 1.0];
    cdl.power = [0.9, 1.0, 1.2, 1.0];
    cdl.saturation = 0.8;
    cdl.mix = 80.0;
    let mut warp = ColourWarper::read(Params::EMPTY);
    warp.resolution = 1;
    let mut moves = vec![[0.02, -0.01]];
    for i in 0..24 {
        let a = i as f32 * 0.7;
        moves.push([0.08 * a.sin(), 0.06 * (1.3 * a).cos()]);
    }
    warp.points = ShapePoints::sanitised(&moves);
    warp.strength = 90.0;
    let mut warp_luma = warp;
    warp_luma.resolution = 0;
    warp_luma.keep_luma = false;

    let words = |name: &str| match name {
        "cdl" => (cdl.packed().words(), cdl.packed().neutral),
        "warper" => (warp.packed().words(), warp.packed().neutral),
        _ => (warp_luma.packed().words(), warp_luma.packed().neutral),
    };
    let cpu_of = |name: &str| {
        let mut out = img.clone();
        for px in out.chunks_exact_mut(4) {
            let o = [px[0], px[1], px[2], px[3]];
            let v = match name {
                "cdl" => cdl.packed().pixel(o),
                "warper" => warp.packed().pixel(o),
                _ => warp_luma.packed().pixel(o),
            };
            px.copy_from_slice(&v);
        }
        out
    };
    for name in ["cdl", "warper", "warper without keep luma"] {
        let (words, neutral) = words(name);
        assert!(!neutral, "{name}: graded rows must not read as rest");
        let cpu = cpu_of(name);
        let run = || {
            let out = if name == "cdl" {
                fx.cdl(&ctx, &tex, w, h, &words)
            } else {
                fx.colour_warper(&ctx, &tex, w, h, &words)
            };
            readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };
        let gpu = run();
        assert!(
            cpu.iter().zip(&img).any(|(a, b)| (a - b).abs() > 1e-2),
            "{name}: the grade must change the picture"
        );
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs() / a.abs().max(1.0))
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-2, "{name}: worst difference {worst}");
        assert!(gpu.iter().all(|v| v.is_finite()), "{name}: not finite");
        assert_eq!(gpu, run(), "{name}: must be bit-stable");
    }
    assert!(Cdl::read(Params::EMPTY).packed().neutral);
    assert!(ColourWarper::read(Params::EMPTY).packed().neutral);
}
