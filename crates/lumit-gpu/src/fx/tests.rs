use super::*;

#[test]
fn f16_round_trips_representative_values() {
    for v in [0.0f32, 1.0, -1.0, 0.5, 4.0, 1.5e-5, 65504.0] {
        let rt = f16_to_f32(f16_bits(v));
        assert!((rt - v).abs() <= (v.abs() * 1e-3).max(1e-6), "{v} → {rt}");
    }
}

/// The §1.6 oracle corpus: a diagonal gradient, a hard alpha edge down
/// the middle, and an HDR spike — already fp16-quantised, so comparisons
/// isolate the kernel maths from upload rounding.
fn corpus(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = (x + y) as f32 / (w + h) as f32;
            let a = if x < w / 2 { 1.0 } else { 0.0 };
            img[i] = g * a;
            img[i + 1] = (1.0 - g) * a;
            img[i + 2] = 0.25 * a;
            img[i + 3] = a;
        }
    }
    let spike = ((10 * w + 20) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);
    img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

/// Worst absolute difference between two images.
fn worst_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}

/// Worst distance between two images in fp16 ULPs — the §1.6 metric for
/// `trivial`/`cheap` effects. Bits are remapped so consecutive integers
/// are consecutive representable halves (±0 coincide).
fn worst_f16_ulp(a: &[f32], b: &[f32]) -> i32 {
    fn key(v: f32) -> i32 {
        let bits = i32::from(f16_bits(v));
        if bits & 0x8000 != 0 {
            -(bits & 0x7fff)
        } else {
            bits
        }
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| (key(*x) - key(*y)).abs())
        .fold(0, i32::max)
}

/// The §1.6 oracle: the WGSL blur agrees with the CPU reference on a
/// corpus of gradient + alpha edge + HDR spike, per edge policy — and is
/// bit-stable against itself (§2.4 determinism).
#[test]
fn wgsl_blur_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    // Corpus (docs/08 §1.6): a diagonal gradient, a hard alpha edge down
    // the middle, and an HDR spike.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = (x + y) as f32 / (w + h) as f32;
            let a = if x < w / 2 { 1.0 } else { 0.0 };
            img[i] = g * a;
            img[i + 1] = (1.0 - g) * a;
            img[i + 2] = 0.25 * a;
            img[i + 3] = a;
        }
    }
    let spike = ((10 * w + 20) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);

    for edge in [0u32, 1, 2] {
        for (radius, mix) in [(3.0f32, 1.0f32), (7.5, 0.6), (0.0, 1.0)] {
            // fp16 quantise the input exactly as the GPU sees it, so the
            // comparison isolates the blur maths from upload rounding.
            let quantised: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
            let mut cpu = quantised.clone();
            lumit_core::fx::cpu::blur_gaussian(&mut cpu, w, h, radius, edge, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = BlurOp {
                radius_px: radius,
                edge,
                mix,
            };
            let out = fx.blur(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = cpu
                .iter()
                .zip(&gpu)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            // Moderate-class perceptual epsilon (§1.6), scaled for the
            // HDR corpus: fp16 has ~2^-11 relative steps, and the spike
            // sits at 6.0.
            assert!(
                worst < 2e-2,
                "edge {edge} radius {radius} mix {mix}: worst diff {worst}"
            );

            // Determinism: a second run is bit-identical to the first.
            let out2 = fx.blur(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU blur must be bit-stable");
        }
    }
}

/// The §1.6 oracle for sharpen: WGSL agrees with the CPU reference on
/// the corpus across parameter sweeps, and is bit-stable (§2.4). The
/// internal gaussian's intermediates round through fp16 textures on the
/// GPU and stay f32 on the CPU, so the bound is an absolute epsilon:
/// 5e-3 ≈ 1–2 fp16 ULP at the corpus's HDR peak of 6.0 (measured worst
/// on NVIDIA: 2.9e-3).
#[test]
fn wgsl_sharpen_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (amount, radius, threshold, luma_only, mix) in [
        (0.6f32, 3.0f32, 0.05f32, true, 1.0f32),
        (1.5, 6.0, 0.0, false, 0.7),
        (3.0, 2.0, 0.2, true, 1.0),
        (0.0, 3.0, 0.0, true, 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::sharpen(&mut cpu, w, h, amount, radius, threshold, luma_only, mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = SharpenOp {
            amount,
            radius_px: radius,
            threshold,
            luma_only,
            mix,
        };
        let out = fx.sharpen(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_diff(&cpu, &gpu);
        // Logged so real cross-vendor deltas accumulate (docs/08 open
        // question 5: the class tolerances are placeholders until then).
        eprintln!("sharpen a={amount} r={radius} t={threshold}: worst {worst:.2e}");
        assert!(
            worst < 5e-3,
            "amount {amount} radius {radius} threshold {threshold} \
                 luma {luma_only} mix {mix}: worst diff {worst}"
        );

        let out2 = fx.sharpen(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU sharpen must be bit-stable");
    }
}

/// The §1.6 oracle for the plain 3×3 sharpen (docs/08 §3.9, K-138): a cheap
/// kernel reading only the pixel and its four integer neighbours directly
/// (no intermediate fp16 texture, unlike the Unsharp mask's internal
/// gaussian), so the CPU and GPU must agree to ≤ 2 fp16 ULP and the GPU is
/// bit-stable (§2.4). Amount 0 (whatever the Mix) and Mix 0 are the bit-exact
/// passthrough on both paths. The corpus carries partial-alpha pixels — the
/// convolution runs on unpremultiplied colour (§2.2), so the premultiply
/// round trip is load-bearing.
#[test]
fn wgsl_sharpen_simple_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus_with_partials(w, h);
    for (name, amount, radius, mix) in [
        ("classic", 1.0f32, 1.0f32, 1.0f32),
        ("strong", 3.0, 1.0, 1.0),
        ("wide-radius", 2.0, 3.0, 1.0),
        ("mixed", 2.0, 1.0, 0.6),
        ("amount-zero", 0.0, 1.0, 1.0),
        ("mix-zero", 2.5, 1.0, 0.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::sharpen_simple(&mut cpu, w, h, amount, radius, mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = SharpenSimpleOp {
            amount,
            radius,
            mix,
        };
        let out = fx.sharpen_simple(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("sharpen_simple {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "amount-zero" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact passthrough");
        }

        let out2 = fx.sharpen_simple(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU sharpen_simple must be bit-stable");
    }
}

/// The §1.6 oracle for RGB split: a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, and the GPU is bit-stable (§2.4).
#[test]
fn wgsl_rgb_split_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // Classic red / green / blue tints and one cross-tint case (T17), plus the
    // classic 1 / 0 / 1 scales and asymmetric per-tap scales (FX-9), one
    // negative, to exercise the tinted-tap displacement path.
    let classic_tints = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let cross_tints = [[1.0f32, 1.0, 0.0], [0.2, 0.5, 0.0], [0.0, 0.3, 0.9]];
    for (amount, angle, scale, tints, mix) in [
        (3.0f32, 0.0f32, [1.0f32, 0.0, 1.0], classic_tints, 1.0f32),
        (2.5, 33.0, [1.0, 0.0, 1.0], classic_tints, 0.6),
        (4.0, 0.0, [1.5, 0.25, 0.5], cross_tints, 1.0),
        (3.0, 20.0, [1.2, -0.4, 0.8], classic_tints, 1.0),
        (0.0, 90.0, [1.0, 0.0, 1.0], classic_tints, 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::rgb_split(&mut cpu, w, h, amount, angle, scale, tints, mix);

        let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, angle);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = RgbSplitOp {
            dx,
            dy,
            scale,
            tints,
            mix,
        };
        let out = fx.rgb_split(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("rgb split a={amount} ang={angle} scale={scale:?}: worst {worst} ulp");
        assert!(
            worst <= 2,
            "amount {amount} angle {angle} scale {scale:?} mix {mix}: \
                 worst {worst} fp16 ULP"
        );

        let out2 = fx.rgb_split(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU rgb split must be bit-stable");
    }
}

/// The §1.6 oracle for the RGB split's Wavelength mode (docs/08 §3.6,
/// K-090): both sides accumulate the same nine host-supplied basis
/// weights over the same fp16-quantised taps in f32, in the same order,
/// so the cheap-class ≤ 2 fp16 ULP bound holds despite the longer sum;
/// the GPU is bit-stable (§2.4). The classic mode's oracle above is
/// untouched — separate kernel, separate maths.
#[test]
fn wgsl_spectral_split_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // Sweeps the sample count too (FX-9/K-144): 9 (the historical density), a
    // denser 24, and both range ends, so the variable-count kernel matches.
    // The picker gradient (A1/K-163) is exercised with the default red/green/blue
    // and one custom yellow→magenta→cyan set.
    let rgb = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let custom = [[1.0f32, 1.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
    for (amount, angle, radial, samples, tints, mix) in [
        (3.0f32, 0.0f32, false, 9i32, rgb, 1.0f32),
        (2.5, 33.0, false, 24, rgb, 0.6),
        (4.0, 0.0, true, 16, custom, 1.0),
        (6.0, 10.0, false, 64, rgb, 1.0),
        (5.0, 0.0, true, 3, custom, 1.0),
        (0.0, 90.0, false, 16, rgb, 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::spectral_split(
            &mut cpu, w, h, amount, angle, radial, samples, tints, mix,
        );

        let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, angle);
        let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = SpectralSplitOp {
            dx,
            dy,
            amount_px: amount,
            radial,
            basis,
            count,
            mix,
        };
        let out = fx.spectral_split(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!(
            "spectral split a={amount} ang={angle} radial={radial} n={samples}: worst {worst} ulp"
        );
        assert!(
            worst <= 2,
            "amount {amount} angle {angle} radial {radial} samples {samples} mix {mix}: \
                 worst {worst} fp16 ULP"
        );

        let out2 = fx.spectral_split(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU spectral split must be bit-stable");
    }
}

/// The §1.6 oracle for chromatic aberration: a cheap pointwise effect
/// (a dedicated, always-radial sibling of RGB split's own radial mode),
/// so the CPU and GPU must agree to ≤ 2 fp16 ULP, and the GPU is
/// bit-stable (§2.4). Amount 0 is a bit-exact passthrough through the
/// general formula — no explicit short-circuit, mirroring RGB split's
/// own un-guarded style (asserted here as it is for RGB split above).
#[test]
fn wgsl_chromatic_aberration_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // Default red / green / blue tints (the classic split), plus a custom set
    // where the middle tap leaks colour (P2/K-143) to exercise the tinted sum.
    let rgb: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mixed: [[f32; 3]; 3] = [[1.0, 0.2, 0.0], [0.1, 1.0, 0.1], [0.0, 0.3, 0.9]];
    for (amount, tints, mix) in [
        (3.0f32, rgb, 1.0f32),
        (8.0, rgb, 0.6),
        (12.5, mixed, 1.0),
        (0.0, rgb, 1.0),
        (6.0, rgb, 0.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::chromatic_aberration(&mut cpu, w, h, amount, tints, mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = ChromaticAberrationOp {
            amount_px: amount,
            tints,
            mix,
        };
        let out = fx.chromatic_aberration(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("chromatic aberration a={amount} mix={mix}: worst {worst} ulp");
        assert!(
            worst <= 2,
            "amount {amount} mix {mix}: worst {worst} fp16 ULP"
        );
        // The default red/green/blue tints keep amount 0 / mix 0 a bit-exact
        // passthrough (the tinted sum returns the input for the primaries).
        if tints == rgb && (amount == 0.0 || mix == 0.0) {
            assert_eq!(
                gpu, img,
                "amount 0 or mix 0 must be the bit-exact passthrough"
            );
        }

        let out2 = fx.chromatic_aberration(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU chromatic aberration must be bit-stable");
    }
}

/// The §1.6 oracle for flash: a trivial pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, and the GPU is bit-stable (§2.4).
#[test]
fn wgsl_flash_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (strength, colour, mix) in [
        (1.0f32, [1.0f32, 1.0, 1.0, 1.0], 1.0f32),
        (0.35, [4.0, 2.0, 1.0, 1.0], 1.0), // HDR flash colour
        (0.8, [1.0, 0.9, 0.7, 1.0], 0.6),
        (0.0, [1.0, 1.0, 1.0, 1.0], 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::flash(&mut cpu, strength, colour, mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = FlashOp {
            strength,
            colour,
            mix,
        };
        let out = fx.flash(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("flash s={strength} mix={mix}: worst {worst} ulp");
        assert!(
            worst <= 2,
            "strength {strength} mix {mix}: worst {worst} fp16 ULP"
        );

        let out2 = fx.flash(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU flash must be bit-stable");
    }
}

/// The §1.6 oracle for colour balance: a cheap pointwise effect, so the
/// CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4),
/// and — the K-090 split's promise — a fully neutral balance is the
/// bit-exact identity on both paths.
#[test]
fn wgsl_colour_balance_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let neutral = ColourBalanceOp {
        lift: [0.0; 3],
        gamma: [1.0; 3],
        gain: [1.0; 3],
        mix: 1.0,
    };
    let teal_orange = ColourBalanceOp {
        lift: [-0.02, 0.0, 0.02],
        gamma: [1.1, 1.0, 0.9],
        gain: [1.2, 1.0, 0.8],
        mix: 1.0,
    };
    let extreme = ColourBalanceOp {
        lift: [0.1; 3],
        gamma: [2.2, 0.6, 1.7],
        gain: [2.0, 0.5, 1.5],
        mix: 0.7,
    };
    for (name, op) in [
        ("neutral", neutral),
        ("teal-orange", teal_orange),
        ("extreme", extreme),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::colour_balance(&mut cpu, op.lift, op.gamma, op.gain, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.colour_balance(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("colour balance {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" {
            assert_eq!(gpu, img, "neutral balance must be the bit-exact identity");
        }

        let out2 = fx.colour_balance(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU colour balance must be bit-stable");
    }
}

/// The §1.6 oracle for saturation: a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4),
/// and saturation 1 is the bit-exact identity on both paths.
#[test]
fn wgsl_saturation_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, op) in [
        (
            "neutral",
            SaturationOp {
                saturation: 1.0,
                mix: 1.0,
            },
        ),
        (
            "greyscale",
            SaturationOp {
                saturation: 0.0,
                mix: 1.0,
            },
        ),
        (
            "boosted",
            SaturationOp {
                saturation: 1.6,
                mix: 1.0,
            },
        ),
        (
            // K-135: above the old 200 % cap — the kernel does not clamp, it
            // keeps extrapolating, so CPU/GPU parity must still hold here.
            "heavy",
            SaturationOp {
                saturation: 3.5,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            SaturationOp {
                saturation: 0.3,
                mix: 0.6,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::saturate(&mut cpu, op.saturation, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.saturation(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("saturation {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" {
            assert_eq!(
                gpu, img,
                "neutral saturation must be the bit-exact identity"
            );
        }

        let out2 = fx.saturation(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU saturation must be bit-stable");
    }
}

/// The §1.6 oracle for vibrancy (K-152): a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4), and
/// amount 0 is the bit-exact identity on both paths.
#[test]
fn wgsl_vibrancy_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, op) in [
        (
            "neutral",
            VibrancyOp {
                amount: 0.0,
                mix: 1.0,
            },
        ),
        (
            "gentle",
            VibrancyOp {
                amount: 0.5,
                mix: 1.0,
            },
        ),
        (
            // K-135: above 100 % — the per-pixel factor keeps extrapolating,
            // so CPU/GPU parity must still hold here.
            "heavy",
            VibrancyOp {
                amount: 2.0,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            VibrancyOp {
                amount: 1.0,
                mix: 0.6,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::vibrance(&mut cpu, op.amount, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.vibrancy(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("vibrancy {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" {
            assert_eq!(gpu, img, "neutral vibrancy must be the bit-exact identity");
        }

        let out2 = fx.vibrancy(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU vibrancy must be bit-stable");
    }
}

/// The §1.6 oracle for matte key: a cheap pointwise Keylight-style keyer, so
/// the CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4),
/// and Mix 0 is the bit-exact identity on both paths. The corpus mixes
/// near-screen greens, far-from-screen colours, partial-alpha (premultiplied)
/// pixels and an HDR spike; the settings sweep gain / balance / despill /
/// clips / replace method / bias colours and the three View modes so the
/// screen-matte, clip, despill, replace and diagnostic paths are all
/// exercised.
#[test]
fn wgsl_matte_key_matches_the_cpu_oracle() {
    use lumit_core::fx::MatteKeyParams;
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    // Corpus (§1.6): a green field on the left sliding to red/magenta on
    // the right, brightness rising down the frame, alpha in bands 0.25..1
    // so the unpremultiply round trip is load-bearing, plus an HDR
    // partial-alpha spike.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let fx_ = x as f32 / (w - 1) as f32;
            let fy = y as f32 / (h - 1) as f32;
            let r = fx_;
            let g = (1.0 - fx_) * (0.4 + 0.6 * fy);
            let b = 0.25 * fx_;
            let a = 0.25 + 0.75 * fy;
            img[i] = r * a;
            img[i + 1] = g * a;
            img[i + 2] = b * a;
            img[i + 3] = a;
        }
    }
    let spike = ((10 * w + 20) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 0.5]);
    let img: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();

    let grey = [0.5f32, 0.5, 0.5, 1.0];
    // A base op mirroring the schema defaults; each case overrides a field or two.
    let base = MatteKeyParams {
        view: 0,
        key: [0.0, 0.6, 0.0, 1.0],
        gain: 1.0,
        balance: 0.5,
        despill_bias: grey,
        alpha_bias: grey,
        spill: 1.0,
        clip_black: 0.0,
        clip_white: 1.0,
        clip_rollback: 0.0,
        replace_method: 2,
        replace_colour: grey,
        mix: 1.0,
    };
    let to_op = |p: &MatteKeyParams| MatteKeyOp {
        view: p.view,
        key: p.key,
        gain: p.gain,
        balance: p.balance,
        despill_bias: p.despill_bias,
        alpha_bias: p.alpha_bias,
        spill: p.spill,
        clip_black: p.clip_black,
        clip_white: p.clip_white,
        clip_rollback: p.clip_rollback,
        replace_method: p.replace_method,
        replace_colour: p.replace_colour,
        mix: p.mix,
    };

    for (name, p) in [
        ("default_soft", base),
        (
            "high_gain_low_balance",
            MatteKeyParams {
                gain: 1.6,
                balance: 0.15,
                ..base
            },
        ),
        (
            "clips_and_rollback",
            MatteKeyParams {
                clip_black: 0.15,
                clip_white: 0.85,
                clip_rollback: 0.4,
                ..base
            },
        ),
        (
            "hard_replace_tinted_bias",
            MatteKeyParams {
                replace_method: 1,
                replace_colour: [0.2, 0.1, 0.4, 1.0],
                despill_bias: [0.6, 0.5, 0.4, 1.0],
                alpha_bias: [0.55, 0.5, 0.45, 1.0],
                ..base
            },
        ),
        (
            "source_replace_no_spill",
            MatteKeyParams {
                replace_method: 0,
                spill: 0.0,
                ..base
            },
        ),
        ("screen_matte_view", MatteKeyParams { view: 1, ..base }),
        ("status_view", MatteKeyParams { view: 2, ..base }),
        (
            "identity_mix0",
            MatteKeyParams {
                spill: 0.4,
                mix: 0.0,
                ..base
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::matte_key(&mut cpu, &p);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = to_op(&p);
        let out = fx.matte_key(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("matte key {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "identity_mix0" {
            assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
        }

        let out2 = fx.matte_key(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU matte key must be bit-stable");
    }
}

/// The §1.6 oracle for vignette: a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4), and
/// Amount 0 (or Mix 0) is the bit-exact identity on both paths.
#[test]
fn wgsl_vignette_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, op) in [
        (
            "neutral",
            VignetteOp {
                amount: 0.0,
                radius: 0.75,
                softness: 0.5,
                roundness: 1.0,
                ramp: 1.0,
                mix: 1.0,
            },
        ),
        (
            "tight-circular",
            VignetteOp {
                amount: 1.0,
                radius: 0.3,
                softness: 0.1,
                roundness: 1.0,
                ramp: 1.0,
                mix: 1.0,
            },
        ),
        (
            "soft-elliptical",
            VignetteOp {
                amount: 0.6,
                radius: 0.5,
                softness: 0.4,
                roundness: 0.0,
                ramp: 1.0,
                mix: 1.0,
            },
        ),
        (
            // K-135: Softness > 1 is a legal, wider feather — the kernel does
            // not clamp it to 1, so CPU/GPU parity must hold for it too.
            "wide-feather",
            VignetteOp {
                amount: 0.9,
                radius: 0.3,
                softness: 1.6,
                roundness: 1.0,
                ramp: 1.0,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            VignetteOp {
                amount: 0.8,
                radius: 0.6,
                softness: 0.3,
                roundness: 0.5,
                // Non-identity ramp (T16): exercises the gamma path, not just ramp == 1.
                ramp: 2.0,
                mix: 0.5,
            },
        ),
        (
            "mix-zero",
            VignetteOp {
                amount: 0.9,
                radius: 0.2,
                softness: 0.05,
                roundness: 1.0,
                ramp: 1.0,
                mix: 0.0,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::vignette(
            &mut cpu,
            w,
            h,
            op.amount,
            op.radius,
            op.softness,
            op.roundness,
            op.ramp,
            op.mix,
        );

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.vignette(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("vignette {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.vignette(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU vignette must be bit-stable");
    }
}

/// The §1.6 oracle for exposure: a cheap pointwise gain, so CPU and GPU
/// must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and 0 stops
/// (`factor` 1.0) or Mix 0 is the bit-exact identity on both paths.
#[test]
fn wgsl_exposure_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, op) in [
        (
            "neutral",
            ExposureOp {
                factor: 1.0,
                mix: 1.0,
            },
        ),
        (
            "brighten",
            ExposureOp {
                factor: 2.0,
                mix: 1.0,
            },
        ),
        (
            "darken",
            ExposureOp {
                factor: 0.5,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            ExposureOp {
                factor: 1.7,
                mix: 0.5,
            },
        ),
        (
            "mix-zero",
            ExposureOp {
                factor: 3.0,
                mix: 0.0,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::exposure(&mut cpu, op.factor, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.exposure(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("exposure {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.exposure(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU exposure must be bit-stable");
    }
}

/// The §1.6 oracle for temperature: a cheap pointwise per-channel R/B gain,
/// so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and
/// temperature 0 (gains `(1.0, 1.0)`) or Mix 0 is the bit-exact identity on
/// both paths. The gains are the host-computed `max(0, 1 ± 0.75·k)` for `k =
/// temperature / 100` (K-135), so the CPU and kernel multiply by identical
/// numbers.
/// The corpus is seeded with partial-alpha pixels too — unlike Contrast the
/// multiply commutes with premultiplied alpha (no unpremultiply wrap), and
/// this pins that: a fractional-alpha pixel comes out identical on both.
#[test]
fn wgsl_temperature_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    // Start from the shared corpus (gradient + alpha edge + HDR spike),
    // then inject partial-alpha pixels: straight colour stored
    // premultiplied, quantised to f16 so both paths begin identical.
    let mut img = corpus(w, h);
    let q = |v: f32| f16_to_f32(f16_bits(v));
    let partials = [
        // (straight rgb, alpha)
        ([0.7_f32, 0.3, 0.5], 0.5_f32),
        ([0.2, 0.8, 0.6], 0.25),
        ([0.9, 0.1, 0.4], 0.75),
        ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
    ];
    for (n, (rgb, a)) in partials.iter().enumerate() {
        let i = n * 4; // the first four pixels of row 0
        img[i] = q(rgb[0] * a);
        img[i + 1] = q(rgb[1] * a);
        img[i + 2] = q(rgb[2] * a);
        img[i + 3] = q(*a);
    }
    // Host-compute the gains exactly as the resolve step does (K-135: the
    // stronger ±0.75·k gain, k clamped to ±2, gains floored at 0), over a
    // spread that reaches the new ±150/±200 extremes and the blue-gain floor.
    let gains = |temperature: f32| {
        let k = (temperature / 100.0).clamp(-2.0, 2.0);
        ((1.0 + 0.75 * k).max(0.0), (1.0 - 0.75 * k).max(0.0))
    };
    for (name, temp, mix) in [
        ("neutral", 0.0, 1.0),
        ("warm", 120.0, 1.0),
        ("cool", -120.0, 1.0),
        ("floor", 200.0, 1.0), // blue gain floored at 0
        ("mixed", 60.0, 0.5),
        ("mix-zero", 100.0, 0.0),
    ] {
        let (gain_r, gain_b) = gains(temp);
        let op = TemperatureOp {
            gain_r,
            gain_b,
            mix,
        };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::temperature(&mut cpu, op.gain_r, op.gain_b, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.temperature(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("temperature {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.temperature(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU temperature must be bit-stable");
    }
}

/// A corpus (§1.6) that seeds the shared gradient + alpha edge + HDR spike
/// with partial-alpha pixels: straight colour stored premultiplied, quantised
/// to f16 so both paths begin identical. The unpremultiply round trip is
/// load-bearing for the affine colour effects (Invert, Tint), so a naive pass
/// on premultiplied colour would diverge exactly on these pixels.
fn corpus_with_partials(w: u32, h: u32) -> Vec<f32> {
    let mut img = corpus(w, h);
    let q = |v: f32| f16_to_f32(f16_bits(v));
    let partials = [
        // (straight rgb, alpha)
        ([0.7_f32, 0.3, 0.5], 0.5_f32),
        ([0.2, 0.8, 0.6], 0.25),
        ([0.9, 0.1, 0.4], 0.75),
        ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
    ];
    for (n, (rgb, a)) in partials.iter().enumerate() {
        let i = n * 4; // the first four pixels of row 0
        img[i] = q(rgb[0] * a);
        img[i + 1] = q(rgb[1] * a);
        img[i + 2] = q(rgb[2] * a);
        img[i + 3] = q(*a);
    }
    img
}

/// The §1.6 oracle for invert: a cheap pointwise colour inverse, so CPU and
/// GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Mix 0 is the
/// bit-exact identity on both paths. The corpus carries partial-alpha pixels
/// (invert runs on unpremultiplied colour, so the premultiply round trip is
/// load-bearing) and the HDR spike (which inverts to honest negatives, never
/// clipped). There is no neutral value, so the only identity case is Mix 0.
#[test]
fn wgsl_invert_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus_with_partials(w, h);
    for (name, op) in [
        ("full", InvertOp { mix: 1.0 }),
        ("mixed", InvertOp { mix: 0.5 }),
        ("mix-zero", InvertOp { mix: 0.0 }),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::invert(&mut cpu, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.invert(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("invert {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
        }

        let out2 = fx.invert(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU invert must be bit-stable");
    }
}

/// The §1.6 oracle for tint: a cheap pointwise luminance duotone, so CPU and
/// GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Mix 0 is the
/// bit-exact identity on both paths. The corpus carries partial-alpha pixels
/// (the luma-driven remap runs on unpremultiplied colour, so the premultiply
/// round trip is load-bearing). Settings sweep the default greyscale
/// (black→black, white→white) and a coloured duotone; the lerp is the
/// `black + (white − black)·luma` form on both paths so they reduce alike.
#[test]
fn wgsl_tint_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus_with_partials(w, h);
    for (name, op) in [
        (
            "greyscale",
            TintOp {
                black: [0.0, 0.0, 0.0],
                white: [1.0, 1.0, 1.0],
                mix: 1.0,
            },
        ),
        (
            "duotone",
            TintOp {
                black: [0.1, 0.05, 0.3],
                white: [1.0, 0.9, 0.6],
                mix: 1.0,
            },
        ),
        (
            "mixed",
            TintOp {
                black: [0.2, 0.0, 0.4],
                white: [0.8, 1.0, 0.5],
                mix: 0.5,
            },
        ),
        (
            "mix-zero",
            TintOp {
                black: [0.1, 0.05, 0.3],
                white: [1.0, 0.9, 0.6],
                mix: 0.0,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::tint(&mut cpu, op.black, op.white, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.tint(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("tint {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
        }

        let out2 = fx.tint(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU tint must be bit-stable");
    }
}

/// The §1.6 oracle for contrast: a cheap pointwise affine grade about
/// mid-grey, so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is
/// bit-stable, and Contrast 100 % (`k` 1.0) or Mix 0 is the bit-exact
/// identity on both paths. The corpus is seeded with partial-alpha pixels
/// (straight colour × alpha), since the affine grade runs on
/// unpremultiplied colour and the − pivot offset makes the premultiply
/// round trip load-bearing — a naive grade on premultiplied colour would
/// diverge exactly there.
#[test]
fn wgsl_contrast_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    // Start from the shared corpus (gradient + alpha edge + HDR spike),
    // then inject partial-alpha pixels: straight colour graded, stored
    // premultiplied, quantised to f16 so both paths begin identical.
    let mut img = corpus(w, h);
    let q = |v: f32| f16_to_f32(f16_bits(v));
    let partials = [
        // (straight rgb, alpha)
        ([0.7_f32, 0.3, 0.5], 0.5_f32),
        ([0.2, 0.8, 0.6], 0.25),
        ([0.9, 0.1, 0.4], 0.75),
        ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
    ];
    for (n, (rgb, a)) in partials.iter().enumerate() {
        let i = n * 4; // the first four pixels of row 0
        img[i] = q(rgb[0] * a);
        img[i + 1] = q(rgb[1] * a);
        img[i + 2] = q(rgb[2] * a);
        img[i + 3] = q(*a);
    }
    for (name, op) in [
        ("neutral", ContrastOp { k: 1.0, mix: 1.0 }),
        ("boosted", ContrastOp { k: 1.8, mix: 1.0 }),
        ("flattened", ContrastOp { k: 0.4, mix: 1.0 }),
        ("mixed", ContrastOp { k: 1.5, mix: 0.6 }),
        ("mix-zero", ContrastOp { k: 2.0, mix: 0.0 }),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::contrast(&mut cpu, op.k, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.contrast(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("contrast {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.contrast(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU contrast must be bit-stable");
    }
}

/// The §1.6 oracle for gamma: a cheap pointwise power curve, so CPU and GPU
/// must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and gamma 1.0 or Mix 0
/// is the bit-exact identity on both paths. Like Contrast, the corpus is
/// seeded with partial-alpha pixels (straight colour × alpha), since the
/// curve runs on unpremultiplied colour and the premultiply round trip is
/// load-bearing — a naive curve on premultiplied colour would diverge there.
#[test]
fn wgsl_gamma_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    // Start from the shared corpus (gradient + alpha edge + HDR spike),
    // then inject partial-alpha pixels: straight colour curved, stored
    // premultiplied, quantised to f16 so both paths begin identical.
    let mut img = corpus(w, h);
    let q = |v: f32| f16_to_f32(f16_bits(v));
    let partials = [
        // (straight rgb, alpha)
        ([0.7_f32, 0.3, 0.5], 0.5_f32),
        ([0.2, 0.8, 0.6], 0.25),
        ([0.9, 0.1, 0.4], 0.75),
        ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
    ];
    for (n, (rgb, a)) in partials.iter().enumerate() {
        let i = n * 4; // the first four pixels of row 0
        img[i] = q(rgb[0] * a);
        img[i + 1] = q(rgb[1] * a);
        img[i + 2] = q(rgb[2] * a);
        img[i + 3] = q(*a);
    }
    for (name, op) in [
        (
            "neutral",
            GammaOp {
                gamma: 1.0,
                mix: 1.0,
            },
        ),
        (
            "encode",
            GammaOp {
                gamma: 0.45,
                mix: 1.0,
            },
        ),
        (
            "decode",
            GammaOp {
                gamma: 2.2,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            GammaOp {
                gamma: 2.2,
                mix: 0.6,
            },
        ),
        (
            "mix-zero",
            GammaOp {
                gamma: 2.2,
                mix: 0.0,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::gamma(&mut cpu, op.gamma, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.gamma(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("gamma {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.gamma(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU gamma must be bit-stable");
    }
}

/// The §1.6 oracle for hue shift: a cheap pointwise colour-matrix product,
/// so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and
/// 0° (the identity matrix) or Mix 0 is the bit-exact identity on both.
#[test]
fn wgsl_hue_shift_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // K-136: both matrix branches — the constant-luminance rotation
    // (`preserve = true`, `hue_matrix`) and the plain-RGB spin
    // (`preserve = false`, `hue_matrix_rgb`) — feed the one matrix-general
    // kernel, so parity must hold for each.
    for (name, deg, mix, preserve) in [
        ("neutral", 0.0, 1.0, true),
        ("quarter", 90.0, 1.0, true),
        ("half", 180.0, 1.0, true),
        ("mixed", 45.0, 0.5, true),
        ("mix-zero", 120.0, 0.0, true),
        ("rgb-neutral", 0.0, 1.0, false),
        ("rgb-quarter", 90.0, 1.0, false),
        ("rgb-mixed", 45.0, 0.5, false),
        ("rgb-mix-zero", 120.0, 0.0, false),
    ] {
        let m = if preserve {
            lumit_core::fx::hue_matrix(deg)
        } else {
            lumit_core::fx::hue_matrix_rgb(deg)
        };
        let op = HueShiftOp { m, mix };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::hue_shift(&mut cpu, op.m, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.hue_shift(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("hue_shift {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if deg % 360.0 == 0.0 || mix == 0.0 {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.hue_shift(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU hue shift must be bit-stable");
    }
}

/// The §1.6 oracle for the transform effect: a trivial one-tap resample,
/// so the CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable
/// (§2.4), and — the docs/08 §3.5 pin — identity parameters reproduce
/// the input bit-exactly.
#[test]
fn wgsl_transform_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let centre = [w as f32 * 0.5, h as f32 * 0.5];
    // The last column is the Edges policy (P3, K-145): the Transform effect
    // itself always passes 0, but Shake dispatches this same kernel with 1
    // (Repeat) and 2 (Mirror), so the oracle exercises all three here.
    for (name, anchor, position, scale, rotation, opacity, mix, edge) in [
        (
            "identity", [0.0; 2], [0.0; 2], [1.0; 2], 0.0, 1.0, 1.0, 0u32,
        ),
        ("shift", [0.0; 2], [2.5, -1.5], [1.0; 2], 0.0, 1.0, 1.0, 0),
        ("punch-in", centre, centre, [1.4, 1.4], 12.0, 1.0, 1.0, 0),
        ("flip-fade", centre, centre, [-1.0, 1.0], 0.0, 0.5, 0.8, 0),
        ("collapsed", centre, centre, [0.0, 1.0], 0.0, 1.0, 0.6, 0),
        (
            "shift-repeat",
            [0.0; 2],
            [5.0, -4.0],
            [1.0; 2],
            0.0,
            1.0,
            1.0,
            1,
        ),
        ("spin-mirror", centre, centre, [1.0; 2], 8.0, 1.0, 1.0, 2),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::transform(
            &mut cpu, w, h, anchor, position, scale, rotation, edge, opacity, mix,
        );

        let (m, off, opacity) =
            lumit_core::fx::transform_op(anchor, position, scale, rotation, opacity);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = TransformOp {
            m,
            off,
            opacity,
            mix,
            edge,
        };
        let out = fx.transform(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("transform {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "identity" {
            assert_eq!(
                gpu, img,
                "identity transform must be the bit-exact passthrough"
            );
        }

        let out2 = fx.transform(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU transform must be bit-stable");
    }
}

/// The §1.6 oracle for shake (docs/08 §3.4): a transform-domain effect
/// with no kernel of its own — the resolved wobble maps through the
/// shared `shake_affine` to the Transform kernel, exactly as `run_ops`
/// dispatches it, and the CPU reference walks the same affine. One-tap
/// resample, so the cheap-class ≤ 2 fp16 ULP bound holds; the GPU is
/// bit-stable (§2.4); the neutral wobble (zero offset, rotation and z
/// shake) is the bit-exact passthrough. The Edges control (P3, K-145) is
/// swept across Transparent / Repeat / Mirror so the kernel's border
/// handling is covered on both paths.
#[test]
fn wgsl_shake_matches_the_cpu_oracle_through_the_transform_kernel() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, offset, rot, zoom, edge, mix) in [
        ("neutral", [0.0f32, 0.0f32], 0.0f32, 1.0f32, 1u32, 1.0f32),
        ("offset", [2.5, -1.5], 0.0, 1.0, 0, 1.0),
        ("twist-repeat", [1.0, 0.5], 4.0, 1.0, 1, 1.0),
        ("pumped-mirror", [0.0, 2.0], -2.0, 0.95, 2, 0.7),
    ] {
        let shake = lumit_core::fx::Resolved::Shake {
            offset_px: offset,
            rotation_deg: rot,
            zoom,
            edge,
            mix,
            mb: None,
        };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::apply(&mut cpu, w, h, &shake);

        // The exact run_ops mapping: shared affine → transform op →
        // the Transform kernel, carrying the Edges policy.
        let (anchor, position, scale, rotation) =
            lumit_core::fx::shake_affine(w, h, offset, rot, zoom);
        let (m, off, opacity) =
            lumit_core::fx::transform_op(anchor, position, scale, rotation, 1.0);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = TransformOp {
            m,
            off,
            opacity,
            mix,
            edge,
        };
        let out = fx.transform(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("shake {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" {
            assert_eq!(
                gpu, img,
                "a neutral shake must be the bit-exact passthrough"
            );
        }

        let out2 = fx.transform(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU shake must be bit-stable");
    }
}

/// The §1.6 oracle for the shake's own motion blur (docs/08 §3.4, T18/K-165):
/// the `fx_shake_mb` kernel averages the wobble resampled at its motion-blur
/// sub-frames, and must agree with `cpu::transform_average` (reached through
/// `cpu::apply` on a `Resolved::Shake` carrying the sub-frames). The sub-frames
/// come from the shared `ShakeWobble`/`shake_mb_offsets` the resolver uses, so
/// this exercises the whole T18 path. One bilinear tap per sub-frame, so the
/// cheap-class ≤ 2 fp16 ULP bound holds; the GPU is bit-stable (§2.4). The Edges
/// control is swept across Transparent / Repeat / Mirror.
#[test]
fn wgsl_shake_motion_blur_matches_the_cpu_oracle() {
    // The GPU crate can't name lumit-core's const (dev-dependency only), so pin
    // the two — and the WGSL `array<Tap, 9>` literal — in agreement here.
    assert_eq!(SHAKE_MB_SAMPLES, lumit_core::fx::SHAKE_MB_SAMPLES);

    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);

    // A realistic sub-frame set: the same sampler the resolver uses, spread
    // across the shutter, with rotation and a touch of z (depth) shake so the
    // average smears translation, rotation and zoom together.
    let wobble = lumit_core::fx::ShakeWobble {
        seed: 7,
        amp_px: 6.0,
        x_amp: 1.0,
        y_amp: 1.0,
        rot_amount: 4.0,
        z_amp: 0.08,
        x_freq: 1.0,
        y_freq: 1.3,
        z_freq: 0.7,
    };
    let base = 2.0f64;
    let offsets = lumit_core::fx::shake_mb_offsets(0.8);
    let mut samples = [lumit_core::fx::ShakeSample::IDENTITY; SHAKE_MB_SAMPLES];
    for (s, db) in samples.iter_mut().zip(offsets) {
        let (offset_px, rotation_deg, zoom) = wobble.at(base + db);
        *s = lumit_core::fx::ShakeSample {
            offset_px,
            rotation_deg,
            zoom,
        };
    }
    let centre = samples[SHAKE_MB_SAMPLES / 2];

    for (name, edge, mix) in [
        ("smear-transparent", 0u32, 1.0f32),
        ("smear-repeat", 1, 1.0),
        ("smear-mirror-mixed", 2, 0.7),
    ] {
        let shake = lumit_core::fx::Resolved::Shake {
            offset_px: centre.offset_px,
            rotation_deg: centre.rotation_deg,
            zoom: centre.zoom,
            edge,
            mix,
            mb: Some(samples),
        };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::apply(&mut cpu, w, h, &shake);

        // The exact run_ops mapping: each sub-frame's shared affine → transform
        // op → one tap of the averaging kernel.
        let mut taps = [ShakeMbTap {
            m: [1.0, 0.0, 0.0, 1.0],
            off: [0.0, 0.0],
        }; SHAKE_MB_SAMPLES];
        for (t, s) in taps.iter_mut().zip(samples.iter()) {
            let (anchor, position, scale, rotation) =
                lumit_core::fx::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
            let (m, off, _opacity) =
                lumit_core::fx::transform_op(anchor, position, scale, rotation, 1.0);
            *t = ShakeMbTap { m, off };
        }
        let op = ShakeMbOp {
            taps,
            count: SHAKE_MB_SAMPLES as u32,
            edge,
            mix,
        };
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.shake_mb(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("shake-mb {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        assert_ne!(gpu, img, "{name}: the motion blur moves pixels");

        let out2 = fx.shake_mb(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU shake motion blur must be bit-stable");
    }

    // A single tap equal to the frame wobble is the plain Shake: the averaging
    // kernel at count 1 matches the Transform kernel within the cheap bound.
    let (anchor, position, scale, rotation) =
        lumit_core::fx::shake_affine(w, h, centre.offset_px, centre.rotation_deg, centre.zoom);
    let (m, off, opacity) = lumit_core::fx::transform_op(anchor, position, scale, rotation, 1.0);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let mut taps = [ShakeMbTap {
        m: [1.0, 0.0, 0.0, 1.0],
        off: [0.0, 0.0],
    }; SHAKE_MB_SAMPLES];
    taps[0] = ShakeMbTap { m, off };
    let single = fx.shake_mb(
        &ctx,
        &tex,
        w,
        h,
        &ShakeMbOp {
            taps,
            count: 1,
            edge: 1,
            mix: 1.0,
        },
    );
    let single_gpu = readback_linear_f32(&ctx, &single, w, h).unwrap();
    let plain = fx.transform(
        &ctx,
        &tex,
        w,
        h,
        &TransformOp {
            m,
            off,
            opacity,
            mix: 1.0,
            edge: 1,
        },
    );
    let plain_gpu = readback_linear_f32(&ctx, &plain, w, h).unwrap();
    let worst = worst_f16_ulp(&single_gpu, &plain_gpu);
    assert!(
        worst <= 2,
        "count-1 motion blur == plain shake: worst {worst}"
    );
}

/// The §1.6 oracle for glow: WGSL agrees with the CPU reference on the
/// corpus across parameter sweeps, is bit-stable (§2.4), and — the
/// effect's neutral pin — intensity 0 is the bit-exact identity. Like
/// sharpen, the internal gaussian's intermediates round through fp16
/// textures on the GPU and stay f32 on the CPU, so the bound is an
/// absolute epsilon rather than a ULP count: 5e-3 ≈ 1–2 fp16 ULP at the
/// corpus's HDR peak of 6.0 (measured worst on NVIDIA: 1.5e-3, on the
/// hard-knee case where the bright stage passes the most energy).
#[test]
fn wgsl_glow_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, radius, threshold, knee, intensity, tint, mix) in [
        (
            // The schema default threshold is now 0.8 (K-135/FX-16); radius
            // here is already raster px (GlowOp is post-resolve), so the
            // %-diag → px@comp change lives in the resolve step, not here.
            "default",
            6.0f32,
            0.8f32,
            0.5f32,
            1.0f32,
            [1.0f32; 4],
            1.0f32,
        ),
        ("hard-knee", 4.0, 0.5, 0.0, 2.0, [1.0; 4], 1.0),
        ("threshold-0", 8.0, 0.0, 0.0, 1.0, [1.0; 4], 1.0),
        (
            "tinted-mixed",
            5.0,
            0.3,
            0.2,
            1.5,
            [2.0, 0.5, 0.25, 1.0],
            0.6,
        ),
        ("neutral", 6.0, 1.0, 0.5, 0.0, [1.0; 4], 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::glow(
            &mut cpu, w, h, radius, threshold, knee, intensity, tint, mix,
        );

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = GlowOp {
            radius_px: radius,
            threshold,
            knee,
            intensity,
            tint,
            mix,
        };
        let out = fx.glow(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_diff(&cpu, &gpu);
        // Logged so real cross-vendor deltas accumulate (docs/08 open
        // question 5: the class tolerances are placeholders until then).
        eprintln!("glow {name}: worst {worst:.2e}");
        assert!(worst < 5e-3, "{name}: worst diff {worst}");
        if name == "neutral" {
            assert_eq!(gpu, img, "intensity 0 must be the bit-exact identity");
        }

        let out2 = fx.glow(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU glow must be bit-stable");
    }
}

/// The §1.6 oracle for Block glitch (docs/08 §3.12, split out by K-107):
/// WGSL agrees with the CPU reference across intensity, seed, tick and
/// the full parameter set, and is bit-stable (§2.4). Mirrors the old
/// combined Glitch oracle's structure — same maths, just without the
/// scanline section and its toggle. The per-block hash is exact integer
/// maths on both sides (`splitmix32`), so the bound stays as tight as
/// the other hash/tap-based kernels; intensity 0 is asserted bit-exact
/// against the untouched corpus regardless of Mix, matching the CPU
/// reference's early return.
#[test]
fn wgsl_block_glitch_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);

    struct Case {
        name: &'static str,
        intensity: f32,
        seed: u32,
        tick: i32,
        block_size_px: f32,
        jitter_frac: f32,
        amount_px: f32,
        chan_px: f32,
        slice_frac: f32,
        mix: f32,
    }
    let cases = [
        Case {
            name: "neutral-intensity0",
            intensity: 0.0,
            seed: 7,
            tick: 3,
            block_size_px: 6.0,
            jitter_frac: 0.5,
            amount_px: 5.0,
            chan_px: 2.0,
            slice_frac: 0.5,
            mix: 0.4,
        },
        Case {
            name: "moderate",
            intensity: 0.7,
            seed: 11,
            tick: 4,
            block_size_px: 6.0,
            jitter_frac: 0.3,
            amount_px: 4.0,
            chan_px: 1.5,
            slice_frac: 0.4,
            mix: 1.0,
        },
        Case {
            name: "full-partial-mix",
            intensity: 1.0,
            seed: 99,
            tick: 12,
            block_size_px: 5.0,
            jitter_frac: 1.0,
            amount_px: 8.0,
            chan_px: 3.0,
            slice_frac: 1.0,
            mix: 0.6,
        },
    ];

    for case in cases {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::block_glitch(
            &mut cpu,
            w,
            h,
            case.intensity,
            case.seed,
            case.tick,
            case.block_size_px,
            case.jitter_frac,
            case.amount_px,
            case.chan_px,
            case.slice_frac,
            case.mix,
        );

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = BlockGlitchOp {
            intensity: case.intensity,
            seed: case.seed,
            tick: case.tick,
            block_size_px: case.block_size_px,
            jitter_frac: case.jitter_frac,
            amount_px: case.amount_px,
            chan_px: case.chan_px,
            slice_frac: case.slice_frac,
            mix: case.mix,
        };
        let out = fx.block_glitch(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("block_glitch {}: worst {worst} ulp", case.name);
        assert!(worst <= 2, "{}: worst {worst} fp16 ULP", case.name);
        if case.name == "neutral-intensity0" {
            assert_eq!(gpu, img, "{}: must be the bit-exact passthrough", case.name);
        }

        let out2 = fx.block_glitch(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU block_glitch must be bit-stable");
    }
}

/// The §1.6 oracle for Scanlines (docs/08 §3.12, split out by K-107; single
/// Intensity since FX-13/K-147): WGSL agrees with the CPU reference across
/// intensity, period, roll and interlace, and is bit-stable (§2.4). Intensity
/// is now the sole darken dial (dark lines reach black at 1). Intensity 0 is
/// asserted bit-exact against the untouched corpus regardless of Mix,
/// matching the CPU reference's early return.
#[test]
fn wgsl_scanlines_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);

    struct Case {
        name: &'static str,
        intensity: f32,
        period_px: f32,
        roll_px: f32,
        interlace: bool,
        mix: f32,
    }
    let cases = [
        Case {
            name: "neutral-intensity0",
            intensity: 0.0,
            period_px: 3.0,
            roll_px: 1.0,
            interlace: true,
            mix: 0.4,
        },
        Case {
            name: "moderate",
            intensity: 0.8,
            period_px: 4.0,
            roll_px: 2.5,
            interlace: true,
            mix: 1.0,
        },
        Case {
            name: "full-partial-mix-no-interlace",
            intensity: 1.0,
            period_px: 2.5,
            roll_px: -1.5,
            interlace: false,
            mix: 0.6,
        },
    ];

    for case in cases {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::scanlines(
            &mut cpu,
            w,
            h,
            case.intensity,
            case.period_px,
            case.roll_px,
            case.interlace,
            case.mix,
        );

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = ScanlinesOp {
            intensity: case.intensity,
            period_px: case.period_px,
            roll_px: case.roll_px,
            interlace: case.interlace,
            mix: case.mix,
        };
        let out = fx.scanlines(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("scanlines {}: worst {worst} ulp", case.name);
        assert!(worst <= 2, "{}: worst {worst} fp16 ULP", case.name);
        if case.name == "neutral-intensity0" {
            assert_eq!(gpu, img, "{}: must be the bit-exact passthrough", case.name);
        }

        let out2 = fx.scanlines(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU scanlines must be bit-stable");
    }
}

/// The §1.6 oracle for the directional blur mode: WGSL agrees with the
/// CPU reference on the corpus per edge policy, and is bit-stable
/// (§2.4). Both sides accumulate the same taps in f32 from the same
/// fp16-quantised input, so the bound is tight even for this
/// moderate-class kernel; the gaussian mode's own oracle is untouched
/// above (same kernel, byte-identical maths).
#[test]
fn wgsl_dir_blur_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for edge in [0u32, 1, 2] {
        for (length, angle, mix) in [(6.0f32, 0.0f32, 1.0f32), (9.5, 33.0, 0.6), (0.0, 90.0, 1.0)] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::blur_directional(&mut cpu, w, h, length, angle, edge, mix);

            let (dx, dy) = lumit_core::fx::rgb_split_offset(1.0, angle);
            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = DirBlurOp {
                dx,
                dy,
                length_px: length,
                taps: lumit_core::fx::cpu::dir_blur_taps(length),
                edge,
                mix,
            };
            let out = fx.dir_blur(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("dir blur e={edge} l={length} a={angle}: worst {worst} ulp");
            assert!(
                worst <= 2,
                "edge {edge} length {length} angle {angle} mix {mix}: \
                     worst {worst} fp16 ULP"
            );

            let out2 = fx.dir_blur(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU directional blur must be bit-stable");
        }
    }
}

/// The §1.6 oracle for Blur's Radial mode (docs/08 §3.8, schema status
/// note): WGSL agrees with the CPU reference across Spin and Zoom,
/// off-centre Centres, several amounts and edge policies, and is
/// bit-stable (§2.4). Neither side runs a per-tap trig call or a
/// division (the schema note's whole point), so the bound stays as
/// tight as the directional blur's; amount 0 is asserted bit-exact
/// against the untouched corpus (mirroring the directional blur's own
/// zero-length case) — the gaussian and directional oracles above are
/// untouched (separate kernels, separate maths, same version).
#[test]
fn wgsl_radial_blur_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for edge in [0u32, 1, 2] {
        for (centre, amount, spin, mix) in [
            ([0.5f32, 0.5f32], 6.0f32, true, 1.0f32),
            ([0.5, 0.5], 6.0, false, 1.0),
            ([0.3, 0.7], 9.5, true, 0.6),
            ([0.3, 0.7], 9.5, false, 0.6),
            ([0.5, 0.5], 0.0, true, 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::blur_radial(&mut cpu, w, h, centre, amount, spin, edge, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = RadialBlurOp {
                centre_frac: centre,
                amount_px: amount,
                taps: lumit_core::fx::cpu::radial_blur_taps(amount),
                spin,
                edge,
                mix,
            };
            let out = fx.radial_blur(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!(
                "radial blur e={edge} c={centre:?} a={amount} spin={spin}: worst {worst} ulp"
            );
            assert!(
                worst <= 2,
                "edge {edge} centre {centre:?} amount {amount} spin {spin} mix {mix}: \
                     worst {worst} fp16 ULP"
            );
            if amount == 0.0 && mix == 1.0 {
                assert_eq!(gpu, img, "amount 0 must be the bit-exact passthrough");
            }

            let out2 = fx.radial_blur(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU radial blur must be bit-stable");
        }
    }
}

/// The adjustment blend (docs/06 §1.5): out = mix(below, processed,
/// coverage·opacity) per channel, alpha included — pinned against a CPU
/// lerp on the corpus, with the end stops bit-exact: zero coverage
/// returns `below` untouched, full coverage at opacity 1 returns
/// `processed` untouched.
#[test]
fn adjust_blend_lerps_by_coverage_times_opacity() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (48u32, 32u32);
    let below = corpus(w, h);
    // A visibly different "effected" copy (any distinct image works).
    let processed: Vec<f32> = below
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i % 4 == 3 {
                *v
            } else {
                f16_to_f32(f16_bits(1.0 - v * 0.5))
            }
        })
        .collect();
    // Coverage ramps left to right in the alpha channel — the mask
    // raster's shape; colour channels are ignored by the kernel.
    let mut cov = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            cov[i + 3] = f16_to_f32(f16_bits(x as f32 / (w - 1) as f32));
        }
    }
    let tb = upload_linear_f32(&ctx, &below, w, h);
    let tp = upload_linear_f32(&ctx, &processed, w, h);
    let tc = upload_linear_f32(&ctx, &cov, w, h);
    for opacity in [1.0f32, 0.35] {
        let out = fx.adjust_blend(&ctx, &tb, &tp, &tc, w, h, opacity);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let want: Vec<f32> = below
            .iter()
            .zip(&processed)
            .enumerate()
            .map(|(i, (b, p))| {
                let c = (cov[(i / 4) * 4 + 3] * opacity).clamp(0.0, 1.0);
                f16_to_f32(f16_bits(b * (1.0 - c) + p * c))
            })
            .collect();
        let worst = worst_f16_ulp(&gpu, &want);
        eprintln!("adjust blend opacity={opacity}: worst {worst} ulp");
        assert!(worst <= 1, "opacity {opacity}: worst {worst} fp16 ULP");

        let out2 = fx.adjust_blend(&ctx, &tb, &tp, &tc, w, h, opacity);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU adjust blend must be bit-stable");
    }
    // End stops: no coverage passes `below` through bit-exactly; full
    // coverage at opacity 1 is `processed` bit-exactly.
    let clear = vec![0.0f32; (w * h * 4) as usize];
    let t0 = upload_linear_f32(&ctx, &clear, w, h);
    let out = fx.adjust_blend(&ctx, &tb, &tp, &t0, w, h, 1.0);
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        below,
        "zero coverage must be a bit-exact passthrough"
    );
    let full: Vec<f32> = clear
        .iter()
        .enumerate()
        .map(|(i, _)| if i % 4 == 3 { 1.0 } else { 0.0 })
        .collect();
    let t1 = upload_linear_f32(&ctx, &full, w, h);
    let out = fx.adjust_blend(&ctx, &tb, &tp, &t1, w, h, 1.0);
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        processed,
        "full coverage at opacity 1 must be the processed image bit-exactly"
    );
}

/// The §1.6 oracle for Echo (docs/08 §3.13; blend modes + 16-echo cap since
/// FX-17/K-149): the GPU chain (an `echo_accumulate` per tap plus a final
/// `echo_mix`) matches `lumit_core::fx::cpu::echo` across every combine mode.
/// Each accumulate stores an fp16 intermediate where the CPU keeps f32, so a
/// two-tap sum can drift a little past the pointwise ≤2 ULP — the historical
/// additive modes are held to 4 ULP with that reason (measured well under it).
/// The multiplicative/perceptual modes (Screen, Multiply, Overlay, Soft/Hard
/// light) additionally amplify the ≤½-ULP gap between the fp16-uploaded
/// current frame and the CPU's f32 corpus by their local slope against the
/// HDR neighbours, so they run single-tap under a looser 8-ULP bound — still
/// orders of magnitude tighter than any formula mismatch. The GPU is
/// bit-stable (§2.4); no taps with Mix 1 is a bit-exact passthrough.
#[test]
fn wgsl_echo_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let current = corpus(w, h);
    // Two distinct neighbour frames, at offsets -1 and -2.
    let neigh = |scale: f32| -> Vec<f32> {
        current
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if i % 4 == 3 {
                    *v
                } else {
                    f16_to_f32(f16_bits((v * scale).min(6.0)))
                }
            })
            .collect()
    };
    let n1 = neigh(0.8);
    let n2 = neigh(0.5);
    let cur_t = upload_linear_f32(&ctx, &current, w, h);
    let n1_t = upload_linear_f32(&ctx, &n1, w, h);
    let n2_t = upload_linear_f32(&ctx, &n2, w, h);
    let gpu_neighbours: [(i32, &wgpu::Texture); 2] = [(-1, &n1_t), (-2, &n2_t)];
    let cpu_neighbours: [(i32, &[f32]); 2] = [(-1, &n1), (-2, &n2)];

    let two_tap = |a: f32, b: f32| {
        let mut w = [0.0f32; 16];
        w[0] = a;
        w[1] = b;
        w
    };
    let one_tap = |a: f32| {
        let mut w = [0.0f32; 16];
        w[0] = a;
        w
    };

    // The compositing orders + Add (Behind/In front/Add), two-tap, ≤4 ULP (T21).
    for (weights, mode, mix, bound) in [
        (two_tap(0.6, 0.3), 0u32, 1.0f32, 4i32),
        (two_tap(0.7, 0.4), 1, 0.8, 4),
        (two_tap(0.9, 0.5), 2, 1.0, 4),
        // The blend modes (FX-17/K-149, T21), single-tap, ≤8 ULP: Screen,
        // Multiply, Overlay, Soft light, Hard light, Lighten, Darken,
        // Difference, Exclusion, Subtract. (Divide is checked separately below,
        // with a neighbour floored away from zero.)
        (one_tap(0.6), 3, 1.0, 8),
        (one_tap(0.7), 4, 0.9, 8),
        (one_tap(0.6), 5, 1.0, 8),
        (one_tap(0.5), 6, 1.0, 8),
        (one_tap(0.8), 7, 1.0, 8),
        (one_tap(0.6), 8, 1.0, 8),
        (one_tap(0.6), 9, 1.0, 8),
        (one_tap(0.7), 10, 1.0, 8),
        (one_tap(0.6), 11, 1.0, 8),
        (one_tap(0.5), 12, 1.0, 8),
    ] {
        let cpu = lumit_core::fx::cpu::echo(&current, &cpu_neighbours, weights, mode, mix);
        let op = EchoOp { weights, mode, mix };
        let out = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("echo mode={mode} mix={mix}: worst {worst} ulp");
        assert!(
            worst <= bound,
            "mode {mode} mix {mix}: worst {worst} fp16 ULP (bound {bound})"
        );
        let out2 = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, &op);
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "GPU echo must be bit-stable"
        );
    }
    // Divide (mode 13, T21): tested with a neighbour floored well away from
    // zero, so the a÷n has no near-singular denominators to blow past fp16.
    {
        let n_div: Vec<f32> = current
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if i % 4 == 3 {
                    *v
                } else {
                    f16_to_f32(f16_bits(v * 0.5 + 0.5))
                }
            })
            .collect();
        let n_div_t = upload_linear_f32(&ctx, &n_div, w, h);
        let gpu_neighbours: [(i32, &wgpu::Texture); 1] = [(-1, &n_div_t)];
        let cpu_neighbours: [(i32, &[f32]); 1] = [(-1, &n_div)];
        let op = EchoOp {
            weights: one_tap(0.9),
            mode: 13,
            mix: 1.0,
        };
        let cpu = lumit_core::fx::cpu::echo(&current, &cpu_neighbours, op.weights, op.mode, op.mix);
        let out = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("echo mode=13 (divide): worst {worst} ulp");
        assert!(worst <= 8, "divide: worst {worst} fp16 ULP");
    }
    // No taps, Mix 1: the accumulator is the current frame and the mix is
    // identity, so the output is the current frame bit-exactly.
    let out = fx.echo(
        &ctx,
        &cur_t,
        &gpu_neighbours,
        w,
        h,
        &EchoOp {
            weights: [0.0; 16],
            mode: 0,
            mix: 1.0,
        },
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        current,
        "no taps at Mix 1 must be a bit-exact passthrough"
    );
}

/// The §1.6 oracle for Flow motion blur (docs/08 §3.2): the GPU smear
/// matches `lumit_core::fx::cpu::motion_blur` given the same flow field,
/// on a constant-motion field and a varying one. Both accumulate the taps
/// in f32 and read the same fp16 source and the same exact (rg32float)
/// flow vectors, so — exactly like the Directional/Radial blur oracles it
/// shares its tap-integral shape with — it holds to the cheap-class ≤ 2
/// fp16 ULP bound despite the multi-tap sum (measured worst: 1 ULP). The
/// GPU is bit-stable (§2.4); a zero flow and a zero shutter are both
/// bit-exact passthroughs.
#[test]
fn wgsl_motion_blur_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let src = upload_linear_f32(&ctx, &img, w, h);
    let n = (w * h) as usize;

    // A constant horizontal motion, and a smoothly varying field (per-pixel
    // direction and magnitude) — the two shapes the kernel must handle.
    let constant: (Vec<f32>, Vec<f32>) = (vec![5.0; n], vec![0.0; n]);
    let mut vary_u = vec![0f32; n];
    let mut vary_v = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            vary_u[i] = (y as f32 - h as f32 / 2.0) * 0.25;
            vary_v[i] = (x as f32 - w as f32 / 2.0) * 0.2;
        }
    }
    let varying = (vary_u, vary_v);

    use lumit_core::fx::MbView;
    let full = vec![1.0f32; n];
    // A smoothly varying confidence (FX-19): proves the GPU scales the streak by
    // .z exactly as the CPU oracle does.
    let mut conf_vary = vec![0f32; n];
    for (i, c) in conf_vary.iter_mut().enumerate() {
        *c = ((i % 5) as f32) / 4.0; // 0, .25, .5, .75, 1 repeating
    }

    let cases = [
        (&constant, &full, 0.5f32, 16i32, 1.0f32, "constant"),
        (&varying, &full, 1.0, 12, 0.7, "varying"),
        (&constant, &full, 0.25, 8, 1.0, "short"),
        (&varying, &conf_vary, 1.0, 12, 1.0, "confidence-scaled"),
    ];
    for (field, conf, shutter_frac, samples, mix, name) in cases {
        let (u, v) = field;
        let mut cpu = img.clone();
        lumit_core::fx::cpu::motion_blur(
            &mut cpu,
            w,
            h,
            u,
            v,
            conf,
            shutter_frac,
            samples,
            mix,
            MbView::Rendered,
        );
        let flow_t = upload_flow_field(&ctx, u, v, conf, w, h);
        let op = MotionBlurOp {
            shutter_frac,
            samples,
            mix,
            view: MbView::Rendered.code(),
        };
        let out = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("motion blur {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        let out2 = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "GPU motion blur must be bit-stable"
        );
    }

    // The diagnostic views (FX-19) match the CPU oracle too, on the varying
    // field with the varying confidence.
    let (u, v) = &varying;
    let flow_t = upload_flow_field(&ctx, u, v, &conf_vary, w, h);
    for view in [MbView::MotionVectors, MbView::Confidence] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::motion_blur(&mut cpu, w, h, u, v, &conf_vary, 0.5, 16, 1.0, view);
        let op = MotionBlurOp {
            shutter_frac: 0.5,
            samples: 16,
            mix: 1.0,
            view: view.code(),
        };
        let out = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        assert!(worst <= 2, "view {view:?}: worst {worst} fp16 ULP");
    }

    // A zero flow, and a real motion with a closed shutter, are both
    // bit-exact passthroughs (every tap collapses onto the pixel itself).
    let zero = upload_flow_field(&ctx, &vec![0.0; n], &vec![0.0; n], &full, w, h);
    let out = fx.motion_blur(
        &ctx,
        &src,
        &zero,
        w,
        h,
        &MotionBlurOp {
            shutter_frac: 0.5,
            samples: 16,
            mix: 1.0,
            view: MbView::Rendered.code(),
        },
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "zero flow must be a bit-exact passthrough"
    );
    let moving = upload_flow_field(&ctx, &constant.0, &constant.1, &full, w, h);
    let out = fx.motion_blur(
        &ctx,
        &src,
        &moving,
        w,
        h,
        &MotionBlurOp {
            shutter_frac: 0.0,
            samples: 16,
            mix: 1.0,
            view: MbView::Rendered.code(),
        },
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "a closed shutter must be a bit-exact passthrough"
    );
}

/// The §1.6 oracle for Datamosh (docs/08 §3.12, K-104; its own effect
/// since K-107; reworked to a flow-driven melt by K-164/T19): the GPU
/// streamline melt matches `lumit_core::fx::cpu::datamosh` given the same
/// -1 neighbour and flow field, on a constant field and a varying one — the
/// same two shapes [`wgsl_motion_blur_matches_the_cpu_oracle`] exercises,
/// since both kernels read flow the same way. The walk is a multi-tap
/// bilinear sum like Motion blur's streak (plus a bilinear flow re-sample
/// each step), so it holds to the same ≤ 2 fp16 ULP bound Motion blur does.
/// The bloom and step counts vary across the cases; the GPU is bit-stable
/// (§2.4); Intensity 0 is a bit-exact passthrough.
#[test]
fn wgsl_datamosh_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let current = corpus(w, h);
    // A distinct -1 neighbour: the alpha channel carried through (as Echo's
    // oracle does), colour channels scaled and requantised to fp16.
    let prev: Vec<f32> = current
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i % 4 == 3 {
                *v
            } else {
                f16_to_f32(f16_bits((v * 0.6 + 0.05).min(6.0)))
            }
        })
        .collect();
    let cur_t = upload_linear_f32(&ctx, &current, w, h);
    let prev_t = upload_linear_f32(&ctx, &prev, w, h);
    let n = (w * h) as usize;

    let constant: (Vec<f32>, Vec<f32>) = (vec![-4.0; n], vec![2.0; n]);
    let mut vary_u = vec![0f32; n];
    let mut vary_v = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            vary_u[i] = (x as f32 - w as f32 / 2.0) * 0.3;
            vary_v[i] = (y as f32 - h as f32 / 2.0) * 0.25;
        }
    }
    let varying = (vary_u, vary_v);

    // Displacement sets the reach, steps the tap count, bloom the accumulation;
    // the > 1 intensity case exercises the open ceiling (K-135), which mix()
    // extrapolates past the moshed frame in both the CPU and GPU paths.
    for (field, intensity, displacement, bloom, steps, name) in [
        (
            &constant,
            1.0f32,
            1.0f32,
            0.6f32,
            1,
            "constant reach1 step1",
        ),
        (&varying, 0.6, 2.0, 1.0, 2, "varying reach2 bloom1"),
        (&constant, 0.35, 4.0, 0.0, 4, "reach4 bloom0"),
        (&varying, 1.4, 6.0, 0.5, 6, "over-unity reach6"),
        (&varying, 0.8, 12.0, 0.85, 12, "long melt reach12"),
    ] {
        let (u, v) = field;
        let cpu = lumit_core::fx::cpu::datamosh(
            &current,
            &prev,
            w,
            h,
            u,
            v,
            intensity,
            displacement,
            bloom,
            steps,
        );
        // Datamosh reads only the flow .xy; confidence is irrelevant (empty).
        let flow_t = upload_flow_field(&ctx, u, v, &[], w, h);
        let op = DatamoshOp {
            intensity,
            displacement,
            bloom,
            steps,
        };
        let out = fx.datamosh(&ctx, &cur_t, &prev_t, &flow_t, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("datamosh {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        let out2 = fx.datamosh(&ctx, &cur_t, &prev_t, &flow_t, w, h, &op);
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "GPU datamosh must be bit-stable"
        );
    }

    // Intensity 0 must be a bit-exact passthrough regardless of the melt.
    let moving = upload_flow_field(&ctx, &constant.0, &constant.1, &[], w, h);
    let out = fx.datamosh(
        &ctx,
        &cur_t,
        &prev_t,
        &moving,
        w,
        h,
        &DatamoshOp {
            intensity: 0.0,
            displacement: 8.0,
            bloom: 0.7,
            steps: 8,
        },
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        current,
        "intensity 0 must be a bit-exact passthrough"
    );
}

/// Build a `Lut3d` (domain 0..1) by mapping each grid point through `f`,
/// pushed **red-fastest** (index `r + g*size + b*size*size`) — the layout
/// `upload_lut_3d` and the shader assume.
fn build_lut(size: usize, f: impl Fn([f32; 3]) -> [f32; 3]) -> lumit_core::lut::Lut3d {
    build_lut_over(size, [0.0; 3], [1.0; 3], f)
}

/// The same, over an explicit `DOMAIN_MIN`/`DOMAIN_MAX` (K-271): the grid
/// points are the domain's own even spacing, so a cube built here says the
/// same thing as one exported by a grading tool that declares a domain.
fn build_lut_over(
    size: usize,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    f: impl Fn([f32; 3]) -> [f32; 3],
) -> lumit_core::lut::Lut3d {
    let maxf = (size - 1) as f32;
    let at = |i: usize, ch: usize| {
        domain_min[ch] + (domain_max[ch] - domain_min[ch]) * (i as f32 / maxf)
    };
    let mut data = Vec::with_capacity(size * size * size);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                data.push(f([at(r, 0), at(g, 1), at(b, 2)]));
            }
        }
    }
    lumit_core::lut::Lut3d {
        size,
        domain_min,
        domain_max,
        data,
    }
}

/// The §1.6 oracle for the 3D LUT (docs/08 §3.11; docs/impl/lut.md): the
/// WGSL manual-trilinear lookup matches `lumit_core::lut::Lut3d::sample`
/// wrapped as unpremultiply -> sample -> re-premultiply -> Mix, on a spread
/// of RGBA pixels **including partial-alpha and out-of-domain HDR ones** and
/// several cubes (identity, a per-channel gamma, an R/B swap). A cheap
/// pointwise effect, so CPU and GPU agree to ≤ 2 fp16 ULP; the GPU is
/// bit-stable (§2.4); Mix 0 is the bit-exact input; and the identity cube
/// round-trips every in-domain pixel to itself (a strong end-to-end check
/// that the red-fastest indexing, the domain scale and the premult handling
/// are all right — if it did not, one of those three is wrong).
#[test]
fn wgsl_lut_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);

    // A premultiplied corpus built from a known *straight* colour and an
    // alpha that cycles through 0, partial and 1, so unpremultiply -> look
    // up -> re-premultiply is exercised at every alpha. A couple of pixels
    // carry straight colour > 1.0 to hit the out-of-domain edge clamp.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let s = [
                x as f32 / (w - 1) as f32,
                y as f32 / (h - 1) as f32,
                (x + y) as f32 / (w + h) as f32,
            ];
            let a = match (x + y) % 4 {
                0 => 0.0,
                1 => 0.25,
                2 => 0.5,
                _ => 1.0,
            };
            img[i] = s[0] * a;
            img[i + 1] = s[1] * a;
            img[i + 2] = s[2] * a;
            img[i + 3] = a;
        }
    }
    // Out-of-domain straight colours (alpha 1): must clamp on both paths.
    img[((5 * w + 7) * 4) as usize..((5 * w + 7) * 4 + 4) as usize]
        .copy_from_slice(&[1.5, 0.2, 2.0, 1.0]);
    img[((9 * w + 3) * 4) as usize..((9 * w + 3) * 4 + 4) as usize]
        .copy_from_slice(&[3.0, 4.0, 0.1, 1.0]);
    // fp16-quantise exactly as the GPU sees it, so the comparison isolates
    // the LUT maths from upload rounding.
    let img: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();

    let unpremult = |c: [f32; 4]| -> [f32; 3] {
        if c[3] > 0.0 {
            [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
        } else {
            [0.0; 3]
        }
    };

    let identity = build_lut(3, |c| c);
    // A per-channel gamma (a real, non-linear "film" curve); trilinear is
    // approximate for it, but both paths use the *same* cube, so they still
    // agree — the point is the interpolation maths, not the cube's fidelity.
    let gamma = build_lut(5, |c| [c[0].powf(2.0), c[1].powf(0.5), c[2].powf(1.5)]);
    // A non-separable swap of red and blue: out = [b, g, r].
    let swap = build_lut(2, |c| [c[2], c[1], c[0]]);

    // A cube over a NON-DEFAULT domain (K-271): the shipped shader assumed
    // 0..1 and skipped the `(c - lo) / (hi - lo)` remap the CPU applies, so a
    // cube like this rendered silently wrong on the GPU while the oracle was
    // right. Asymmetric per channel, and one axis deliberately narrower than
    // 0..1 so mid-grey lands in a different cell on each path if the remap is
    // missing.
    let domained = build_lut_over(4, [-0.25, 0.0, 0.1], [1.5, 0.75, 1.0], |c| {
        [c[2], c[0], c[1]]
    });
    // The degenerate domain a malformed file can declare: DOMAIN_MIN equal to
    // DOMAIN_MAX. The CPU reads a zero span as 0 rather than dividing; the
    // shader must do the same and not produce NaN.
    let zero_span = build_lut_over(3, [0.5; 3], [0.5; 3], |c| [c[1], c[2], c[0]]);

    let cases: [(&str, &lumit_core::lut::Lut3d, f32); 8] = [
        ("identity-full", &identity, 1.0),
        ("identity-mix0", &identity, 0.0),
        ("gamma-full", &gamma, 1.0),
        ("gamma-mixed", &gamma, 0.5),
        ("swap-rb", &swap, 1.0),
        ("domained-full", &domained, 1.0),
        ("domained-mixed", &domained, 0.5),
        ("zero-span-domain", &zero_span, 1.0),
    ];

    for (name, lut, mix) in cases {
        // CPU expected: unpremultiply -> Lut3d::sample -> re-premultiply ->
        // Mix, using the same lerp form the shader uses for the final blend.
        let mut cpu = vec![0.0f32; img.len()];
        for px in 0..(w * h) as usize {
            let i = px * 4;
            let o = [img[i], img[i + 1], img[i + 2], img[i + 3]];
            let graded = lut.sample(unpremult(o));
            let pm = [graded[0] * o[3], graded[1] * o[3], graded[2] * o[3]];
            cpu[i] = o[0] + (pm[0] - o[0]) * mix;
            cpu[i + 1] = o[1] + (pm[1] - o[1]) * mix;
            cpu[i + 2] = o[2] + (pm[2] - o[2]) * mix;
            cpu[i + 3] = o[3];
        }

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let lut_tex = upload_lut_3d(&ctx, lut.size as u32, &lut.data);
        let out = fx.lut(
            &ctx,
            &tex,
            w,
            h,
            &lut_tex,
            lut.size as u32,
            mix,
            lut.domain_min,
            lut.domain_max,
        );
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("lut {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");

        if name == "identity-mix0" {
            // Mix 0 is the bit-exact input on the GPU path.
            assert_eq!(gpu, img, "{name}: Mix 0 must be the bit-exact input");
        }

        // Determinism: a second run is bit-identical to the first (§2.4).
        let out2 = fx.lut(
            &ctx,
            &tex,
            w,
            h,
            &lut_tex,
            lut.size as u32,
            mix,
            lut.domain_min,
            lut.domain_max,
        );
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "{name}: GPU LUT must be bit-stable");
    }

    // End-to-end: the identity cube at Mix 1.0 returns every *in-domain*
    // pixel to itself (out-of-domain HDR pixels legitimately clamp, so they
    // are excluded). A transposed cube or a broken premult round-trip would
    // fail this loudly.
    let lut_tex = upload_lut_3d(&ctx, identity.size as u32, &identity.data);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let gpu = readback_linear_f32(
        &ctx,
        &fx.lut(
            &ctx,
            &tex,
            w,
            h,
            &lut_tex,
            identity.size as u32,
            1.0,
            identity.domain_min,
            identity.domain_max,
        ),
        w,
        h,
    )
    .unwrap();
    for px in 0..(w * h) as usize {
        let i = px * 4;
        let o = [img[i], img[i + 1], img[i + 2], img[i + 3]];
        let s = unpremult(o);
        if s.iter().all(|v| (0.0..=1.0).contains(v)) {
            for c in 0..4 {
                assert!(
                    (gpu[i + c] - img[i + c]).abs() < 5e-3,
                    "identity must round-trip in-domain pixel {px} chan {c}: \
                         {} vs {}",
                    gpu[i + c],
                    img[i + c]
                );
            }
        }
    }
}
/// One resolved depth-of-field setting, as both paths need it.
///
/// The two sides are built from **one** value rather than from two parallel
/// argument lists: `lumit_core::fx::cpu::DofParams` is the oracle's input and
/// `lumit_gpu::fx::DofOp` the kernel's, field for field, so a field added to one
/// and forgotten on the other stops compiling instead of quietly diverging.
fn dof_op(p: &lumit_core::fx::cpu::DofParams) -> crate::fx::DofOp {
    crate::fx::DofOp {
        focus: p.focus,
        range: p.range,
        near_aperture: p.near_aperture,
        far_aperture: p.far_aperture,
        blade_normals: p.blade_normals,
        blade_count: p.blade_count,
        apothem2: p.apothem2,
        roundness: p.roundness,
        rim: p.rim,
        aspect_scale: p.aspect_scale,
        threshold: p.threshold,
        bokeh_power: p.bokeh_power,
        repeat_edge: p.repeat_edge,
        depth_bound: true,
        depth_channel: p.depth_channel,
        depth_invert: p.depth_invert,
        use_focus_point: p.use_focus_point,
        focus_point: p.focus_point,
        gamma: p.gamma,
        remove_edge_leak: p.remove_edge_leak,
        detect_edge_threshold: p.detect_edge_threshold,
        display: p.display,
        mix: p.mix,
    }
}

/// The shipped defaults, resolved: the plain circle, no weighting, no tonal
/// split — the aperture this effect gathered before it grew any of them.
fn dof_defaults() -> lumit_core::fx::cpu::DofParams {
    let (blade_normals, apothem2) = lumit_core::fx::aperture_blades(6, 0.0);
    lumit_core::fx::cpu::DofParams {
        focus: 0.5,
        range: 0.1,
        near_aperture: 6.0,
        far_aperture: 6.0,
        blade_normals,
        blade_count: 6,
        apothem2,
        roundness: 1.0,
        rim: 0.0,
        aspect_scale: [1.0, 1.0],
        threshold: 1.0,
        bokeh_power: 1.0,
        repeat_edge: true,
        depth_channel: 2, // Red: the oracle writes its ramp to red alone
        depth_invert: false,
        use_focus_point: false,
        focus_point: [0.0, 0.0],
        gamma: 1.0,
        remove_edge_leak: 0.0,
        detect_edge_threshold: 0.1,
        display: 0,
        mix: 1.0,
    }
}

/// The §1.6 oracle for the depth-of-field lens blur (docs/08 §3.22): the WGSL
/// gather matches `lumit_core::fx::cpu::dof` over a depth ramp and a sweep of
/// focus, aperture, aperture *shape*, tonal and Display settings.
///
/// The oracle is the shipping CPU reference itself, not a second copy of the
/// maths written for the test (K-019): one function, two callers, so a change
/// to the kernel that the reference does not follow shows up here rather than in
/// a render. A tap-summing gather like Motion blur, reading exact (r32float)
/// depth and the same fp16 source, so it holds to the cheap-class ≤ 2 fp16 ULP
/// bound; the GPU is bit-stable (§2.4); Mix 0, a zero aperture, and a depth that
/// sits everywhere inside the sharp band are all bit-exact passthroughs.
#[test]
fn wgsl_dof_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let src = upload_linear_f32(&ctx, &img, w, h);
    let n = (w * h) as usize;

    // A left-to-right depth ramp: 0 at the left edge, 1 at the right, so
    // the CoC sweeps its whole range across the frame. r32float, uploaded
    // exact — the depth is not fp16-quantised.
    let mut ramp = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            ramp[(y * w + x) as usize] = x as f32 / (w - 1) as f32;
        }
    }
    let depth_t = upload_depth_map(&ctx, &ramp, w, h);
    // The CPU reference reads a whole RGBA picture and picks a channel, because
    // which channel carries depth is one of the effect's controls; the GPU reads
    // the same numbers out of an R32Float map. Red in both.
    let mut depth_rgba = vec![0f32; n * 4];
    for (i, d) in ramp.iter().enumerate() {
        depth_rgba[i * 4] = *d;
        depth_rgba[i * 4 + 3] = 1.0;
    }

    // Every case is continuous in depth — the near/far select flips only where
    // s == 0, the aperture polygon and the tonal split are continuous in their
    // own controls, and the Depth/Focus maps are smooth — so the cheap-class
    // ≤ 2 fp16 ULP bound holds across all of them and none is excluded.
    let star = lumit_core::fx::aperture_blades(5, 30.0);
    let cases: Vec<(&str, lumit_core::fx::cpu::DofParams)> = vec![
        ("centre-focus", dof_defaults()),
        (
            "near-focus",
            lumit_core::fx::cpu::DofParams {
                focus: 0.0,
                range: 0.05,
                near_aperture: 8.0,
                far_aperture: 8.0,
                ..dof_defaults()
            },
        ),
        (
            "partial mix",
            lumit_core::fx::cpu::DofParams {
                mix: 0.5,
                ..dof_defaults()
            },
        ),
        (
            "wide aperture",
            lumit_core::fx::cpu::DofParams {
                range: 0.2,
                near_aperture: 10.0,
                far_aperture: 10.0,
                ..dof_defaults()
            },
        ),
        (
            "inverted near-focus",
            lumit_core::fx::cpu::DofParams {
                focus: 0.2,
                depth_invert: true,
                near_aperture: 8.0,
                far_aperture: 8.0,
                ..dof_defaults()
            },
        ),
        (
            "asymmetric near>far",
            lumit_core::fx::cpu::DofParams {
                range: 0.05,
                near_aperture: 12.0,
                far_aperture: 3.0,
                ..dof_defaults()
            },
        ),
        (
            "asymmetric far>near",
            lumit_core::fx::cpu::DofParams {
                range: 0.05,
                near_aperture: 3.0,
                far_aperture: 12.0,
                ..dof_defaults()
            },
        ),
        (
            "depth map",
            lumit_core::fx::cpu::DofParams {
                display: 1,
                ..dof_defaults()
            },
        ),
        (
            "depth map inverted",
            lumit_core::fx::cpu::DofParams {
                display: 1,
                depth_invert: true,
                ..dof_defaults()
            },
        ),
        (
            "focus map",
            lumit_core::fx::cpu::DofParams {
                display: 2,
                ..dof_defaults()
            },
        ),
        (
            "focus map asymmetric",
            lumit_core::fx::cpu::DofParams {
                focus: 0.3,
                range: 0.15,
                near_aperture: 12.0,
                far_aperture: 4.0,
                display: 2,
                ..dof_defaults()
            },
        ),
        // The aperture: a hexagon, a star (Roundness below zero), a squeezed
        // oval, and rim/centre weighting.
        (
            "hexagonal iris",
            lumit_core::fx::cpu::DofParams {
                roundness: 0.0,
                ..dof_defaults()
            },
        ),
        (
            "five-point star",
            lumit_core::fx::cpu::DofParams {
                roundness: -1.0,
                blade_count: 5,
                blade_normals: star.0,
                apothem2: star.1,
                ..dof_defaults()
            },
        ),
        (
            "anamorphic squeeze",
            lumit_core::fx::cpu::DofParams {
                roundness: 0.0,
                aspect_scale: [1.0, 2.0],
                ..dof_defaults()
            },
        ),
        (
            "rim-weighted",
            lumit_core::fx::cpu::DofParams {
                rim: 0.8,
                ..dof_defaults()
            },
        ),
        (
            "centre-weighted",
            lumit_core::fx::cpu::DofParams {
                rim: -0.8,
                ..dof_defaults()
            },
        ),
        // The highlights: the split-at-threshold power mean, at a threshold the
        // corpus actually crosses.
        (
            "bloomed highlights",
            lumit_core::fx::cpu::DofParams {
                threshold: 0.2,
                bokeh_power: 4.0,
                ..dof_defaults()
            },
        ),
        (
            "bloomed hexagons",
            lumit_core::fx::cpu::DofParams {
                threshold: 0.2,
                bokeh_power: 4.0,
                roundness: 0.0,
                ..dof_defaults()
            },
        ),
        // The depth model's own controls.
        (
            "focus point",
            lumit_core::fx::cpu::DofParams {
                use_focus_point: true,
                focus_point: [24.0, 12.0],
                ..dof_defaults()
            },
        ),
        (
            "profile squeezed",
            lumit_core::fx::cpu::DofParams {
                gamma: 4.0,
                ..dof_defaults()
            },
        ),
        (
            "edge leak removed",
            lumit_core::fx::cpu::DofParams {
                remove_edge_leak: 0.7,
                detect_edge_threshold: 0.05,
                ..dof_defaults()
            },
        ),
        (
            "green channel",
            lumit_core::fx::cpu::DofParams {
                depth_channel: 1,
                ..dof_defaults()
            },
        ),
        (
            "transparent edges",
            lumit_core::fx::cpu::DofParams {
                repeat_edge: false,
                ..dof_defaults()
            },
        ),
    ];
    for (name, p) in &cases {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::dof(&mut cpu, Some(&depth_rgba), w, h, p);
        let out = fx.dof(&ctx, &src, w, h, &depth_t, &dof_op(p));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("dof {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        // Determinism (§2.4): a second run is bit-identical to the first.
        let out2 = fx.dof(&ctx, &src, w, h, &depth_t, &dof_op(p));
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "{name}: GPU DoF must be bit-stable"
        );
    }

    // Mix 0 is the bit-exact input regardless of depth or aperture (Rendered
    // mode).
    let zero_mix = lumit_core::fx::cpu::DofParams {
        near_aperture: 10.0,
        far_aperture: 10.0,
        mix: 0.0,
        ..dof_defaults()
    };
    let out = fx.dof(&ctx, &src, w, h, &depth_t, &dof_op(&zero_mix));
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "Mix 0 must be the bit-exact input"
    );

    // Both apertures zero collapses every aperture to the centre tap — a
    // bit-exact passthrough at full Mix, whatever the depth (invert cannot
    // change a zero radius).
    let zero_ap = lumit_core::fx::cpu::DofParams {
        near_aperture: 0.0,
        far_aperture: 0.0,
        depth_invert: true,
        ..dof_defaults()
    };
    let out = fx.dof(&ctx, &src, w, h, &depth_t, &dof_op(&zero_ap));
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "a zero aperture must be a bit-exact passthrough"
    );

    // A depth that sits everywhere inside the sharp band leaves the CoC at
    // zero for every pixel — also a bit-exact passthrough at full Mix,
    // even with large apertures. Inverting a flat 0.5 leaves it in-band.
    let flat = upload_depth_map(&ctx, &vec![0.5f32; n], w, h);
    let in_band = lumit_core::fx::cpu::DofParams {
        near_aperture: 10.0,
        far_aperture: 10.0,
        ..dof_defaults()
    };
    let out = fx.dof(&ctx, &src, w, h, &flat, &dof_op(&in_band));
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "an in-band depth must be a bit-exact passthrough"
    );
}

/// The kernel's blade ceiling and the document model's are the same number.
/// `lumit-core` is only a dev-dependency of this crate, so the constant is
/// declared twice; this is what stops the two drifting.
#[test]
fn max_blades_matches_the_core_constant() {
    assert_eq!(crate::fx::MAX_BLADES, lumit_core::fx::MAX_BLADES);
}

// ---------------------------------------------------------------------------
// Lens flare (docs/08 §3.27, docs/impl/lens-flare.md §8, K-256): the staged
// oracle's GPU half.
// ---------------------------------------------------------------------------

/// The documented drop-on defaults (docs/08 §3.27) as a resolved bundle.
fn flare_params() -> lumit_core::fx::lens_flare::LensFlareParams {
    lumit_core::fx::lens_flare::LensFlareParams {
        // Raster pixels of a 192×108 probe framing (K-260).
        light: [63.4, 32.4],
        // A point source, as the effect has always defaulted to, and no
        // comp lights — Manual mode never reads them.
        source_size: [0.0, 0.0],
        lights: [lumit_core::fx::lens_flare::DEAD_LIGHT; lumit_core::fx::lens_flare::MAX_SOURCES],
        light_count: 0,
        intensity: 1.0,
        lens: 16,
        fstop: 2.8,
        focus_m: 100.0,
        blades: 8,
        aperture_rotation_deg: 0.0,
        roundness: 0.15,
        aperture_softness: 0.05,
        ghost_intensity: 1.0,
        ghost_softness: 0.05,
        max_ghosts: 10,
        dispersion: 1.0,
        coating: 0.75,
        starburst_intensity: 1.0,
        scale: 1.0,
        source: 0,
        threshold: 1.0,
        threshold_softness: 0.25,
        light_tint: [1.0, 1.0, 1.0],
        use_source_colour: true,
        anamorphic: 1.0,
        quality: 0,
        detail: 1.0,
        blend: lumit_core::fx::lens_flare::BLEND_ADD,
        mix: 1.0,
    }
}

/// The fxops params→op conversion, mirrored for the tests (the production
/// copy lives in lumit-render's dispatch arm; both derive every number from
/// the same lumit-core functions, which is the point).
fn flare_op(p: &lumit_core::fx::lens_flare::LensFlareParams, w: u32, h: u32) -> LensFlareOp {
    use lumit_core::fx::lens_flare as lf;
    let (tier_base, tier_lambda, flare_div) = lf::quality_ladder(p.quality);
    let grid = lf::detail_base(tier_base, p.detail);
    let lambda_count = lf::detail_lambda(tier_lambda, p.detail);
    let energy = p.ghost_intensity;
    let bands = lf::spectral_bands(lambda_count, p.dispersion)
        .into_iter()
        .map(|b| crate::fx::lens_flare::FlareBand {
            traced_nm: b.traced_nm,
            sub_idx: b.sub_idx,
            sub_rgb: b
                .sub_rgb
                .map(|c| [c[0] * energy, c[1] * energy, c[2] * energy]),
        })
        .collect();
    LensFlareOp {
        light_frac: [p.light[0] / w.max(1) as f32, p.light[1] / h as f32],
        // The same expansion `fxops` makes for the production path, so the
        // oracle drives the GPU exactly as the renderer does (K-355).
        manual_lights: lf::expand_area_lights(&lf::manual_light(p, w, h), lf::AREA_SAMPLES_MAX)
            .iter()
            .map(|l| [l.pos[0], l.pos[1], l.rgb[0], l.rgb[1], l.rgb[2]])
            .collect(),
        intensity: p.intensity,
        bands,
        max_ghosts: p.max_ghosts,
        coating: p.coating,
        focus_m: p.focus_m,
        fstop: p.fstop,
        blades: p.blades,
        aperture_rotation_deg: p.aperture_rotation_deg,
        roundness: p.roundness,
        aperture_softness: p.aperture_softness,
        ghost_softness: p.ghost_softness,
        grid,
        flare_div,
        screen_transform: lf::screen_transform(w),
        starburst_intensity: p.starburst_intensity,
        scale: p.scale,
        anamorphic: p.anamorphic,
        source: p.source,
        threshold: p.threshold,
        threshold_softness: p.threshold_softness,
        light_tint: p.light_tint,
        use_source_colour: p.use_source_colour,
        blend: p.blend,
        mix: p.mix,
        bake_key: lf::bake_key(p),
    }
}

/// The fxops frame-probe closure (K-267), mirrored for the tests: the GPU
/// hands back its cached bake's tables and this runs the one lumit-core
/// probe both twins share, at the op's light direction.
fn flare_probe(
    p: &lumit_core::fx::lens_flare::LensFlareParams,
    w: u32,
    h: u32,
) -> impl Fn(&crate::fx::lens_flare::FlareProbeBake) -> Vec<u32> {
    use lumit_core::fx::lens_flare as lf;
    let (tier_base, _, _) = lf::quality_ladder(p.quality);
    let grid = lf::detail_base(tier_base, p.detail);
    let light_frac = [p.light[0] / w.max(1) as f32, p.light[1] / h.max(1) as f32];
    let aspect = h as f32 / w.max(1) as f32;
    let (coating, fstop, focus_m) = (p.coating, p.fstop, p.focus_m);
    move |pb: &crate::fx::lens_flare::FlareProbeBake| {
        let needs = lf::frame_grid_needs_from_rows(
            pb.surfaces,
            pb.ghosts,
            pb.sensor_z_mm,
            pb.focal_mm,
            pb.pupil_mm,
            pb.start_z_mm,
            pb.pair_count,
            lf::light_direction(light_frac, aspect, pb.focal_mm),
            coating,
            lf::fstop_scale(pb.native_fstop, fstop),
            lf::focus_shift_mm(focus_m, pb.focal_mm),
        );
        lf::plan_frame_grids(grid, pb.spreads, &needs)
    }
}

fn flare_bake_data(p: &lumit_core::fx::lens_flare::LensFlareParams) -> FlareBakeData {
    use lumit_core::fx::lens_flare as lf;
    let b = lf::bake(p);
    FlareBakeData {
        surfaces: b
            .surfaces
            .iter()
            .map(|s| {
                [
                    s.radius_mm,
                    s.z_mm,
                    s.semi_ap_mm,
                    s.cauchy_a,
                    s.cauchy_b,
                    s.coating_layers,
                    s.is_stop,
                    0.0,
                ]
            })
            .collect(),
        ghosts: b.pairs.clone(),
        spreads: b.spreads.clone(),
        sensor_z_mm: b.sensor_z_mm,
        focal_mm: b.focal_mm,
        native_fstop: b.native_fstop,
        pupil_mm: b.pupil_mm,
        start_z_mm: b.start_z_mm,
        energy_gain: b.energy_gain,
        reflectance: b.reflectance.clone(),
        starburst: b.starburst,
        sb_res: lf::STARBURST_RES,
    }
}

/// The reflectance table's grid is spelled out twice — once in lumit-core,
/// which bakes it, once in the WGSL that reads it (K-364) — because the
/// shader cannot import a Rust constant. A drift would silently index the
/// wrong wavelength for every ray, so the shader source is checked against
/// the constants it mirrors.
#[test]
fn lens_flare_wgsl_spectral_constants_match_lumit_core() {
    use lumit_core::fx::lens_flare as lf;
    let src = include_str!("../fx_lens_flare_trace.wgsl");
    for (name, want) in [
        ("REFL_LAMBDA_BINS", lf::REFL_LAMBDA_BINS),
        ("REFL_COS_BINS", lf::REFL_COS_BINS),
        ("SPECTRAL_SUB", lf::SPECTRAL_SUB),
    ] {
        let want = format!("const {name}: u32 = {want}u;");
        assert!(
            src.contains(&want),
            "the trace shader must declare `{want}`"
        );
    }
}

// The adaptive grid formula is mirrored in this crate (lumit-gpu stays
// lumit-core-free in production), so the two copies are pinned together —
// a drift would make the GPU trace different rays from the oracle (K-262).
#[test]
fn lens_flare_pair_grid_mirrors_lumit_core() {
    for base in [8u32, 24, 48, 64, 96, 144, 320] {
        for spread in [
            0.0f32, 0.05, 0.119, 0.12, 0.3, 0.49, 0.5, 1.0, 1.49, 1.5, 4.0, 99.0,
        ] {
            assert_eq!(
                crate::fx::lens_flare::pair_grid_of(base, spread),
                lumit_core::fx::lens_flare::pair_grid(base, spread),
                "base {base} spread {spread}"
            );
        }
    }
    // The K-267 padded-buffer dims are mirrored the same way.
    for (fw, fh) in [(960u32, 540u32), (480, 270), (1, 1), (1920, 1080)] {
        for (sq, sc) in [
            (1.0f32, 1.0f32),
            (0.5, 1.0),
            (0.25, 1.0),
            (1.0, 0.5),
            (0.7, 0.6),
            (2.0, 3.0),
            (0.0, 0.0),
        ] {
            assert_eq!(
                crate::fx::lens_flare::flare_pad_dims_of(fw, fh, sq, sc),
                lumit_core::fx::lens_flare::flare_pad_dims(fw, fh, sq, sc),
                "fw {fw} fh {fh} squeeze {sq} scale {sc}"
            );
        }
    }
}

/// The frame's dispatch plan covers every combo exactly once, for every
/// light, and no batch's scratch passes the budget (K-263).
///
/// The budget is the whole point: a batch that overran it is the
/// hundred-megabyte allocation a frame used to make at Ultra across eight
/// matte sources, and "allocate what the settings ask for" is how a flare
/// ends up taking the graphics device down with it.
#[test]
fn lens_flare_batches_cover_every_combo_within_the_scratch_budget() {
    use crate::fx::lens_flare::{
        plan_batches, AREA_BYTES, CELL_BYTES, RAY_BYTES, SCRATCH_BYTE_BUDGET,
    };
    // Grid-major tables as the frame builds them, including the worst case
    // (every combo at the widest grid) and a mixed one.
    let tables: Vec<Vec<u32>> = vec![
        vec![32; 480],
        vec![64; 8],
        {
            let mut t = vec![32u32; 300];
            t.extend(std::iter::repeat_n(64u32, 120));
            t.extend(std::iter::repeat_n(160u32, 60));
            t
        },
        vec![256; 40],
        vec![8; 1],
    ];
    for lights in [1u32, 8] {
        for table in &tables {
            let plan = plan_batches(table, lights);
            // Every (combo, light) appears exactly once.
            let mut seen = vec![0u32; table.len() * lights as usize];
            for b in &plan {
                assert_eq!(
                    b.grid, table[b.combo_offset as usize],
                    "a batch must dispatch at its combos' own grid"
                );
                let rays = u64::from(b.grid) * u64::from(b.grid);
                let quads = u64::from(b.grid - 1) * u64::from(b.grid - 1);
                let slots = u64::from(b.lights) * u64::from(b.combos);
                assert_eq!(b.ray_bytes, slots * rays * RAY_BYTES);
                assert_eq!(b.area_bytes, slots * quads * AREA_BYTES);
                assert_eq!(b.vert_bytes, slots * quads * CELL_BYTES);
                assert!(
                    b.ray_bytes + b.area_bytes + b.vert_bytes <= SCRATCH_BYTE_BUDGET,
                    "batch at grid {} × {} combos × {} lights wants {} bytes",
                    b.grid,
                    b.combos,
                    b.lights,
                    b.ray_bytes + b.area_bytes + b.vert_bytes
                );
                for c in b.combo_offset..b.combo_offset + b.combos {
                    assert_eq!(
                        table[c as usize], b.grid,
                        "a batch may not straddle two grids"
                    );
                    for l in b.light_offset..b.light_offset + b.lights {
                        seen[c as usize * lights as usize + l as usize] += 1;
                    }
                }
            }
            assert!(
                seen.iter().all(|&n| n == 1),
                "every combo × light renders exactly once (lights {lights}, grids {:?}…)",
                &table[..table.len().min(3)]
            );
        }
    }
}

/// Render one flare frame through the REAL GPU pipeline and write it as a
/// tone-mapped PPM for eyeballing — the harness behind the K-264 artefact
/// work, kept because "does it look right" is the one question no numeric
/// bound in this file answers. `#[ignore]`d; run by hand:
///
/// ```text
/// LUMIT_FLARE_DUMP=/tmp/flare.ppm cargo test -p lumit-gpu --release --lib \
///     lens_flare_dump_frame -- --ignored --nocapture
/// ```
///
/// Optional env overrides: LUMIT_FLARE_QUALITY (0-3), LUMIT_FLARE_LENS
/// (library index), LUMIT_FLARE_LIGHT ("x,y" raster fractions).
#[test]
#[ignore = "a diagnostic image dump, not a gate"]
fn lens_flare_dump_frame() {
    let Some(path) = std::env::var_os("LUMIT_FLARE_DUMP") else {
        eprintln!("LUMIT_FLARE_DUMP unset; skipping");
        return;
    };
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (1152u32, 648u32);
    let quality: u32 = std::env::var("LUMIT_FLARE_QUALITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let lens: u32 = std::env::var("LUMIT_FLARE_LENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let light = std::env::var("LUMIT_FLARE_LIGHT")
        .ok()
        .and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some([x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?])
        })
        .unwrap_or([0.42, 0.28]);
    let fstop: f32 = std::env::var("LUMIT_FLARE_FSTOP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.8);
    let detail: f32 = std::env::var("LUMIT_FLARE_DETAIL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    let anamorphic: f32 = std::env::var("LUMIT_FLARE_ANAMORPHIC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    let p = lf::LensFlareParams {
        light: [light[0] * w as f32, light[1] * h as f32],
        lens,
        quality,
        fstop,
        detail,
        anamorphic,
        max_ghosts: 60,
        ghost_softness: 0.0, // bare geometry — nothing hides behind blur
        ..flare_params()
    };
    let img = vec![0.0f32; (w * h * 4) as usize]; // black scene: flare alone
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let op = flare_op(&p, w, h);
    let out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    // Tone map for eyeballing: fixed gain (LUMIT_FLARE_GAIN, default 8),
    // then sRGB-ish gamma. Fixed rather than auto so two dumps of different
    // code are comparable.
    let gain: f32 = std::env::var("LUMIT_FLARE_GAIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8.0);
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in gpu.chunks_exact(4) {
        for c in &px[..3] {
            let v = (c * gain).clamp(0.0, 1.0).powf(1.0 / 2.2);
            ppm.push((v * 255.0).round() as u8);
        }
    }
    std::fs::write(&path, ppm).unwrap();
    eprintln!("wrote {}", std::path::Path::new(&path).display());
}

/// What one default-settings flare frame costs, printed. Not a gate — the
/// number is whatever the machine running it can do, and CI runs on
/// everything from a software rasteriser to a workstation card — so it is
/// `#[ignore]`d and run by hand:
///
/// ```text
/// cargo test -p lumit-gpu --release lens_flare_frame_cost -- --ignored --nocapture
/// ```
///
/// It exists because "the flare is faster now" is the sort of claim that
/// rots quietly. Run it before and after a change to the pipeline.
#[test]
#[ignore = "a measurement, not a gate: prints a time, asserts nothing"]
fn lens_flare_frame_cost() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (960u32, 540u32);
    let img = corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    // The shipped defaults (docs/08 §3.27): Normal quality, 60 ghosts.
    let p = lf::LensFlareParams {
        light: [317.0, 162.0],
        max_ghosts: 60,
        quality: 1,
        ..flare_params()
    };
    let op = flare_op(&p, w, h);
    let data = std::sync::Arc::new(flare_bake_data(&p));
    let bake = {
        let data = std::sync::Arc::clone(&data);
        std::sync::Arc::new(move || (*data).clone()) as crate::fx::FlareBake
    };
    // Warm: shader compilation and the bake upload are one-off costs.
    let warm = fx.lens_flare(&ctx, &tex, w, h, &op, None, &bake, &flare_probe(&p, w, h));
    drop(readback_linear_f32(&ctx, &warm, w, h));
    let runs = 3;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        let out = fx.lens_flare(&ctx, &tex, w, h, &op, None, &bake, &flare_probe(&p, w, h));
        // Read back so the timing includes the card finishing the work.
        drop(readback_linear_f32(&ctx, &out, w, h));
    }
    let each = started.elapsed() / runs;
    eprintln!("lens flare {w}×{h} Normal/60 ghosts: {each:?} per frame");
}

/// The Ghost blur agrees with the CPU reference at a radius wide enough to
/// span several of the line cache's tiles (K-263).
///
/// The frame oracle cannot cover this: its small test frame puts the default
/// Ghost softness at a radius of zero, so the blur is skipped entirely. This
/// one uses a frame large enough for a radius in the tens of pixels, which is
/// where the cache's halo — the 2r texels a tile needs beyond its own 64 —
/// either lines up with the direct loop's clamped reads or does not.
#[test]
fn wgsl_lens_flare_ghost_blur_matches_the_cpu_reference() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    // Big enough that the flare buffer is many tiles wide and the blur radius
    // is a real one; few enough ghosts that it stays a quick test.
    let (w, h) = (768u32, 432u32);
    let p = lf::LensFlareParams {
        light: [380.0, 130.0],
        ghost_softness: 2.0,
        max_ghosts: 3,
        ..flare_params()
    };
    let (_, _, div) = lf::quality_ladder(p.quality);
    let (fw, fh) = ((w / div).max(1), (h / div).max(1));
    let radius = lf::ghost_blur_radius(p.ghost_softness, fw, fh);
    assert!(
        radius >= 8,
        "the test frame must produce a multi-tile blur radius, got {radius}"
    );

    let img = corpus(w, h);
    let baked = lf::bake(&p);
    let op = flare_op(&p, w, h);
    let lights = lf::manual_light(&p, w, h);
    let flare = lf::cpu_flare(&p, &baked, fw, fh, &lights);
    let mut cpu = img.clone();
    lf::cpu_combine(&mut cpu, w, h, &p, &baked, &flare, fw, fh, &lights);

    let tex = upload_linear_f32(&ctx, &img, w, h);
    let out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

    let added: f32 = gpu
        .iter()
        .zip(&img)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / (w * h) as f32;
    assert!(added > 1e-4, "the blurred flare adds no energy: {added}");
    let mean: f32 = cpu
        .iter()
        .zip(&gpu)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / cpu.len() as f32;
    let worst = cpu
        .iter()
        .zip(&gpu)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    // Tighter than the frame oracle's 2e-3 ON PURPOSE. A blur that reads the
    // wrong texels — a tile that forgets its halo, an off-by-r cache index —
    // shifts a soft, dim wash a little sideways, and the frame bound does not
    // notice: measured, dropping the halo entirely left the mean at 8.5e-4,
    // comfortably inside 2e-3. These bounds sit about three times above what
    // the correct kernel produces and three times below what that break did,
    // so they hold across cards and still fail the break.
    assert!(
        mean < 2.5e-4,
        "mean |Δ| {mean} at blur radius {radius} (added {added})"
    );
    assert!(worst < 3e-3, "worst |Δ| {worst} at blur radius {radius}");
}

/// A heavy frame is handed to the card in several submissions, and a light
/// one in exactly one (K-263).
///
/// This is the guard against the failure the owner hit: a submission long
/// enough for the operating system's watchdog to kill does not cost a frame,
/// it costs the device — after which the Viewer is frozen for the rest of the
/// session and re-opening the project does not help.
#[test]
fn lens_flare_splits_a_heavy_frame_into_several_submissions() {
    use crate::fx::lens_flare::{plan_batches, plan_flushes, STEPS_PER_SUBMIT};
    let surfaces = 20u32;
    // A working-tier frame: Normal's base grid across a default ghost count.
    let heavy = plan_batches(&vec![64u32; 480], 1);
    let flushes = plan_flushes(&heavy, surfaces);
    assert!(
        flushes.iter().filter(|f| **f).count() >= 2,
        "a default Normal frame must not be one giant submission"
    );
    // No submission holds more work than the budget plus the one batch that
    // crossed it — the bound the watchdog guard rests on.
    let biggest_batch = heavy
        .iter()
        .map(|b| b.steps(surfaces))
        .max()
        .unwrap_or_default();
    let mut run = 0u64;
    for (b, flush) in heavy.iter().zip(&flushes) {
        run += b.steps(surfaces);
        assert!(
            run <= STEPS_PER_SUBMIT + biggest_batch,
            "a submission grew to {run} steps"
        );
        if *flush {
            run = 0;
        }
    }
    // A small frame stays a single submission: the split must not cost
    // ordinary work extra queue round trips.
    let light = plan_batches(&[32u32; 24], 1);
    assert!(
        plan_flushes(&light, surfaces).iter().all(|f| !f),
        "a light frame should submit once"
    );
}

/// A deferred bake really is made beside the frame: the frame that asked for
/// a lens it does not hold draws the lens before it, the bake lands on the
/// bake thread, and the frame after it draws the new one (K-350).
///
/// The picture is not what this checks — a card is needed for that, and the
/// oracle tests above do it. What it checks is the part that is pure
/// bookkeeping and would otherwise only ever be exercised by hand: which bake
/// a frame is given, and when.
#[test]
fn lens_flare_deferred_bakes_answer_with_the_previous_lens_then_the_new_one() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (64u32, 36u32);

    let first = lf::LensFlareParams {
        lens: 3,
        ..flare_params()
    };
    let second = lf::LensFlareParams {
        lens: 7,
        ..flare_params()
    };
    let tex = crate::fx::work_texture(&ctx, w, h, "flare-defer-src");

    // Exact to begin with, so there is a lens on screen to fall back to —
    // and so the first frame of a session is never a flare-less one by
    // accident.
    fx.set_deferred_flare_bakes(false);
    let op_a = flare_op(&first, w, h);
    let bake_a = std::sync::Arc::new(move || flare_bake_data(&first)) as crate::fx::FlareBake;
    drop(fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op_a,
        None,
        &bake_a,
        &flare_probe(&first, w, h),
    ));
    assert!(
        !fx.flare_bake_pending(),
        "an exact bake is finished by the time the frame is"
    );
    let after_exact = fx.flare_bake_generation();

    // Now defer, and ask for a lens nothing holds.
    fx.set_deferred_flare_bakes(true);
    let op_b = flare_op(&second, w, h);
    let bake_b = std::sync::Arc::new(move || flare_bake_data(&second)) as crate::fx::FlareBake;
    drop(fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op_b,
        None,
        &bake_b,
        &flare_probe(&second, w, h),
    ));
    assert_ne!(
        fx.flare_bake_generation(),
        after_exact,
        "asking for a lens that is not held queues its bake, and says so"
    );

    // The bake thread finishes and the next frame picks it up. Bounded, so a
    // machine that will not give us a thread fails the wait rather than the
    // suite hanging.
    // Every pass waits for the queue before asking for another frame. A frame
    // is what collects a landed bake, so the loop has to render — but on a
    // software rasteriser (CI's WARP, Mesa's lavapipe) a frame takes long
    // enough that firing them off every 10 ms would leave submissions piling
    // up faster than they retire, and nothing in flight is ever reclaimed.
    // Waiting makes each pass one frame, which is all this is counting.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        drop(fx.lens_flare(
            &ctx,
            &tex,
            w,
            h,
            &op_b,
            None,
            &bake_b,
            &flare_probe(&second, w, h),
        ));
        ctx.device.poll(wgpu::Maintain::Wait);
        if !fx.flare_bake_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        !fx.flare_bake_pending(),
        "the bake lands and the frame after it is no longer waiting"
    );

    // And once it is held, the frame is nameable again: nothing more is
    // queued and the generation sits still.
    let settled = fx.flare_bake_generation();
    drop(fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op_b,
        None,
        &bake_b,
        &flare_probe(&second, w, h),
    ));
    assert_eq!(
        fx.flare_bake_generation(),
        settled,
        "a held lens neither queues nor lands anything"
    );
}

/// Deferring changes when a bake is made, never what it is: the same key
/// gives the same bake either way (docs/impl/lens-flare.md §5 — the bake is
/// pure, and K-350 must not make it less so).
#[test]
fn lens_flare_a_deferred_bake_is_the_same_bake() {
    use lumit_core::fx::lens_flare as lf;
    let p = lf::LensFlareParams {
        lens: 11,
        ..flare_params()
    };
    let inline = flare_bake_data(&p);
    let bake = std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake;
    // Run it where the bake thread would: another thread entirely.
    let elsewhere = std::thread::spawn(move || bake())
        .join()
        .expect("the bake thread finishes");
    assert_eq!(inline.surfaces, elsewhere.surfaces);
    assert_eq!(inline.ghosts, elsewhere.ghosts);
    assert_eq!(inline.spreads, elsewhere.spreads);
    assert_eq!(inline.starburst, elsewhere.starburst);
    assert_eq!(
        inline.energy_gain.to_bits(),
        elsewhere.energy_gain.to_bits(),
        "the auto-exposure gain is bit-equal, not merely close"
    );
}

/// The bake cache keeps its most recent entries and drops the oldest — it
/// does not empty itself (K-263). Emptying is what made trying lenses
/// quadratic: every overflow threw away a full cap's worth of bakes, and a
/// bake is the effect's one slow, blocking step.
#[test]
fn lens_flare_bake_cache_evicts_the_oldest_not_everything() {
    let mut cache = crate::fx::lens_flare::BakeCache::new(4);
    for k in 0..4u64 {
        cache.insert(k, k * 10);
    }
    assert_eq!(cache.len(), 4);
    // One past the cap: only the oldest goes.
    cache.insert(4, 40);
    assert_eq!(cache.len(), 4);
    assert_eq!(cache.get(0), None, "the oldest is the one evicted");
    for k in 1..=4u64 {
        assert_eq!(cache.get(k), Some(k * 10), "{k} was still recent");
    }
    // Re-inserting a held key keeps the held value and does not evict.
    assert_eq!(cache.insert(4, 999), 40);
    assert_eq!(cache.len(), 4);
    assert_eq!(cache.get(1), Some(10));
}

/// Impl note §8.5: the WGSL trace agrees with the CPU trace corner-for-
/// corner (K-261 splat model): landing positions at a mean/percentile pixel
/// bound, weights at a relative bound, with ≥ 99% live/dead agreement,
/// across two lenses and two light positions. Not ULP-exact — GPU
/// transcendentals are not correctly rounded (the note's stated reason).
#[test]
fn wgsl_lens_flare_trace_matches_the_cpu_reference() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (192u32, 108u32);
    for lens in [16u32, 5] {
        for light_frac in [[0.33f32, 0.30f32], [0.85, 0.75]] {
            let p = lf::LensFlareParams {
                lens,
                light: [light_frac[0] * w as f32, light_frac[1] * h as f32],
                ..flare_params()
            };
            let baked = lf::bake(&p);
            let op = flare_op(&p, w, h);
            let dir = lf::light_direction(light_frac, h as f32 / w as f32, baked.focal_mm);
            let combo_limit = 12u32;
            let gpu = fx.lens_flare_trace_debug(
                &ctx,
                &op,
                &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
                combo_limit,
                w,
                h,
            );
            assert!(!gpu.is_empty(), "trace debug returned nothing");

            // Rebuild the same combo order the GPU used: pair-major over
            // the ranked list, band-minor.
            let (grid, lambda_count, _) = lf::quality_ladder(p.quality);
            let bands = lf::spectral_bands(lambda_count, p.dispersion);
            let mut combos = Vec::new();
            'outer: for &pair in baked.pairs.iter().take(p.max_ghosts as usize) {
                for band in &bands {
                    if combos.len() >= combo_limit as usize {
                        break 'outer;
                    }
                    combos.push((pair, band));
                }
            }
            let ray_count = (grid * grid) as usize;
            assert_eq!(gpu.len(), combos.len() * ray_count);

            // The frame-time optics the CPU side mirrors.
            let stop_scale = lf::fstop_scale(baked.native_fstop, p.fstop);
            let roundness = lf::effective_roundness(p.roundness, p.fstop, baked.native_fstop);
            let rot = p.aperture_rotation_deg.to_radians();
            let st = lf::screen_transform(w);
            let shift = lf::focus_shift_mm(p.focus_m, baked.focal_mm);

            let mut mismatched_liveness = 0u32;
            let mut total = 0u32;
            let mut live = 0u32;
            let mut sum_pos = 0.0f32;
            let mut pos_errs: Vec<f32> = Vec::new();
            let mut weight_errs: Vec<f32> = Vec::new();
            let mut rgb_errs: Vec<f32> = Vec::new();
            let mut worst_pos = 0.0f32;
            let mut worst_weight = 0.0f32;
            let mut worst_rgb = 0.0f32;
            for (ci, &(pair, band)) in combos.iter().enumerate() {
                for ry in 0..grid {
                    for rx in 0..grid {
                        let g = gpu[ci * ray_count + (ry * grid + rx) as usize];
                        let g1 = (grid.max(2) - 1) as f32;
                        let u = (rx as f32 / g1) * 2.0 - 1.0;
                        let v = (ry as f32 / g1) * 2.0 - 1.0;
                        // A masked-out corner traces with weight 0 since
                        // K-264 (geometry survives the iris; see cpu_flare)
                        // — except far outside the iris, where no cell can
                        // hold light and both twins skip the trace.
                        let g1f = (grid.max(2) - 1) as f32;
                        let lim = 1.0 + 1.5 * (2.0 / g1f);
                        let mask =
                            lf::pupil_mask(u, v, p.blades, rot, roundness, p.aperture_softness);
                        let cpu = if u * u + v * v > lim * lim {
                            None
                        } else {
                            let origin = [
                                u * baked.pupil_mm * stop_scale,
                                v * baked.pupil_mm * stop_scale,
                                baked.start_z_mm,
                            ];
                            lf::trace_splat_spectral(
                                &baked, pair, band, origin, dir, p.coating, stop_scale, shift,
                            )
                            .map(|(pos, wt, rgb)| {
                                // The op's bands carry Ghost intensity, the
                                // reference's do not (K-364).
                                let e = p.ghost_intensity;
                                (
                                    [pos[0] * st + w as f32 / 2.0, h as f32 / 2.0 - pos[1] * st],
                                    wt * mask,
                                    [rgb[0] * e, rgb[1] * e, rgb[2] * e],
                                )
                            })
                        };
                        total += 1;
                        let gpu_live = g[2] >= 0.0;
                        match cpu {
                            None => {
                                if gpu_live {
                                    mismatched_liveness += 1;
                                }
                            }
                            Some((pos, wt, rgb)) => {
                                if !gpu_live {
                                    mismatched_liveness += 1;
                                    continue;
                                }
                                // The spectral half (K-364): the ray's
                                // band-integrated energy, relative to its
                                // own magnitude. This is what the eight
                                // per-sub throughputs and the baked
                                // reflectance table actually produce — the
                                // weight below is now geometry alone, so
                                // without this the trace's radiometry would
                                // go unchecked corner for corner.
                                let cmax = rgb.iter().fold(0.0f32, |a, &b| a.max(b));
                                if cmax > 1e-7 {
                                    let rerr = (0..3)
                                        .map(|c| (g[4 + c] - rgb[c]).abs())
                                        .fold(0.0f32, f32::max)
                                        / cmax;
                                    rgb_errs.push(rerr);
                                    worst_rgb = worst_rgb.max(rerr);
                                }
                                // Position agreement is claimed only for rays
                                // CARRYING light. A K-264 virtual
                                // continuation (a mount-absorbed miss) has
                                // weight ~0 and its path may branch-flip on
                                // a few-ULP difference at the miss boundary
                                // — real geometry for the raster, no light,
                                // and no meaningful "true" position to pin.
                                let werr = (g[2] - wt).abs() / wt.max(2e-4);
                                weight_errs.push(werr);
                                worst_weight = worst_weight.max(werr);
                                if wt <= 1e-3 {
                                    continue;
                                }
                                live += 1;
                                let pos_err = (g[0] - pos[0]).abs().max((g[1] - pos[1]).abs());
                                sum_pos += pos_err;
                                pos_errs.push(pos_err);
                                worst_pos = worst_pos.max(pos_err);
                            }
                        }
                    }
                }
            }
            eprintln!(
                "lens {lens} light {light_frac:?}: pos {worst_pos}px weight-rel {worst_weight} rgb-rel {worst_rgb}"
            );
            // Mean position error is what a porting bug blows up by orders
            // of magnitude; the tail is pinned at the 99th percentile (a
            // few-ULP input difference near a caustic fold legitimately
            // lands a ray on the other branch, far away).
            let mean_pos = sum_pos / live.max(1) as f32;
            assert!(mean_pos < 0.2, "mean position error {mean_pos} px");
            let p99 = |v: &mut Vec<f32>| {
                v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                v[(v.len() * 99) / 100]
            };
            let pos_p99 = p99(&mut pos_errs);
            assert!(
                pos_p99 < 3.0,
                "p99 position error {pos_p99} px (worst {worst_pos})"
            );
            let w_p99 = p99(&mut weight_errs);
            assert!(
                w_p99 < 0.05,
                "p99 weight error {w_p99} (worst {worst_weight})"
            );
            assert!(
                rgb_errs.len() > 100,
                "too few rays carried energy ({}) to check the spectral walk",
                rgb_errs.len()
            );
            let rgb_p99 = p99(&mut rgb_errs);
            assert!(
                rgb_p99 < 0.05,
                "p99 spectral rgb error {rgb_p99} (worst {worst_rgb})"
            );
            let flip_rate = mismatched_liveness as f32 / total.max(1) as f32;
            assert!(
                flip_rate < 0.01,
                "lens {lens}: {mismatched_liveness}/{total} rays flipped live/dead"
            );
            assert!(live > 100, "too few live rays ({live}) to mean anything");
        }
    }
}

/// **Sprite flare** (docs/08 §3.29, K-359): the WGSL agrees with the CPU
/// reference, the neutral points pass through bit-exactly, and — the property
/// the whole effect exists for — moving the light moves the flare *smoothly*,
/// with no threshold to pop across.
#[test]
fn wgsl_sprite_flare_matches_the_cpu_oracle_and_never_pops() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (64u32, 48u32);
    let img = corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let params = |lx: f32| lumit_core::fx::cpu::SpriteFlareParams {
        light: [lx, 18.0],
        intensity: 1.0,
        tint: [1.0, 0.9, 0.7],
        glow_size: 14.0,
        glow_intensity: 1.0,
        ghosts: 5,
        ghost_spacing: 0.35,
        ghost_size: 10.0,
        ghost_intensity: 0.5,
        streak_length: 30.0,
        streak_intensity: 0.6,
        streak_angle_deg: 12.0,
        mix: 1.0,
    };
    let op = |p: &lumit_core::fx::cpu::SpriteFlareParams| crate::fx::SpriteFlareOp {
        light: p.light,
        intensity: p.intensity,
        tint: p.tint,
        glow_size: p.glow_size,
        glow_intensity: p.glow_intensity,
        ghosts: p.ghosts,
        ghost_spacing: p.ghost_spacing,
        ghost_size: p.ghost_size,
        ghost_intensity: p.ghost_intensity,
        streak_length: p.streak_length,
        streak_intensity: p.streak_intensity,
        streak_angle_deg: p.streak_angle_deg,
        mix: p.mix,
    };

    let p = params(20.0);
    let mut cpu = img.clone();
    lumit_core::fx::cpu::sprite_flare(&mut cpu, w, h, &p);
    let out = fx.sprite_flare(&ctx, &tex, w, h, &op(&p));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

    let added: f32 = gpu.iter().zip(&img).map(|(a, b)| (a - b).abs()).sum();
    assert!(added > 1e-2, "the flare drew nothing ({added})");
    let worst = worst_diff(&cpu, &gpu);
    eprintln!("sprite_flare: worst {worst:.2e}");
    assert!(worst < 5e-3, "worst |Δ| {worst}");

    // Bit-stable.
    let again = fx.sprite_flare(&ctx, &tex, w, h, &op(&p));
    assert_eq!(
        gpu,
        readback_linear_f32(&ctx, &again, w, h).unwrap(),
        "the sprite flare must be bit-stable"
    );

    // **It cannot pop.** This is the whole reason the effect exists beside the
    // physically simulated one: there is no bright-pass, so nudging the light
    // by a pixel nudges the picture by a little. A threshold-driven flare
    // fails this — a source crossing the gate appears all at once.
    let mut previous: Option<Vec<f32>> = None;
    let mut worst_step = 0.0f32;
    for step in 0..6 {
        let q = params(20.0 + step as f32);
        let mut frame = img.clone();
        lumit_core::fx::cpu::sprite_flare(&mut frame, w, h, &q);
        if let Some(prev) = &previous {
            worst_step = worst_step.max(worst_diff(prev, &frame));
        }
        previous = Some(frame);
    }
    assert!(
        worst_step < 0.35,
        "a one-pixel move of the light changed a pixel by {worst_step} — \
         the flare must slide, not pop"
    );

    // Neutral points: Intensity 0 and Mix 0 are the input, untouched.
    for neutral in [
        lumit_core::fx::cpu::SpriteFlareParams {
            intensity: 0.0,
            ..p
        },
        lumit_core::fx::cpu::SpriteFlareParams { mix: 0.0, ..p },
    ] {
        let nout = fx.sprite_flare(&ctx, &tex, w, h, &op(&neutral));
        assert_eq!(
            readback_linear_f32(&ctx, &nout, w, h).unwrap(),
            img,
            "a neutral sprite flare must be the bit-exact input"
        );
    }
}

/// **Light wrap** (docs/08 §3.28, K-358): the WGSL agrees with the CPU
/// reference, the neutral points pass the input through bit-exactly, and the
/// wrap lands where it should — inside the foreground's edge, nowhere else.
#[test]
fn wgsl_light_wrap_matches_the_cpu_oracle_and_stays_inside_the_edge() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (48u32, 32u32);

    // A foreground: an opaque square in the middle of an empty frame, so it
    // has a real edge with transparency on both sides of it.
    let mut fg = vec![0.0f32; (w * h * 4) as usize];
    for y in 8..24u32 {
        for x in 12..36u32 {
            let i = ((y * w + x) * 4) as usize;
            fg[i] = 0.1;
            fg[i + 1] = 0.1;
            fg[i + 2] = 0.1;
            fg[i + 3] = 1.0;
        }
    }
    let fg: Vec<f32> = fg.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    // A background bright enough that its spill is unmistakable.
    let bg: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [2.0f32, 0.5, 0.25, 1.0])
        .map(|v| f16_to_f32(f16_bits(v)))
        .collect();

    let fg_tex = upload_linear_f32(&ctx, &fg, w, h);
    let bg_tex = upload_linear_f32(&ctx, &bg, w, h);

    for (width, intensity, mix) in [(6.0f32, 1.0f32, 1.0f32), (3.0, 0.5, 0.75)] {
        let mut cpu = fg.clone();
        lumit_core::fx::cpu::light_wrap(&mut cpu, &bg, w, h, width, intensity, mix);

        let out = fx.light_wrap(
            &ctx,
            &fg_tex,
            w,
            h,
            &bg_tex,
            &crate::fx::LightWrapOp {
                width_px: width,
                intensity,
                mix,
            },
        );
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        // The effect must actually do something, or the bound below passes
        // by rendering nothing.
        let added: f32 = gpu.iter().zip(&fg).map(|(a, b)| (a - b).abs()).sum();
        assert!(added > 1e-2, "width {width}: no wrap was drawn ({added})");

        let worst = worst_diff(&cpu, &gpu);
        eprintln!("light_wrap w={width} i={intensity} mix={mix}: worst {worst:.2e}");
        assert!(worst < 5e-3, "width {width}: worst |Δ| {worst}");

        // **Nowhere outside the matte.** A wrap that painted on transparent
        // pixels would grow a halo round the subject, which is the classic way
        // to get this wrong.
        for i in (0..gpu.len()).step_by(4) {
            if fg[i + 3] == 0.0 {
                for c in 0..3 {
                    assert_eq!(
                        gpu[i + c],
                        fg[i + c],
                        "the wrap leaked outside the matte at texel {}",
                        i / 4
                    );
                }
            }
        }

        // And it is the same picture every time (docs/14 §2.4).
        let again = fx.light_wrap(
            &ctx,
            &fg_tex,
            w,
            h,
            &bg_tex,
            &crate::fx::LightWrapOp {
                width_px: width,
                intensity,
                mix,
            },
        );
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &again, w, h).unwrap(),
            "light wrap must be bit-stable"
        );
    }

    // Deep inside the subject, far from any edge, nothing changes: the wrap is
    // an EDGE treatment, not a grade.
    let mut cpu = fg.clone();
    lumit_core::fx::cpu::light_wrap(&mut cpu, &bg, w, h, 4.0, 1.0, 1.0);
    let middle = (((16 * w) + 24) * 4) as usize;
    for c in 0..3 {
        assert!(
            (cpu[middle + c] - fg[middle + c]).abs() < 1e-6,
            "the middle of the subject must be untouched"
        );
    }
}

/// **An area light flares like an area, not like a point** (K-355).
///
/// Source size gives the light a real emitting area, and the flare of one is
/// the sum of the point flares across it. So the picture must genuinely change
/// — a wider source spreads its ghosts — while the total light it adds stays
/// put, because every sample carries a share of one light's flux rather than a
/// light of its own. A source that grew brighter as it grew wider would be the
/// obvious way to get this wrong, and is what the energy bound below catches.
#[test]
fn an_area_source_spreads_its_flare_without_gaining_energy() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (128u32, 72u32);
    let img = corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let render = |p: lf::LensFlareParams| {
        let op = flare_op(&p, w, h);
        let out = fx.lens_flare(
            &ctx,
            &tex,
            w,
            h,
            &op,
            None,
            &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
            &flare_probe(&p, w, h),
        );
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };

    let point = flare_params();
    let area = lf::LensFlareParams {
        // Half-extents in raster pixels: a wide, short strip, like a tube.
        source_size: [18.0, 4.0],
        ..point
    };
    assert!(
        lf::expand_area_lights(&lf::manual_light(&area, w, h), lf::AREA_SAMPLES_MAX).len() > 1,
        "the test's own source must actually be an area one"
    );

    let a = render(point);
    let b = render(area);
    assert_ne!(a, b, "an area source must not render as a point does");

    // Energy is conserved: the samples share one light's flux.
    let energy = |v: &[f32]| -> f32 { v.iter().zip(&img).map(|(x, y)| (x - y).abs()).sum::<f32>() };
    let (ea, eb) = (energy(&a), energy(&b));
    assert!(ea > 1e-2, "the point flare must be visible: {ea}");
    let ratio = eb / ea.max(1e-9);
    assert!(
        (0.5..=1.5).contains(&ratio),
        "an area source must spread its light, not multiply it: {ratio} \
         ({eb} vs {ea})"
    );

    // And it is still the same picture every time.
    assert_eq!(b, render(area), "an area source must be bit-stable too");
}

/// **The flare is bit-stable across repeated renders, in every shape of
/// frame** (K-353, docs/14 determinism, impl/lens-flare.md §2.4).
///
/// The shipped bit-stability check renders twice with one set of parameters.
/// That was not enough to catch what was actually wrong: the flare's raster
/// drew into a 4x multisample target, and additively blending fp16 into one
/// is not reproducible run to run on this hardware — the same frame came
/// back a few hundred fp16 ULPs different each time, in different places.
/// Four runs across configurations that switch each stage off in turn is
/// what localised it, and it is what would catch a return of it: the frame
/// varied whatever the ghost blur and the starburst were doing, and varied
/// down to a single ghost at minimum detail.
///
/// `added` is asserted alongside, because a configuration that renders
/// nothing is trivially stable and would prove nothing — that is exactly how
/// the first attempt at this measurement fooled itself.
#[test]
fn the_flare_renders_the_same_frame_every_time() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (128u32, 72u32);
    let img = corpus(w, h);
    let base = flare_params();
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Bisect by stage. Each configuration switches one stage off; the one
    // whose absence makes the frame stable is the one introducing variance.
    // `added` proves the flare actually drew — a configuration that renders
    // nothing is trivially "stable" and says nothing.
    for (name, p) in [
        ("all stages", base),
        (
            "no ghost blur",
            lf::LensFlareParams {
                ghost_softness: 0.0,
                ..base
            },
        ),
        (
            "no starburst",
            lf::LensFlareParams {
                starburst_intensity: 0.0,
                ..base
            },
        ),
        (
            "one ghost only",
            lf::LensFlareParams {
                max_ghosts: 1,
                ..base
            },
        ),
        (
            "one ghost, no blur, no starburst",
            lf::LensFlareParams {
                max_ghosts: 1,
                ghost_softness: 0.0,
                starburst_intensity: 0.0,
                ..base
            },
        ),
        (
            "minimal: 1 ghost, no dispersion, min detail",
            lf::LensFlareParams {
                max_ghosts: 1,
                ghost_softness: 0.0,
                starburst_intensity: 0.0,
                dispersion: 0.0,
                detail: 0.0,
                ..base
            },
        ),
    ] {
        let op = flare_op(&p, w, h);
        let mut runs: Vec<Vec<f32>> = Vec::new();
        for _ in 0..4 {
            let out = fx.lens_flare(
                &ctx,
                &tex,
                w,
                h,
                &op,
                None,
                &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
                &flare_probe(&p, w, h),
            );
            runs.push(readback_linear_f32(&ctx, &out, w, h).unwrap());
        }
        let added: f32 = runs[0]
            .iter()
            .zip(&img)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / (w * h) as f32;
        assert!(
            added > 1e-4,
            "{name}: renders nothing ({added:e}), so stability would be vacuous"
        );
        for i in 1..runs.len() {
            let differing = runs[0].iter().zip(&runs[i]).filter(|(x, y)| x != y).count();
            assert_eq!(
                differing,
                0,
                "{name}: run {i} differs from run 0 in {differing} floats \
                 (worst {:e}) — the flare must render the same frame every time",
                worst_diff(&runs[0], &runs[i])
            );
        }
    }

    // And the trace under it, which is where a variance would be worst: every
    // stage downstream reads these ray landings.
    let p = base;
    let op = flare_op(&p, w, h);
    let bake = std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake;
    let first = fx.lens_flare_trace_debug(&ctx, &op, &bake, 8, w, h);
    assert!(!first.is_empty(), "the trace oracle hook rendered no rays");
    for i in 1..3 {
        let again = fx.lens_flare_trace_debug(&ctx, &op, &bake, 8, w, h);
        assert_eq!(first, again, "trace run {i} differs from run 0");
    }
}

/// Impl note §8.6 + §8.7: the full GPU frame (trace → raster → combine)
/// stays within the perceptual bound of the CPU scanline reference, the
/// flare is actually visible (the energy floor that keeps the bound honest),
/// Intensity 0 and Mix 0 are bit-exact passthroughs, and the render is
/// bit-stable across two runs (§2.4).
#[test]
fn wgsl_lens_flare_matches_the_cpu_frame_reference_and_neutrals() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (128u32, 72u32);
    let img = corpus(w, h);
    let p = flare_params();
    let baked = lf::bake(&p);
    let op = flare_op(&p, w, h);

    // CPU reference: flare at the Draft half-size buffer, then the combine.
    let (_, _, div) = lf::quality_ladder(p.quality);
    let (fw, fh) = ((w / div).max(1), (h / div).max(1));
    let lights = lf::manual_light(&p, w, h);
    let flare = lf::cpu_flare(&p, &baked, fw, fh, &lights);
    let mut cpu = img.clone();
    lf::cpu_combine(&mut cpu, w, h, &p, &baked, &flare, fw, fh, &lights);

    // GPU.
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

    // The flare must be visible — otherwise the perceptual bound below
    // passes vacuously (and the energy-scale constant has rotted).
    let added: f32 = gpu
        .iter()
        .zip(&img)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / (w * h) as f32;
    assert!(
        added > 1e-4,
        "the default flare adds no visible energy: {added}"
    );

    // Perceptual bound (impl note §8.6): mean |Δ| and total-energy ratio.
    // Per-pixel differences at triangle edges are legitimate; the mean is
    // what pins agreement.
    let mean: f32 = cpu
        .iter()
        .zip(&gpu)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / cpu.len() as f32;
    assert!(mean < 2e-3, "mean |Δ| {mean}");
    let e_cpu: f32 = cpu.iter().sum();
    let e_gpu: f32 = gpu.iter().sum();
    let ratio = e_gpu / e_cpu.max(1e-9);
    assert!(
        (0.99..=1.01).contains(&ratio),
        "energy ratio {ratio} ({e_gpu} vs {e_cpu})"
    );

    // Determinism (§2.4): a second run is bit-identical.
    let out2 = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
    assert_eq!(gpu, gpu2, "GPU lens flare must be bit-stable");

    // Neutral points: Intensity 0 and Mix 0 pass the input through
    // bit-exactly (fp16 texel in, identical fp16 texel out).
    for neutral in [
        lf::LensFlareParams {
            intensity: 0.0,
            ..p
        },
        lf::LensFlareParams { mix: 0.0, ..p },
    ] {
        let nop = flare_op(&neutral, w, h);
        let nout = fx.lens_flare(
            &ctx,
            &tex,
            w,
            h,
            &nop,
            None,
            &(std::sync::Arc::new(move || flare_bake_data(&neutral)) as crate::fx::FlareBake),
            &flare_probe(&neutral, w, h),
        );
        let ngpu = readback_linear_f32(&ctx, &nout, w, h).unwrap();
        assert_eq!(ngpu, img, "neutral point must be bit-exact");
    }
}

/// K-267: an anamorphic squeeze below 1 renders into a PADDED flare buffer,
/// so the widened field carries real flare where K-266's zero-outside tap
/// showed black — and the padded pipeline still matches the CPU reference.
/// Fails without the padding: the region past the base buffer's edge is
/// exactly zero on the GPU, and the edge-energy floor below trips.
#[test]
fn wgsl_lens_flare_padded_anamorphic_matches_and_fills_the_edge() {
    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (128u32, 72u32);
    let img = vec![0.0f32; (w * h * 4) as usize];
    let p = lf::LensFlareParams {
        anamorphic: 0.5,
        ghost_softness: 0.0,
        starburst_intensity: 0.0,
        ..flare_params()
    };
    let baked = lf::bake(&p);
    let op = flare_op(&p, w, h);

    let (_, _, div) = lf::quality_ladder(p.quality);
    let (fw, fh) = ((w / div).max(1), (h / div).max(1));
    let lights = lf::manual_light(&p, w, h);
    let flare = lf::cpu_flare(&p, &baked, fw, fh, &lights);
    // The buffer really is padded: 2x wide at squeeze 0.5.
    let (rw, rh) = lf::flare_pad_dims(fw, fh, p.anamorphic, p.scale);
    assert_eq!((rw, rh), (fw * 2, fh), "squeeze 0.5 must double the width");
    assert_eq!(flare.len(), (rw * rh * 3) as usize);
    let mut cpu = img.clone();
    lf::cpu_combine(&mut cpu, w, h, &p, &baked, &flare, fw, fh, &lights);

    let tex = upload_linear_f32(&ctx, &img, w, h);
    let out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

    // The squeezed field must carry energy in the outer horizontal fifths
    // of the frame — the zone the un-padded buffer rendered black.
    let fifth = (w / 5) as usize;
    let mut edge = 0.0f32;
    let mut total = 0.0f32;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let v = gpu[(y * w as usize + x) * 4]
                + gpu[(y * w as usize + x) * 4 + 1]
                + gpu[(y * w as usize + x) * 4 + 2];
            total += v;
            if x < fifth || x >= w as usize - fifth {
                edge += v;
            }
        }
    }
    assert!(total > 1e-2, "no flare rendered at all: {total}");
    assert!(
        edge / total.max(1e-9) > 0.02,
        "squeezed flare leaves the frame edges black: edge share {}",
        edge / total.max(1e-9)
    );

    // And the padded path still agrees with the CPU reference.
    let mean: f32 = cpu
        .iter()
        .zip(&gpu)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / cpu.len() as f32;
    assert!(mean < 2e-3, "mean |Δ| {mean}");
    let e_cpu: f32 = cpu.iter().sum();
    let e_gpu: f32 = gpu.iter().sum();
    let ratio = e_gpu / e_cpu.max(1e-9);
    assert!(
        (0.99..=1.01).contains(&ratio),
        "energy ratio {ratio} ({e_gpu} vs {e_cpu})"
    );
}

/// Matte mode (docs/08 §3.27, K-257): the GPU detection + per-light flare
/// agrees with the CPU reference (detect_lights → cpu_flare → cpu_combine)
/// at the frame bound, the detected flares actually render, and the shared
/// constants the two crates must agree on are pinned.
#[test]
fn wgsl_lens_flare_matte_mode_matches_the_cpu_reference() {
    assert_eq!(MAX_LIGHTS as usize, lumit_core::fx::lens_flare::MAX_LIGHTS);
    // The combine kernel's `flare_blend` implements exactly the menu
    // lumit-core declares (K-289) — a mode added to one and not the other
    // would silently clamp to Divide.
    assert_eq!(
        crate::fx::lens_flare::BLEND_COUNT as usize,
        lumit_core::fx::lens_flare::BLEND_OPTIONS.len()
    );

    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (128u32, 72u32);
    // A dark scene as the layer input, fp16-quantised AFTER the scale so the
    // bit-exact passthrough assert below sees exactly what the GPU uploads.
    let img: Vec<f32> = corpus(w, h)
        .iter()
        .map(|v| f16_to_f32(f16_bits(v * 0.05)))
        .collect();
    // The matte: two bright sources on black, fp16-quantised exactly as the
    // GPU texture upload rounds it.
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    for (x, y, rgb) in [
        (30u32, 20u32, [5.0f32, 4.0, 3.0]),
        (100, 50, [2.0, 2.5, 3.0]),
    ] {
        let i = ((y * w + x) * 4) as usize;
        matte[i] = rgb[0];
        matte[i + 1] = rgb[1];
        matte[i + 2] = rgb[2];
        matte[i + 3] = 1.0;
    }
    let matte: Vec<f32> = matte.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();

    let p = lf::LensFlareParams {
        source: 1,
        threshold: 1.0,
        threshold_softness: 0.25,
        max_ghosts: 6,
        ..flare_params()
    };
    let baked = lf::bake(&p);
    let op = flare_op(&p, w, h);

    // CPU: detect on the quantised matte, then render per light.
    let lights = lf::detect_lights(
        &matte,
        w,
        h,
        p.threshold,
        p.threshold_softness,
        p.use_source_colour,
        p.light_tint,
    );
    assert_eq!(lights.len(), 2, "both sources must be found: {lights:?}");
    let (_, _, div) = lf::quality_ladder(p.quality);
    let (fw, fh) = ((w / div).max(1), (h / div).max(1));
    let flare = lf::cpu_flare(&p, &baked, fw, fh, &lights);
    let mut cpu = img.clone();
    lf::cpu_combine(&mut cpu, w, h, &p, &baked, &flare, fw, fh, &lights);

    // GPU.
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let matte_tex = upload_linear_f32(&ctx, &matte, w, h);
    let out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        Some(&matte_tex),
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

    // The detected flares must be visible…
    let added: f32 = gpu
        .iter()
        .zip(&img)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / (w * h) as f32;
    assert!(added > 1e-4, "matte mode added no visible energy: {added}");
    // …and match the CPU reference at the frame bound.
    let mean: f32 = cpu
        .iter()
        .zip(&gpu)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / cpu.len() as f32;
    assert!(mean < 2e-3, "mean |Δ| {mean}");
    let e_cpu: f32 = cpu.iter().sum();
    let e_gpu: f32 = gpu.iter().sum();
    let ratio = e_gpu / e_cpu.max(1e-9);
    assert!((0.99..=1.01).contains(&ratio), "energy ratio {ratio}");

    // An unset matte in Matte mode is the labelled no-op: bit-exact input.
    let nout = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &op,
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let ngpu = readback_linear_f32(&ctx, &nout, w, h).unwrap();
    assert_eq!(ngpu, img, "matte mode without a matte must pass through");

    // Source colour OFF with a warm Light tint (K-259): the GPU detection
    // must build the same lights the CPU does, and the frame must still
    // agree — this is the path where the matte says only *where*.
    let tinted = lf::LensFlareParams {
        use_source_colour: false,
        light_tint: [1.0, 0.6, 0.3],
        ..p
    };
    let t_lights = lf::detect_lights(
        &matte,
        w,
        h,
        tinted.threshold,
        tinted.threshold_softness,
        tinted.use_source_colour,
        tinted.light_tint,
    );
    assert_eq!(t_lights.len(), 2);
    for l in &t_lights {
        assert!(
            l.rgb[0] > l.rgb[2],
            "the warm tint must dominate the source colour: {l:?}"
        );
    }
    let t_flare = lf::cpu_flare(&tinted, &baked, fw, fh, &t_lights);
    let mut t_cpu = img.clone();
    lf::cpu_combine(
        &mut t_cpu, w, h, &tinted, &baked, &t_flare, fw, fh, &t_lights,
    );
    let t_out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &flare_op(&tinted, w, h),
        Some(&matte_tex),
        &(std::sync::Arc::new(move || flare_bake_data(&tinted)) as crate::fx::FlareBake),
        &flare_probe(&tinted, w, h),
    );
    let t_gpu = readback_linear_f32(&ctx, &t_out, w, h).unwrap();
    let t_mean: f32 = t_cpu
        .iter()
        .zip(&t_gpu)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / t_cpu.len() as f32;
    assert!(t_mean < 2e-3, "tinted matte mean |Δ| {t_mean}");
}
/// Render every bundled lens through the real GPU pipeline into one tiled
/// montage (K-264) — the harness the curation was chosen with, kept because
/// "do the twenty look different" is a question only eyes answer.
/// `LUMIT_FLARE_DUMP` names the output PPM.
#[test]
#[ignore = "a diagnostic image dump, not a gate"]
fn lens_flare_montage() {
    let Ok(ctx) = GpuContext::headless() else {
        return;
    };
    let fx = FxEngine::new(&ctx);
    use lumit_core::fx::lens_flare as lf;
    let picks: [u32; 20] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
    ];
    let (tw, th) = (288u32, 162u32);
    let (cols, rows) = (5u32, 4u32);
    let (mw, mh) = (tw * cols, th * rows);
    let mut canvas = vec![0u8; (mw * mh * 3) as usize];
    let img = vec![0.0f32; (tw * th * 4) as usize];
    let tex = upload_linear_f32(&ctx, &img, tw, th);
    for (k, &lens) in picks.iter().enumerate() {
        let p = lf::LensFlareParams {
            light: [0.68 * tw as f32, 0.30 * th as f32],
            lens,
            quality: 1,
            max_ghosts: 60,
            ghost_softness: 0.0,
            ..flare_params()
        };
        let op = flare_op(&p, tw, th);
        let out = fx.lens_flare(
            &ctx,
            &tex,
            tw,
            th,
            &op,
            None,
            &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
            &flare_probe(&p, tw, th),
        );
        let gpu = readback_linear_f32(&ctx, &out, tw, th).unwrap();
        let (ox, oy) = ((k as u32 % cols) * tw, (k as u32 / cols) * th);
        for y in 0..th {
            for x in 0..tw {
                let i = ((y * tw + x) * 4) as usize;
                let o = (((oy + y) * mw + ox + x) * 3) as usize;
                for c in 0..3 {
                    let v = (gpu[i + c] * 6.0).clamp(0.0, 1.0).powf(1.0 / 2.2);
                    canvas[o + c] = (v * 255.0).round() as u8;
                }
            }
        }
        eprintln!("tile {k}: lens {lens} done");
    }
    let mut ppm = format!("P6\n{mw} {mh}\n255\n").into_bytes();
    ppm.extend_from_slice(&canvas);
    std::fs::write(std::env::var("LUMIT_FLARE_DUMP").unwrap(), ppm).unwrap();
}

/// The §1.6 oracle rule applied to the lighting pass (docs/06, K-361), which
/// is not an effect but is held to the same standard: the kernel must agree
/// with `lumit_core::lighting::shade`, be bit-stable, and leave the picture
/// untouched to the byte when nothing lights it.
///
/// The tolerance is looser than a pointwise kernel's 2 ULP because the sum is
/// four `acos` calls deep and the two paths do not have to agree on the last
/// bit of a transcendental — an absolute epsilon, as the blur oracle uses.
#[test]
fn wgsl_lighting_matches_the_cpu_oracle() {
    use lumit_core::lighting::{ShadingLight, ShadingSurface};

    let Ok(ctx) = GpuContext::headless() else {
        crate::no_adapter();
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);

    // A layer lying in the z = 0 plane at 1:1, facing the viewer.
    let surface = ShadingSurface {
        origin: [0.0, 0.0, 0.0],
        du: [1.0, 0.0, 0.0],
        dv: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, -1.0],
    };
    let rect = |cx: f32, cy: f32, cz: f32, half: f32| {
        [
            [cx - half, cy - half, cz],
            [cx + half, cy - half, cz],
            [cx + half, cy + half, cz],
            [cx - half, cy + half, cz],
        ]
    };
    let softbox = ShadingLight {
        corners: rect(16.0, 12.0, -40.0, 30.0),
        colour: [1.0, 0.8, 0.6],
        falloff_px: 0.0,
        is_area: true,
        cone_cos: -2.0,
        axis: [0.0, 0.0, 1.0],
    };
    let bulb = ShadingLight {
        corners: [[4.0, 4.0, -20.0]; 4],
        colour: [0.2, 0.4, 1.0],
        falloff_px: 120.0,
        is_area: false,
        cone_cos: -2.0,
        axis: [0.0, 0.0, 1.0],
    };
    let spot = ShadingLight {
        corners: [[16.0, 12.0, -60.0]; 4],
        colour: [1.0, 1.0, 1.0],
        falloff_px: 0.0,
        is_area: false,
        cone_cos: 25f32.to_radians().cos(),
        axis: [0.0, 0.0, 1.0],
    };
    // A light sunk behind the layer: the horizon clip is what makes this
    // nothing rather than nonsense, so it belongs in the comparison.
    let behind = ShadingLight {
        corners: rect(16.0, 12.0, 50.0, 30.0),
        colour: [1.0, 1.0, 1.0],
        falloff_px: 0.0,
        is_area: true,
        cone_cos: -2.0,
        axis: [0.0, 0.0, 1.0],
    };

    for (name, lights) in [
        ("none", vec![]),
        ("softbox", vec![softbox]),
        ("point", vec![bulb]),
        ("spot", vec![spot]),
        ("behind", vec![behind]),
        ("three", vec![softbox, bulb, spot]),
    ] {
        let mut cpu = img.clone();
        lumit_core::lighting::shade(&mut cpu, w, h, &surface, &lights);

        let op = crate::fx::LightingOp {
            origin: surface.origin,
            du: surface.du,
            dv: surface.dv,
            normal: surface.normal,
            lights: lights
                .iter()
                .map(|l| crate::fx::LightingLight {
                    corners: l.corners,
                    colour: l.colour,
                    falloff_px: l.falloff_px,
                    is_area: l.is_area,
                    cone_cos: l.cone_cos,
                    axis: l.axis,
                })
                .collect(),
        };
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.lighting(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!("lighting {name}: worst {worst}");
        assert!(worst < 2e-2, "{name}: worst absolute difference {worst}");

        if name == "none" || name == "behind" {
            assert_eq!(gpu, img, "{name}: must leave the picture exactly alone");
        } else {
            assert!(gpu != img, "{name}: the light must actually do something");
        }

        let out2 = fx.lighting(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "{name}: the lighting pass must be bit-stable");
    }
}
