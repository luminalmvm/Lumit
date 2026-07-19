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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
        return;
    };
    let fx = FxEngine::new(&ctx);
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // Sweeps the sample count too (FX-9/K-144): 9 (the historical density), a
    // denser 24, and both range ends, so the variable-count kernel matches.
    for (amount, angle, radial, samples, mix) in [
        (3.0f32, 0.0f32, false, 9i32, 1.0f32),
        (2.5, 33.0, false, 24, 0.6),
        (4.0, 0.0, true, 16, 1.0),
        (6.0, 10.0, false, 64, 1.0),
        (5.0, 0.0, true, 3, 1.0),
        (0.0, 90.0, false, 16, 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::spectral_split(&mut cpu, w, h, amount, angle, radial, samples, mix);

        let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, angle);
        let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples);
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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
/// since K-107): the GPU single-tap warp matches
/// `lumit_core::fx::cpu::datamosh` given the same -1 neighbour and flow
/// field, on a constant field and a varying one — the same two shapes
/// [`wgsl_motion_blur_matches_the_cpu_oracle`] exercises, since both
/// kernels read flow the same way. One bilinear tap, no multi-tap sum,
/// so it holds to the same ≤ 2 fp16 ULP cheap-class bound. The GPU is
/// bit-stable (§2.4); Intensity 0 is a bit-exact passthrough.
#[test]
fn wgsl_datamosh_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        eprintln!("no GPU adapter; skipping WGSL parity test");
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

    // Streak length (FX-14) scales the flow reach; the > 1 intensity case
    // exercises the open ceiling (K-135), which mix() extrapolates past the
    // moshed frame in both the CPU and GPU paths.
    for (field, intensity, streak, name) in [
        (&constant, 1.0f32, 1.0f32, "constant streak1"),
        (&varying, 0.6, 2.0, "varying streak2"),
        (&constant, 0.35, 4.0, "partial mix streak4"),
        (&varying, 1.4, 1.5, "over-unity streak1.5"),
    ] {
        let (u, v) = field;
        let cpu = lumit_core::fx::cpu::datamosh(&current, &prev, w, h, u, v, intensity, streak);
        // Datamosh reads only the flow .xy; confidence is irrelevant (empty).
        let flow_t = upload_flow_field(&ctx, u, v, &[], w, h);
        let op = DatamoshOp { intensity, streak };
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

    // Intensity 0 must be a bit-exact passthrough regardless of motion/streak.
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
            streak: 8.0,
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
    let maxf = (size - 1) as f32;
    let mut data = Vec::with_capacity(size * size * size);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                data.push(f([r as f32 / maxf, g as f32 / maxf, b as f32 / maxf]));
            }
        }
    }
    lumit_core::lut::Lut3d {
        size,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
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
        eprintln!("no GPU adapter; skipping WGSL parity test");
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

    let cases: [(&str, &lumit_core::lut::Lut3d, f32); 5] = [
        ("identity-full", &identity, 1.0),
        ("identity-mix0", &identity, 0.0),
        ("gamma-full", &gamma, 1.0),
        ("gamma-mixed", &gamma, 0.5),
        ("swap-rb", &swap, 1.0),
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
        let out = fx.lut(&ctx, &tex, w, h, &lut_tex, lut.size as u32, mix);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("lut {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");

        if name == "identity-mix0" {
            // Mix 0 is the bit-exact input on the GPU path.
            assert_eq!(gpu, img, "{name}: Mix 0 must be the bit-exact input");
        }

        // Determinism: a second run is bit-identical to the first (§2.4).
        let out2 = fx.lut(&ctx, &tex, w, h, &lut_tex, lut.size as u32, mix);
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
        &fx.lut(&ctx, &tex, w, h, &lut_tex, identity.size as u32, 1.0),
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

/// The CPU oracle for [`FxEngine::dof`]: byte-for-byte the WGSL kernel's
/// maths (the same CoC ramp with explicit min/max/mul, the same integer
/// disc taps in the same row-major order, box weighted, edges clamped,
/// the same `o*(1-mix)+v*mix` final blend). Consumes the fp16-quantised
/// image and the exact-f32 depth the GPU reads, so the two agree.
#[allow(clippy::too_many_arguments)]
fn dof_reference(
    img: &[f32],
    depth: &[f32],
    w: u32,
    h: u32,
    focus: f32,
    range: f32,
    near_aperture: f32,
    far_aperture: f32,
    depth_invert: bool,
    display: u32,
    mix: f32,
) -> Vec<f32> {
    let wi = w as i32;
    let hi = h as i32;
    let mut out = vec![0.0f32; img.len()];
    for y in 0..hi {
        for x in 0..wi {
            let pi = (y * wi + x) as usize;
            let oi = pi * 4;
            let raw = depth[pi];
            // Depth invert (swap near and far): the shader's
            // `select(raw, 1 - raw, invert)`, bit-identical here.
            let d = if depth_invert { 1.0 - raw } else { raw };
            let dist = (d - focus).abs();
            let denom = (1.0f32 - range).max(1e-4);
            // clamp(0,1) is bit-identical to the shader's min(max(·,0),1)
            // for every finite input (the ±0 corner collapses to the same
            // smoothstep zero and coincides in fp16), so parity holds.
            let e = ((dist - range) / denom).clamp(0.0, 1.0);
            let s = e * e * (3.0 - 2.0 * e);
            // Diagnostic views (mirror the kernel): write the view directly,
            // ignoring the disc gather and Mix.
            if display == 1 {
                // Depth map: post-invert depth as opaque greyscale.
                out[oi] = d;
                out[oi + 1] = d;
                out[oi + 2] = d;
                out[oi + 3] = 1.0;
                continue;
            }
            if display == 2 {
                // Focus map: 1 - s, white where sharp.
                let m = 1.0 - s;
                out[oi] = m;
                out[oi + 1] = m;
                out[oi + 2] = m;
                out[oi + 3] = 1.0;
                continue;
            }
            // Per-side aperture: the shader's
            // `select(far, near, d < focus)`, far at equality.
            let ap = if d < focus {
                near_aperture
            } else {
                far_aperture
            };
            let coc = ap * s;
            let coc2 = coc * coc;
            let ri = coc.ceil() as i32;
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let r2 = (dx * dx + dy * dy) as f32;
                    if r2 <= coc2 {
                        let sx = (x + dx).clamp(0, wi - 1);
                        let sy = (y + dy).clamp(0, hi - 1);
                        let si = ((sy * wi + sx) * 4) as usize;
                        acc[0] += img[si];
                        acc[1] += img[si + 1];
                        acc[2] += img[si + 2];
                        acc[3] += img[si + 3];
                        wsum += 1.0;
                    }
                }
            }
            for c in 0..4 {
                let v = acc[c] / wsum;
                let o = img[oi + c];
                out[oi + c] = o * (1.0 - mix) + v * mix;
            }
        }
    }
    out
}

/// The §1.6 oracle for the depth-of-field lens blur (foundation for the
/// planned DoF effects): the WGSL variable-radius disc blur matches
/// [`dof_reference`] over a depth ramp and several focus/aperture/mix
/// settings. A tap-summing gather like Motion blur, reading exact
/// (r32float) depth and the same fp16 source, so it holds to the cheap-
/// class ≤ 2 fp16 ULP bound; the GPU is bit-stable (§2.4); Mix 0, a zero
/// aperture, and a depth that sits everywhere inside the sharp band are
/// all bit-exact passthroughs.
#[test]
fn wgsl_dof_matches_the_cpu_oracle() {
    let Ok(ctx) = GpuContext::headless() else {
        eprintln!("no GPU adapter; skipping WGSL parity test");
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

    // (focus, range, near, far, invert, display, mix, name). Invert, an
    // asymmetric near/far pair, and every shipped Display mode all stay
    // continuous (the aperture select flips only where s == 0; Depth/Focus
    // maps are smooth in depth), so the cheap-class ≤ 2 fp16 ULP bound holds
    // across modes — none is excluded.
    let cases = [
        (
            0.5f32,
            0.1f32,
            6.0f32,
            6.0f32,
            false,
            0u32,
            1.0f32,
            "centre-focus",
        ),
        (0.0, 0.05, 8.0, 8.0, false, 0, 1.0, "near-focus"),
        (0.5, 0.1, 6.0, 6.0, false, 0, 0.5, "partial mix"),
        (0.5, 0.2, 10.0, 10.0, false, 0, 1.0, "wide aperture"),
        (0.2, 0.1, 8.0, 8.0, true, 0, 1.0, "inverted near-focus"),
        (0.5, 0.1, 6.0, 6.0, true, 0, 1.0, "inverted centre-focus"),
        (0.5, 0.05, 12.0, 3.0, false, 0, 1.0, "asymmetric near>far"),
        (0.5, 0.05, 3.0, 12.0, false, 0, 1.0, "asymmetric far>near"),
        (0.5, 0.05, 12.0, 3.0, true, 0, 1.0, "asymmetric inverted"),
        (0.5, 0.1, 8.0, 8.0, false, 1, 1.0, "depth map"),
        (0.5, 0.1, 8.0, 8.0, true, 1, 1.0, "depth map inverted"),
        (0.5, 0.1, 8.0, 8.0, false, 2, 1.0, "focus map"),
        (0.3, 0.15, 12.0, 4.0, false, 2, 1.0, "focus map asymmetric"),
    ];
    for (focus, range, near, far, invert, display, mix, name) in cases {
        let cpu = dof_reference(
            &img, &ramp, w, h, focus, range, near, far, invert, display, mix,
        );
        let out = fx.dof(
            &ctx, &src, w, h, &depth_t, focus, range, near, far, invert, display, mix,
        );
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("dof {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        // Determinism (§2.4): a second run is bit-identical to the first.
        let out2 = fx.dof(
            &ctx, &src, w, h, &depth_t, focus, range, near, far, invert, display, mix,
        );
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "{name}: GPU DoF must be bit-stable"
        );
    }

    // Mix 0 is the bit-exact input regardless of depth or aperture (Rendered
    // mode).
    let out = fx.dof(
        &ctx, &src, w, h, &depth_t, 0.5, 0.1, 10.0, 10.0, false, 0, 0.0,
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "Mix 0 must be the bit-exact input"
    );

    // Both apertures zero collapses every disc to the centre tap — a
    // bit-exact passthrough at full Mix, whatever the depth (invert cannot
    // change a zero radius).
    let out = fx.dof(&ctx, &src, w, h, &depth_t, 0.5, 0.1, 0.0, 0.0, true, 0, 1.0);
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "a zero aperture must be a bit-exact passthrough"
    );

    // A depth that sits everywhere inside the sharp band leaves the CoC at
    // zero for every pixel — also a bit-exact passthrough at full Mix,
    // even with large apertures. Inverting a flat 0.5 leaves it in-band.
    let flat = upload_depth_map(&ctx, &vec![0.5f32; n], w, h);
    let out = fx.dof(&ctx, &src, w, h, &flat, 0.5, 0.1, 10.0, 10.0, false, 0, 1.0);
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "an in-band depth must be a bit-exact passthrough"
    );
}
