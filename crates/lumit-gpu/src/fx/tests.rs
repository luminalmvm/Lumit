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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
            let out = fx.blur(&ctx, &tex, w, h, None, &op);
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
            let out2 = fx.blur(&ctx, &tex, w, h, None, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU blur must be bit-stable");
        }
    }
}

/// **The Matte-driven blur agrees op-for-op, and genuinely varies in width**
/// (K-395, docs/08 §2.6, §1.6).
///
/// Two claims, and both need making. The first is the ordinary parity one: the
/// matted path has its own arithmetic — a per-pixel radius, weights built in the
/// loop rather than precomputed once — and so needs its own oracle comparison,
/// at both Invert settings.
///
/// The second is the one that says the override was worth having. A flat matte
/// at a quarter must produce a blur that is *narrower*, not a full-width blur
/// faded back — which is what the generic dissolve gives, and what a wrong
/// implementation would silently produce. So the test also measures how far a
/// lone bright pixel's light reaches at two matte levels and insists the dim one
/// reaches less far. A dissolve cannot pass that: it changes the halo's height
/// everywhere, never its width.
#[test]
fn wgsl_matted_blur_matches_the_cpu_oracle_and_varies_in_width() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 16u32);

    // A left-to-right ramp of matte over an opaque picture with an HDR spike, so
    // the parity corpus covers every matte level at once.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            img[i] = 0.2;
            img[i + 1] = 0.3;
            img[i + 2] = 0.4;
            img[i + 3] = 1.0;
            let k = x as f32 / (w - 1) as f32;
            matte[i] = k;
            matte[i + 1] = k;
            matte[i + 2] = k;
            matte[i + 3] = 1.0;
        }
    }
    let spike = ((8 * w + 24) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);

    // Invert is the seam's business since K-425: the matte is prepared once
    // (`matte_prepare`, both paths) and the kernel reads it as it is. Running
    // the pair through here is what proves the kernel applies no invert of its
    // own any more.
    let plain_tex = upload_linear_f32(&ctx, &matte, w, h);
    for invert in [false, true] {
        let matte_tex = if invert {
            fx.matte_prepare(&ctx, &plain_tex, w, h, 0, true)
        } else {
            plain_tex.clone()
        };
        for (radius, mix) in [(6.0f32, 1.0f32), (9.0, 0.7)] {
            let quantised: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
            let mut qmatte: Vec<f32> = matte.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
            if invert {
                lumit_core::fx::cpu::matte_prepare(&mut qmatte, 0, true);
                qmatte = qmatte.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
            }
            let mut cpu = quantised.clone();
            lumit_core::fx::cpu::blur_gaussian_matted(&mut cpu, w, h, radius, 1, mix, &qmatte);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = BlurOp {
                radius_px: radius,
                edge: 1,
                mix,
            };
            let out = fx.blur(&ctx, &tex, w, h, Some(&matte_tex), &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let worst = worst_diff(&cpu, &gpu);
            assert!(
                worst < 2e-2,
                "invert {invert} radius {radius} mix {mix}: worst diff {worst}"
            );

            let out2 = fx.blur(&ctx, &tex, w, h, Some(&matte_tex), &op);
            assert_eq!(
                gpu,
                readback_linear_f32(&ctx, &out2, w, h).unwrap(),
                "the matted blur must be bit-stable"
            );
        }
    }

    // **The width probe.** One bright pixel on black, blurred at radius 10 under
    // a FLAT matte — once at 1.0, once at 0.25 — and the question is how far the
    // light reaches, not how bright it is at the centre.
    let reach = |k: f32| -> usize {
        let mut dot = vec![0.0f32; (w * h * 4) as usize];
        let c = ((8 * w + 24) * 4) as usize;
        dot[c..c + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        let flat: Vec<f32> = (0..(w * h) as usize).flat_map(|_| [k, k, k, 1.0]).collect();
        let tex = upload_linear_f32(&ctx, &dot, w, h);
        let mtex = upload_linear_f32(&ctx, &flat, w, h);
        let out = fx.blur(
            &ctx,
            &tex,
            w,
            h,
            Some(&mtex),
            &BlurOp {
                radius_px: 10.0,
                edge: 1,
                mix: 1.0,
            },
        );
        let got = readback_linear_f32(&ctx, &out, w, h).unwrap();
        // How many pixels to the right of centre still carry light.
        (25..w)
            .take_while(|x| got[((8 * w + x) * 4) as usize] > 1e-4)
            .count()
    };
    let wide = reach(1.0);
    let narrow = reach(0.25);
    assert!(
        wide > narrow + 2,
        "a quarter matte reached {narrow} px and a full one {wide} px — the \
         matte is not changing the blur's WIDTH, which is the whole reason this \
         effect claims its matte instead of taking the dissolve"
    );
    assert!(narrow > 0, "a quarter matte blurred nothing at all");
}

/// **An unbound matte leaves the blur exactly what it was** (K-258).
///
/// The override added a branch to the hot kernel, so the campaign's hardest
/// invariant now has a half that lives *inside* a kernel rather than beside it.
/// Two things are checked: that the oracle's empty-matte path is literally the
/// old function rather than a copy of it, and that the GPU's unbound path still
/// tracks it.
#[test]
fn an_unmatted_blur_is_the_pre_matte_blur() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (24u32, 16u32);
    let img: Vec<f32> = (0..(w * h * 4))
        .map(|i| match i % 4 {
            3 => 1.0,
            _ => (i % 17) as f32 / 17.0,
        })
        .collect();
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let op = BlurOp {
        radius_px: 4.0,
        edge: 1,
        mix: 1.0,
    };
    let unmatted = readback_linear_f32(&ctx, &fx.blur(&ctx, &tex, w, h, None, &op), w, h).unwrap();
    assert_ne!(unmatted, img, "the blur must actually have blurred");

    let mut oracle: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    lumit_core::fx::cpu::blur_gaussian_matted(&mut oracle, w, h, 4.0, 1, 1.0, &[]);
    let mut plain: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    lumit_core::fx::cpu::blur_gaussian(&mut plain, w, h, 4.0, 1, 1.0);
    assert_eq!(
        oracle, plain,
        "the oracle's empty-matte path must BE the old function, not a copy of it"
    );
    assert!(
        worst_diff(&plain, &unmatted) < 2e-2,
        "the unmatted GPU blur drifted from the unmatted oracle"
    );
}

/// **The Matte gates which pixels SEED the glow** (K-395, docs/08 §2.6, §1.6).
///
/// Parity first, at both Invert settings. Then the claim that makes the override
/// worth its branch: a glow gated by a matte covering only the left half must
/// still spill light *past* the matte's edge into the right half, because the
/// halo spreads from the seeds after the gate. Fading a finished glow by the
/// same matte cannot — that clips the halo to the matte and leaves the right
/// half exactly as it came in.
#[test]
fn wgsl_matted_glow_seeds_only_inside_the_matte_and_spills_past_it() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 16u32);

    // Two bright squares on black: one just inside the matte's right edge, one
    // well outside it, and a matte that covers the LEFT half only. Only the
    // first may seed, and its halo must not stop at the matte's edge.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let lit = (18..24).contains(&x) || (36..42).contains(&x);
            if lit && (6..10).contains(&y) {
                img[i] = 3.0;
                img[i + 1] = 3.0;
                img[i + 2] = 3.0;
                img[i + 3] = 1.0;
            }
            let k = f32::from(x < w / 2);
            matte[i] = k;
            matte[i + 1] = k;
            matte[i + 2] = k;
            matte[i + 3] = 1.0;
        }
    }

    let matte_tex = upload_linear_f32(&ctx, &matte, w, h);
    let op = GlowOp {
        radius_px: 6.0,
        threshold: 0.8,
        knee: 0.5,
        intensity: 1.0,
        tint: [1.0; 4],
        mix: 1.0,
    };
    // Invert arrives through the seam's prepare pass (K-425), both paths.
    for invert in [false, true] {
        let quantised: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
        let mut qmatte: Vec<f32> = matte.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
        let mtex = if invert {
            lumit_core::fx::cpu::matte_prepare(&mut qmatte, 0, true);
            fx.matte_prepare(&ctx, &matte_tex, w, h, 0, true)
        } else {
            matte_tex.clone()
        };
        let mut cpu = quantised.clone();
        lumit_core::fx::cpu::glow(&mut cpu, w, h, 6.0, 0.8, 0.5, 1.0, [1.0; 4], 1.0, &qmatte);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.glow(&ctx, &tex, w, h, Some(&mtex), &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        assert!(worst < 5e-3, "invert {invert}: worst diff {worst}");
    }

    // The picture claim, on the un-inverted run.
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let out = fx.glow(&ctx, &tex, w, h, Some(&matte_tex), &op);
    let got = readback_linear_f32(&ctx, &out, w, h).unwrap();
    let at = |x: u32, y: u32| got[((y * w + x) * 4) as usize];
    // Beside the RIGHT square, well outside it: no halo, because the matte was
    // black there and that square never seeded.
    assert!(
        at(33, 8) < 1e-3,
        "the right square glowed at {} — the matte did not gate the seed",
        at(33, 8)
    );
    // Just PAST the matte's edge, in line with the seeded square's halo: light,
    // because a seeded halo keeps spreading.
    assert!(
        at(26, 8) > 1e-3,
        "the halo stopped dead at the matte edge ({}) — that is a dissolve, not \
         a gated seed",
        at(26, 8)
    );
}

/// The §1.6 oracle for sharpen: WGSL agrees with the CPU reference on
/// the corpus across parameter sweeps, and is bit-stable (§2.4). The
/// internal gaussian's intermediates round through fp16 textures on the
/// GPU and stay f32 on the CPU, so the bound is an absolute epsilon:
/// 5e-3 ≈ 1–2 fp16 ULP at the corpus's HDR peak of 6.0 (measured worst
/// on NVIDIA: 2.9e-3).
#[test]
fn wgsl_sharpen_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.sharpen(&ctx, &tex, w, h, None, &op);
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

        let out2 = fx.sharpen(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.sharpen_simple(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("sharpen_simple {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "amount-zero" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact passthrough");
        }

        let out2 = fx.sharpen_simple(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU sharpen_simple must be bit-stable");
    }
}

/// The §1.6 oracle for RGB split: a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, and the GPU is bit-stable (§2.4).
#[test]
fn wgsl_rgb_split_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.rgb_split(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("rgb split a={amount} ang={angle} scale={scale:?}: worst {worst} ulp");
        assert!(
            worst <= 2,
            "amount {amount} angle {angle} scale {scale:?} mix {mix}: \
                 worst {worst} fp16 ULP"
        );

        let out2 = fx.rgb_split(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.spectral_split(&ctx, &tex, w, h, None, &op);
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

        let out2 = fx.spectral_split(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.chromatic_aberration(&ctx, &tex, w, h, None, &op);
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

        let out2 = fx.chromatic_aberration(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU chromatic aberration must be bit-stable");
    }
}

/// The §1.6 oracle for flash: a trivial pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, and the GPU is bit-stable (§2.4).
#[test]
fn wgsl_flash_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.colour_balance(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("colour balance {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" {
            assert_eq!(gpu, img, "neutral balance must be the bit-exact identity");
        }

        let out2 = fx.colour_balance(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU colour balance must be bit-stable");
    }
}

/// The §1.6 oracle for saturation: a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4),
/// and saturation 1 is the bit-exact identity on both paths.
#[test]
fn wgsl_saturation_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.saturation(&ctx, &tex, w, h, None, &op);
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

        let out2 = fx.saturation(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU saturation must be bit-stable");
    }
}

/// The §1.6 oracle for vibrancy (K-152): a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4), and
/// amount 0 is the bit-exact identity on both paths.
#[test]
fn wgsl_vibrancy_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.vibrancy(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("vibrancy {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" {
            assert_eq!(gpu, img, "neutral vibrancy must be the bit-exact identity");
        }

        let out2 = fx.vibrancy(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        pre_blur: 0.0,
        shrink_grow: 0.0,
        softness: 0.0,
        despot_black: 0.0,
        despot_white: 0.0,
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
        pre_blur: p.pre_blur,
        shrink_grow: p.shrink_grow,
        softness: p.softness,
        despot_black: p.despot_black,
        despot_white: p.despot_white,
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
        let blank = MaskFillOp::blank();
        let out = fx.matte_key(&ctx, &tex, w, h, &op, &blank, &blank);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("matte key {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "identity_mix0" {
            assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
        }

        let out2 = fx.matte_key(&ctx, &tex, w, h, &op, &blank, &blank);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU matte key must be bit-stable");
    }
}

/// **The §1.6 oracle for the Matte key's spatial controls** (K-546): Screen
/// pre-blur, shrink/grow, softness, despot black/white and the two garbage
/// masks agree with the CPU reference, one control at a time and then all at
/// once, and the GPU is bit-stable (§2.4).
///
/// Moderate-class now rather than cheap: the matte becomes a picture of its own
/// and travels through as many as seven fp16 passes, so the comparison is the
/// perceptual epsilon §1.6 allows a moderate effect, the one the Gaussian blur's
/// own oracle uses, scaled for an HDR corpus.
///
/// **The defaults are pinned bit-for-bit against the pointwise kernel**, which
/// is what "adding these controls changed nothing" means: with nothing asked for
/// and neither mask set, the staged path is not taken at all.
#[test]
fn wgsl_matte_key_spatial_matches_the_cpu_oracle() {
    use lumit_core::fx::MatteKeyParams;
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    // Corpus (§1.6): a green field sliding to red, brightness rising down the
    // frame, alpha in bands — plus two single-pixel specks, one black hole in
    // the kept side and one green fleck in the keyed side, so the despot has
    // something to find, and an HDR spike.
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let fx_ = x as f32 / (w - 1) as f32;
            let fy = y as f32 / (h - 1) as f32;
            let a = 0.25 + 0.75 * fy;
            img[i] = fx_ * a;
            img[i + 1] = (1.0 - fx_) * (0.4 + 0.6 * fy) * a;
            img[i + 2] = 0.25 * fx_ * a;
            img[i + 3] = a;
        }
    }
    // A lone screen-coloured pixel deep in the foreground (a hole), and a lone
    // foreground-coloured pixel deep in the screen (a fleck).
    let hole = ((12 * w + 26) * 4) as usize;
    img[hole..hole + 4].copy_from_slice(&[0.0, 0.6, 0.0, 1.0]);
    let fleck = ((12 * w + 4) * 4) as usize;
    img[fleck..fleck + 4].copy_from_slice(&[0.7, 0.1, 0.6, 1.0]);
    let spike = ((10 * w + 20) * 4) as usize;
    img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 0.5]);
    let img: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();

    let grey = [0.5f32, 0.5, 0.5, 1.0];
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
        pre_blur: 0.0,
        shrink_grow: 0.0,
        softness: 0.0,
        despot_black: 0.0,
        despot_white: 0.0,
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
        pre_blur: p.pre_blur,
        shrink_grow: p.shrink_grow,
        softness: p.softness,
        despot_black: p.despot_black,
        despot_white: p.despot_white,
        replace_method: p.replace_method,
        replace_colour: p.replace_colour,
        mix: p.mix,
    };
    let to_fill = |m: &lumit_core::fx::cpu::MaskFillParams| MaskFillOp {
        segments: m.segments,
        count: m.count,
        ramp: m.ramp,
        expansion: m.expansion,
    };

    // Two real masks off the mask model, flattened exactly as the carriage
    // flattens them (K-408): a rectangle over the left half to hold in, and one
    // over the top-right corner to cut out. The feather and expansion are set on
    // the polyline directly, which is where `mask_path_at` puts a mask's own.
    let rect = |x: f64, y: f64, rw: f64, rh: f64, feather: f32, expansion: f32| {
        let masks = vec![lumit_core::mask::Mask::rectangle(x, y, rw, rh)];
        let mut poly = lumit_core::mask::mask_path_at(&masks, None, true, 0.0);
        poly.feather = feather;
        poly.expansion = expansion;
        poly
    };
    let inside_poly = rect(2.0, 2.0, 10.0, 20.0, 3.0, 0.0);
    let outside_poly = rect(20.0, 1.0, 10.0, 8.0, 0.0, -1.5);
    let blank = lumit_core::fx::cpu::MaskFillParams::blank();
    let inside = lumit_core::fx::cpu::mask_fill_params(&inside_poly, 1.0);
    let outside = lumit_core::fx::cpu::mask_fill_params(&outside_poly, 1.0);
    assert!(inside.count >= 4, "the hold-out flattened to nothing");
    assert!(outside.count >= 4, "the cut-out flattened to nothing");

    for (name, p, ins, outs) in [
        (
            "pre_blur",
            MatteKeyParams {
                pre_blur: 3.0,
                ..base
            },
            blank,
            blank,
        ),
        (
            "grow",
            MatteKeyParams {
                shrink_grow: 2.5,
                ..base
            },
            blank,
            blank,
        ),
        (
            "shrink",
            MatteKeyParams {
                shrink_grow: -1.75,
                ..base
            },
            blank,
            blank,
        ),
        (
            "softness",
            MatteKeyParams {
                softness: 4.0,
                ..base
            },
            blank,
            blank,
        ),
        (
            "despot_black",
            MatteKeyParams {
                despot_black: 1.0,
                ..base
            },
            blank,
            blank,
        ),
        (
            "despot_white",
            MatteKeyParams {
                despot_white: 1.0,
                ..base
            },
            blank,
            blank,
        ),
        ("inside_mask", base, inside, blank),
        ("outside_mask", base, blank, outside),
        (
            "both_masks_matte_view",
            MatteKeyParams { view: 1, ..base },
            inside,
            outside,
        ),
        (
            "everything_at_once",
            MatteKeyParams {
                pre_blur: 2.0,
                shrink_grow: -1.5,
                softness: 2.5,
                despot_black: 0.75,
                despot_white: 0.5,
                clip_black: 0.1,
                clip_white: 0.9,
                ..base
            },
            inside,
            outside,
        ),
        (
            "identity_mix0",
            MatteKeyParams {
                pre_blur: 3.0,
                softness: 3.0,
                mix: 0.0,
                ..base
            },
            inside,
            outside,
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::matte_key_spatial(&mut cpu, w, h, &p, &ins, &outs);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = to_op(&p);
        let out = fx.matte_key(&ctx, &tex, w, h, &op, &to_fill(&ins), &to_fill(&outs));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!("matte key spatial {name}: worst {worst}");
        assert!(worst < 2e-2, "{name}: worst diff {worst}");
        if name == "identity_mix0" {
            assert_eq!(gpu, img, "Mix 0 must be the identity through the pipeline");
        }

        let out2 = fx.matte_key(&ctx, &tex, w, h, &op, &to_fill(&ins), &to_fill(&outs));
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU spatial matte key must be bit-stable");
    }

    // **The default is the old picture.** Nothing spatial, no masks: both paths
    // must produce exactly what the pointwise kernel produces, bit for bit — on
    // the CPU because `matte_key_spatial` hands straight over, and on the GPU
    // because the staged pipeline is not dispatched at all.
    let mut fused = img.clone();
    lumit_core::fx::cpu::matte_key(&mut fused, &base);
    let mut staged = img.clone();
    lumit_core::fx::cpu::matte_key_spatial(&mut staged, w, h, &base, &blank, &blank);
    assert_eq!(fused, staged, "the defaults took a different CPU path");
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let bl = MaskFillOp::blank();
    let a = readback_linear_f32(
        &ctx,
        &fx.matte_key(&ctx, &tex, w, h, &to_op(&base), &bl, &bl),
        w,
        h,
    )
    .unwrap();
    let worst = fused
        .iter()
        .zip(&a)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        worst <= 1e-3,
        "the default GPU picture moved: worst {worst}"
    );
}

/// The §1.6 oracle for vignette: a cheap pointwise effect, so the CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4), and
/// Amount 0 (or Mix 0) is the bit-exact identity on both paths.
#[test]
fn wgsl_vignette_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, op) in [
        (
            "neutral",
            ExposureOp {
                stops: 0.0,
                factor: 1.0,
                mix: 1.0,
            },
        ),
        (
            "brighten",
            ExposureOp {
                stops: 0.0,
                factor: 2.0,
                mix: 1.0,
            },
        ),
        (
            "darken",
            ExposureOp {
                stops: 0.0,
                factor: 0.5,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            ExposureOp {
                stops: 0.0,
                factor: 1.7,
                mix: 0.5,
            },
        ),
        (
            "mix-zero",
            ExposureOp {
                stops: 0.0,
                factor: 3.0,
                mix: 0.0,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::exposure(&mut cpu, op.factor, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.exposure(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("exposure {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.exposure(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
            t: 0.0,
            gain_r,
            gain_b,
            mix,
        };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::temperature(&mut cpu, op.gain_r, op.gain_b, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.temperature(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("temperature {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.temperature(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.gamma(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("gamma {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.gamma(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU gamma must be bit-stable");
    }
}

/// The §1.6 oracle for hue shift: a cheap pointwise colour-matrix product,
/// so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and
/// 0° (the identity matrix) or Mix 0 is the bit-exact identity on both.
#[test]
fn wgsl_hue_shift_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let op = HueShiftOp {
            angle_rad: 0.0,
            preserve: true,
            m,
            mix,
        };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::hue_shift(&mut cpu, op.m, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.hue_shift(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("hue_shift {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if deg % 360.0 == 0.0 || mix == 0.0 {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.hue_shift(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let centre = [w as f32 * 0.5, h as f32 * 0.5];
    // The last column is the Edges policy (P3, K-145): the Transform effect
    // itself always passes 0, but Shake dispatches this same kernel with 1
    // (Repeat) and 2 (Mirror), so the oracle exercises all three here.
    // The skew column is `[amount, axis]` (K-666); `NO_SKEW` is the pre-skew
    // road, and the last two rows are the ones that lean.
    for (name, anchor, position, scale, rotation, skew, opacity, mix, edge) in [
        (
            "identity",
            [0.0; 2],
            [0.0; 2],
            [1.0; 2],
            0.0,
            lumit_core::fx::NO_SKEW,
            1.0,
            1.0,
            0u32,
        ),
        (
            "shift",
            [0.0; 2],
            [2.5, -1.5],
            [1.0; 2],
            0.0,
            lumit_core::fx::NO_SKEW,
            1.0,
            1.0,
            0,
        ),
        (
            "punch-in",
            centre,
            centre,
            [1.4, 1.4],
            12.0,
            lumit_core::fx::NO_SKEW,
            1.0,
            1.0,
            0,
        ),
        (
            "flip-fade",
            centre,
            centre,
            [-1.0, 1.0],
            0.0,
            lumit_core::fx::NO_SKEW,
            0.5,
            0.8,
            0,
        ),
        (
            "collapsed",
            centre,
            centre,
            [0.0, 1.0],
            0.0,
            lumit_core::fx::NO_SKEW,
            1.0,
            0.6,
            0,
        ),
        (
            "shift-repeat",
            [0.0; 2],
            [5.0, -4.0],
            [1.0; 2],
            0.0,
            lumit_core::fx::NO_SKEW,
            1.0,
            1.0,
            1,
        ),
        (
            "spin-mirror",
            centre,
            centre,
            [1.0; 2],
            8.0,
            lumit_core::fx::NO_SKEW,
            1.0,
            1.0,
            2,
        ),
        (
            "skew",
            centre,
            centre,
            [1.0; 2],
            0.0,
            [20.0, 0.0],
            1.0,
            1.0,
            0,
        ),
        (
            "skew-axis-and-spin",
            centre,
            centre,
            [1.2, 0.8],
            15.0,
            [-30.0, 55.0],
            1.0,
            1.0,
            2,
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::transform(
            &mut cpu, w, h, anchor, position, scale, rotation, skew, edge, opacity, mix,
        );

        let (m, off, opacity) =
            lumit_core::fx::transform_op(anchor, position, scale, rotation, skew, opacity);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = TransformOp {
            m,
            off,
            opacity,
            mix,
            edge,
        };
        let out = fx.transform(&ctx, &tex, w, h, None, &op);
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

        let out2 = fx.transform(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU transform must be bit-stable");
    }
}

/// One shake's resolved parameters, hand-built so the wobble is exactly what a
/// test asks for (K-388).
///
/// A *resolved* wobble is whatever the seeded noise says, which is no way to
/// sweep the kernel's border cases. `Shake::packed` builds each offset as
/// `amplitude · axis amount · noise`, so amplitudes of exactly 1 make the
/// unit-free noise vector *be* the wobble, and `zoom = 1 + z · noise` makes the
/// z component `zoom - 1` — an exact f32 subtraction near 1, so it round-trips.
/// The bag is the real thing either way: both the CPU reference and the op below
/// are read out of it through the effect's own typed reader.
fn shake_bag(
    wobble: lumit_core::fx::ShakeSample,
    edge: u32,
    mix: f32,
    mb: Option<[lumit_core::fx::ShakeSample; SHAKE_MB_SAMPLES]>,
) -> Vec<(lumit_core::fx::ParamId, lumit_core::fx::Value)> {
    use lumit_core::fx::effects::shake::Shake;
    use lumit_core::fx::Value;
    let noise = |s: lumit_core::fx::ShakeSample| {
        Value::Vec4([s.offset_px[0], s.offset_px[1], s.rotation_deg, s.zoom - 1.0])
    };
    let mut bag = vec![
        (Shake::AMPLITUDE, Value::Float(1.0)),
        (Shake::X_AMP, Value::Float(1.0)),
        (Shake::Y_AMP, Value::Float(1.0)),
        (Shake::ROTATION, Value::Float(1.0)),
        (Shake::MIX, Value::Float(mix * 100.0)),
        (Shake::DERIVED_Z_AMP, Value::Float(1.0)),
        (Shake::DERIVED_EDGE, Value::Choice(edge)),
        (Shake::DERIVED_NOISE, noise(wobble)),
    ];
    if let Some(samples) = mb {
        for (id, s) in Shake::DERIVED_MB_NOISE.iter().zip(samples.iter()) {
            bag.push((*id, noise(*s)));
        }
    }
    bag
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for (name, offset, rot, zoom, edge, mix) in [
        ("neutral", [0.0f32, 0.0f32], 0.0f32, 1.0f32, 1u32, 1.0f32),
        ("offset", [2.5, -1.5], 0.0, 1.0, 0, 1.0),
        ("twist-repeat", [1.0, 0.5], 4.0, 1.0, 1, 1.0),
        ("pumped-mirror", [0.0, 2.0], -2.0, 0.95, 2, 0.7),
    ] {
        use lumit_core::fx::effects::shake::{Shake, ShakeDef, Shaken};
        use lumit_core::fx::{EffectDef, EffectMetadata, Params};
        let bag = shake_bag(
            lumit_core::fx::ShakeSample {
                offset_px: offset,
                rotation_deg: rot,
                zoom,
            },
            edge,
            mix,
            None,
        );
        let p = Params::new(&bag);
        let mut cpu = img.clone();
        ShakeDef.apply_cpu(&mut cpu, w, h, p);

        // The exact `gpufx` mapping: the bag → `packed` → the shared affine →
        // transform op → the Transform kernel, carrying the Edges policy. The
        // wobble is read back out of the bag rather than reused from the case
        // above, so the reassembly K-388 pins is under test too.
        let Shaken::Plain {
            wobble,
            edge: packed_edge,
            mix: packed_mix,
        } = Shake::read(p).packed(Shake::derived_of(p))
        else {
            panic!("no sub-frames in the bag: this is the plain shake");
        };
        assert_eq!(
            (wobble.offset_px, wobble.rotation_deg, wobble.zoom),
            (offset, rot, zoom),
            "the noise vectors reassemble the wobble exactly"
        );
        assert_eq!((packed_edge, packed_mix), (edge, mix));
        let (anchor, position, scale, rotation) =
            lumit_core::fx::shake_affine(w, h, wobble.offset_px, wobble.rotation_deg, wobble.zoom);
        let (m, off, opacity) = lumit_core::fx::transform_op(
            anchor,
            position,
            scale,
            rotation,
            lumit_core::fx::NO_SKEW,
            1.0,
        );
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let op = TransformOp {
            m,
            off,
            opacity,
            mix,
            edge,
        };
        let out = fx.transform(&ctx, &tex, w, h, None, &op);
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

        let out2 = fx.transform(&ctx, &tex, w, h, None, &op);
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

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        rot_freq: 1.6,
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
        use lumit_core::fx::effects::shake::{Shake, ShakeDef, Shaken};
        use lumit_core::fx::{EffectDef, EffectMetadata, Params};
        let bag = shake_bag(centre, edge, mix, Some(samples));
        let p = Params::new(&bag);
        let mut cpu = img.clone();
        ShakeDef.apply_cpu(&mut cpu, w, h, p);

        // The exact `gpufx` mapping: the bag → `packed` → each sub-frame's
        // shared affine → transform op → one tap of the averaging kernel.
        let Shaken::Blurred {
            samples: packed_samples,
            ..
        } = Shake::read(p).packed(Shake::derived_of(p))
        else {
            panic!("the bag carries sub-frames, so this is the smeared shake");
        };
        assert_eq!(
            packed_samples, samples,
            "the noise vectors reassemble every sub-frame exactly"
        );
        let mut taps = [ShakeMbTap {
            m: [1.0, 0.0, 0.0, 1.0],
            off: [0.0, 0.0],
        }; SHAKE_MB_SAMPLES];
        for (t, s) in taps.iter_mut().zip(packed_samples.iter()) {
            let (anchor, position, scale, rotation) =
                lumit_core::fx::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
            let (m, off, _opacity) = lumit_core::fx::transform_op(
                anchor,
                position,
                scale,
                rotation,
                lumit_core::fx::NO_SKEW,
                1.0,
            );
            *t = ShakeMbTap { m, off };
        }
        let op = ShakeMbOp {
            taps,
            count: SHAKE_MB_SAMPLES as u32,
            edge,
            mix,
        };
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.shake_mb(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("shake-mb {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        assert_ne!(gpu, img, "{name}: the motion blur moves pixels");

        let out2 = fx.shake_mb(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU shake motion blur must be bit-stable");
    }

    // A single tap equal to the frame wobble is the plain Shake: the averaging
    // kernel at count 1 matches the Transform kernel within the cheap bound.
    let (anchor, position, scale, rotation) =
        lumit_core::fx::shake_affine(w, h, centre.offset_px, centre.rotation_deg, centre.zoom);
    let (m, off, opacity) = lumit_core::fx::transform_op(
        anchor,
        position,
        scale,
        rotation,
        lumit_core::fx::NO_SKEW,
        1.0,
    );
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
        None,
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
        None,
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
            &mut cpu,
            w,
            h,
            radius,
            threshold,
            knee,
            intensity,
            tint,
            mix,
            &[],
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
        let out = fx.glow(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_diff(&cpu, &gpu);
        // Logged so real cross-vendor deltas accumulate (docs/08 open
        // question 5: the class tolerances are placeholders until then).
        eprintln!("glow {name}: worst {worst:.2e}");
        assert!(worst < 5e-3, "{name}: worst diff {worst}");
        if name == "neutral" {
            assert_eq!(gpu, img, "intensity 0 must be the bit-exact identity");
        }

        let out2 = fx.glow(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.block_glitch(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("block_glitch {}: worst {worst} ulp", case.name);
        assert!(worst <= 2, "{}: worst {worst} fp16 ULP", case.name);
        if case.name == "neutral-intensity0" {
            assert_eq!(gpu, img, "{}: must be the bit-exact passthrough", case.name);
        }

        let out2 = fx.block_glitch(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.scanlines(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("scanlines {}: worst {worst} ulp", case.name);
        assert!(worst <= 2, "{}: worst {worst} fp16 ULP", case.name);
        if case.name == "neutral-intensity0" {
            assert_eq!(gpu, img, "{}: must be the bit-exact passthrough", case.name);
        }

        let out2 = fx.scanlines(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
            let out = fx.dir_blur(&ctx, &tex, w, h, None, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("dir blur e={edge} l={length} a={angle}: worst {worst} ulp");
            assert!(
                worst <= 2,
                "edge {edge} length {length} angle {angle} mix {mix}: \
                     worst {worst} fp16 ULP"
            );

            let out2 = fx.dir_blur(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for edge in [0u32, 1, 2] {
        // Centres are raster pixels since K-558 (px@comp, resolved to this
        // raster): the middle of the 32x24 corpus, and a point three tenths
        // across and seven tenths down it.
        for (centre, amount, spin, mix) in [
            ([16.0f32, 12.0f32], 6.0f32, true, 1.0f32),
            ([16.0, 12.0], 6.0, false, 1.0),
            ([9.6, 16.8], 9.5, true, 0.6),
            ([9.6, 16.8], 9.5, false, 0.6),
            ([16.0, 12.0], 0.0, true, 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::blur_radial(&mut cpu, w, h, centre, amount, spin, edge, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = RadialBlurOp {
                centre_px: centre,
                amount_px: amount,
                taps: lumit_core::fx::cpu::radial_blur_taps(amount),
                spin,
                edge,
                mix,
            };
            let out = fx.radial_blur(&ctx, &tex, w, h, None, &op);
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

            let out2 = fx.radial_blur(&ctx, &tex, w, h, None, &op);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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

/// The §1.6 oracle for the generic Matte dissolve (K-395, docs/08 §2.6):
/// `fx_matte_mix.wgsl` against `lumit_core::fx::cpu::matte_mix`, on the corpus,
/// both ways round the Invert switch.
///
/// The three things this pins, because each of them is a picture that renders
/// and is wrong rather than a failure anyone would notice:
///
/// - **The weights and the order.** Rec. 709 luma of the *premultiplied* matte,
///   clamped, then inverted. Unpremultiplying, or inverting before the clamp,
///   both look plausible and both drive the effect differently everywhere the
///   matte is dark or transparent.
/// - **The end stops are bit-exact.** A white matte must be the effect's output
///   untouched and a black one its input untouched — the whole claim that this
///   pass costs nothing where nobody asked for it.
/// - **Bit stability** (§2.4).
#[test]
fn wgsl_matte_mix_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let input = corpus(w, h);
    // A visibly different "effected" copy: any distinct image serves.
    let processed: Vec<f32> = input
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
    // A matte that ramps across the frame, with a dark and a bright band so
    // the clamp is exercised at both ends and the HDR spike drives past 1.
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = x as f32 / (w - 1) as f32;
            matte[i] = f16_to_f32(f16_bits(g * 1.4));
            matte[i + 1] = f16_to_f32(f16_bits(g));
            matte[i + 2] = f16_to_f32(f16_bits(1.0 - g));
            matte[i + 3] = 1.0;
        }
    }
    let ti = upload_linear_f32(&ctx, &input, w, h);
    let tp = upload_linear_f32(&ctx, &processed, w, h);
    let tm = upload_linear_f32(&ctx, &matte, w, h);
    for invert in [false, true] {
        let out = fx.matte_mix(&ctx, &ti, &tp, &tm, w, h, invert);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let mut want = processed.clone();
        lumit_core::fx::cpu::matte_mix(&mut want, &input, &matte, invert);
        let want: Vec<f32> = want.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
        let worst = worst_f16_ulp(&gpu, &want);
        eprintln!("matte mix invert={invert}: worst {worst} ulp");
        assert!(worst <= 1, "invert {invert}: worst {worst} fp16 ULP");

        let out2 = fx.matte_mix(&ctx, &ti, &tp, &tm, w, h, invert);
        assert_eq!(
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            gpu,
            "the matte dissolve must be bit-stable"
        );
    }

    // The end stops, both ways round: a white matte is the effect in full, a
    // black one is the untouched input, and Invert swaps exactly those two.
    let white: Vec<f32> = vec![1.0; (w * h * 4) as usize];
    let black: Vec<f32> = (0..(w * h * 4))
        .map(|i| if i % 4 == 3 { 1.0 } else { 0.0 })
        .collect();
    let tw = upload_linear_f32(&ctx, &white, w, h);
    let tb = upload_linear_f32(&ctx, &black, w, h);
    for (name, tex, invert, want) in [
        ("white", &tw, false, &processed),
        ("black", &tb, false, &input),
        ("white inverted", &tw, true, &input),
        ("black inverted", &tb, true, &processed),
    ] {
        let out = fx.matte_mix(&ctx, &ti, &tp, tex, w, h, invert);
        assert_eq!(
            &readback_linear_f32(&ctx, &out, w, h).unwrap(),
            want,
            "a {name} matte must pass its end of the dissolve through bit-exactly"
        );
    }
}

/// The §1.6 oracle for the seam's matte preparation (K-425, docs/08 §2.6):
/// `fx_matte_prepare.wgsl` against `lumit_core::fx::cpu::matte_prepare`, on
/// every channel, both ways round Invert. The pass is pointwise arithmetic —
/// a channel pick, a clamp, a subtraction — so the bound is the fp16 store
/// alone, and the GPU must be bit-stable (§2.4).
///
/// The picture claim beside it: the prepared matte is grey (R = G = B) with
/// alpha 1, so every kernel's luma read returns the chosen channel — which is
/// the whole mechanism by which a kernel that only knows luma gains a Channel
/// row without learning anything.
#[test]
fn wgsl_matte_prepare_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (40u32, 24u32);
    // A matte with four distinct channels, partial alpha and an HDR band.
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let g = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            matte[i] = f16_to_f32(f16_bits(g * 1.5));
            matte[i + 1] = f16_to_f32(f16_bits(1.0 - g));
            matte[i + 2] = f16_to_f32(f16_bits(v));
            matte[i + 3] = f16_to_f32(f16_bits(0.25 + 0.75 * v));
        }
    }
    let tm = upload_linear_f32(&ctx, &matte, w, h);
    for channel in 0..5u32 {
        for invert in [false, true] {
            let out = fx.matte_prepare(&ctx, &tm, w, h, channel, invert);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let mut want = matte.clone();
            lumit_core::fx::cpu::matte_prepare(&mut want, channel, invert);
            let want: Vec<f32> = want.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
            let worst = worst_f16_ulp(&gpu, &want);
            assert!(
                worst <= 1,
                "channel {channel} invert {invert}: worst {worst} fp16 ULP"
            );
            for px in gpu.chunks_exact(4) {
                assert_eq!(px[0], px[1], "grey: R = G");
                assert_eq!(px[1], px[2], "grey: G = B");
                assert_eq!(px[3], 1.0, "alpha 1");
                assert!((0.0..=1.0).contains(&px[0]), "clamped");
            }
            let again = fx.matte_prepare(&ctx, &tm, w, h, channel, invert);
            assert_eq!(
                readback_linear_f32(&ctx, &again, w, h).unwrap(),
                gpu,
                "the prepare pass must be bit-stable"
            );
        }
    }
}

/// **Invert is applied exactly once** (K-425). The three kernels that used to
/// invert their own matte — Gaussian blur, Glow, Turbulent displace — now read
/// it as it arrives, so a matte the seam has inverted drives them the other
/// way round, and a matte it has not is read straight. The proof is the
/// Gaussian blur's width probe under a FLAT matte: prepared with Invert from
/// black, it reads as white and the blur runs at full radius; handed the
/// prepared (white) matte, the kernel must not flip it back to black, which
/// would leave the picture sharp. A second invert would cancel the first, and
/// the dot would not spread at all.
#[test]
fn the_seam_inverts_the_matte_once_and_the_kernel_not_again() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 16u32);
    let mut dot = vec![0.0f32; (w * h * 4) as usize];
    let c = ((8 * w + 24) * 4) as usize;
    dot[c..c + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
    let tex = upload_linear_f32(&ctx, &dot, w, h);
    let black: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [0.0, 0.0, 0.0, 1.0])
        .collect();
    let black_tex = upload_linear_f32(&ctx, &black, w, h);
    let op = BlurOp {
        radius_px: 10.0,
        edge: 1,
        mix: 1.0,
    };
    let spread = |m: &wgpu::Texture| -> f32 {
        let out = fx.blur(&ctx, &tex, w, h, Some(m), &op);
        let got = readback_linear_f32(&ctx, &out, w, h).unwrap();
        got[c]
    };
    // A black matte: no blur, the dot keeps its full value.
    assert_eq!(spread(&black_tex), 1.0, "black matte: the dot is untouched");
    // The same matte inverted once, by the seam: a white matte, a full blur,
    // the dot's light spread thin.
    let inverted = fx.matte_prepare(&ctx, &black_tex, w, h, 0, true);
    let centre = spread(&inverted);
    assert!(
        centre < 0.2,
        "the seam's invert makes the matte white and the blur runs: centre {centre}"
    );
    // And preparing the *inverted* matte without Invert changes nothing: the
    // kernel reads luma of a grey, which is the grey.
    let again = fx.matte_prepare(&ctx, &inverted, w, h, 0, false);
    assert_eq!(
        spread(&again),
        centre,
        "a second, non-inverting prepare is a no-op"
    );
}

/// The §1.6 oracle for the seam's Blend and Mix (K-425, docs/08 §1.5):
/// `fx_blend_mix.wgsl` against `lumit_core::fx::cpu::blend_mix`, across every
/// layer blend mode on a small picture, at full and partial Mix.
///
/// The linear modes are a handful of arithmetic ops and hold to 1 fp16 ULP.
/// The encoded set passes through `pow` twice, whose CPU and GPU
/// implementations differ by a few f32 ULP before the fp16 store, so they are
/// held to 2; Hard mix is a step function on a derived value and a pixel that
/// lands on the threshold may fall either side of it, so it is held on the
/// fraction of pixels that agree rather than the worst one. Every mode must be
/// bit-stable (§2.4), and Mix 0 must be the input bit-exactly on every mode.
#[test]
fn wgsl_blend_mix_matches_the_cpu_oracle_on_every_mode() {
    use lumit_core::model::BlendMode;
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (24u32, 16u32);
    let input = corpus(w, h);
    // A distinct "effected" picture: the corpus shifted and recoloured.
    let processed: Vec<f32> = input
        .iter()
        .enumerate()
        .map(|(i, v)| match i % 4 {
            3 => *v,
            0 => f16_to_f32(f16_bits(1.0 - v * 0.6)),
            1 => f16_to_f32(f16_bits((v * 1.3).min(5.0))),
            _ => f16_to_f32(f16_bits(v * 0.4 + 0.1)),
        })
        .collect();
    let ti = upload_linear_f32(&ctx, &input, w, h);
    let tp = upload_linear_f32(&ctx, &processed, w, h);
    for (mode, name) in BlendMode::NAMES.iter().enumerate().skip(1) {
        let mode = mode as u32;
        for mix in [1.0f32, 0.35, 0.0] {
            let out = fx.blend_mix(&ctx, &ti, &tp, w, h, mode, mix);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let mut want = processed.clone();
            lumit_core::fx::cpu::blend_mix(&mut want, &input, mode, mix);
            let want: Vec<f32> = want.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
            if *name == "Hard mix" {
                let agree = gpu
                    .iter()
                    .zip(&want)
                    .filter(|(a, b)| (*a - *b).abs() <= 1e-3)
                    .count();
                assert!(
                    agree * 100 >= gpu.len() * 98,
                    "{name} mix {mix}: {agree} of {} values agree",
                    gpu.len()
                );
            } else {
                let worst = worst_f16_ulp(&gpu, &want);
                let bound = if matches!(mode, 1 | 2 | 6 | 7 | 20) {
                    1
                } else {
                    2
                };
                assert!(worst <= bound, "{name} mix {mix}: worst {worst} fp16 ULP");
            }
            if mix == 0.0 {
                assert_eq!(gpu, input, "{name}: Mix 0 is the input bit-exactly");
            }
            let again = fx.blend_mix(&ctx, &ti, &tp, w, h, mode, mix);
            assert_eq!(
                readback_linear_f32(&ctx, &again, w, h).unwrap(),
                gpu,
                "{name}: the blend pass must be bit-stable"
            );
        }
    }
    // Normal at Mix 1 through the pass is the effect's output: the seam never
    // dispatches it, but the kernel's own table says so too.
    let out = fx.blend_mix(&ctx, &ti, &tp, w, h, 0, 1.0);
    assert_eq!(readback_linear_f32(&ctx, &out, w, h).unwrap(), processed);
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        let out = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("echo mode={mode} mix={mix}: worst {worst} ulp");
        assert!(
            worst <= bound,
            "mode {mode} mix {mix}: worst {worst} fp16 ULP (bound {bound})"
        );
        let out2 = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, None, &op);
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
        let out = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, None, &op);
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
        None,
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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

    // The tile side is duplicated in this crate (it may not depend on
    // lumit-core outside tests); if the two ever drift the kernel and the
    // oracle would tile the frame differently and every case below would fail
    // for an obscure reason. Say so plainly instead.
    assert_eq!(
        crate::fx::MB_TILE,
        lumit_core::fx::cpu::MB_TILE,
        "the tile side must match the oracle's"
    );

    use lumit_core::fx::MbQuality;
    let cases = [
        (
            &constant,
            &full,
            0.5f32,
            16i32,
            1.0f32,
            MbQuality::Normal,
            "constant",
        ),
        (&varying, &full, 1.0, 12, 0.7, MbQuality::Normal, "varying"),
        (&constant, &full, 0.25, 8, 1.0, MbQuality::Normal, "short"),
        (
            &varying,
            &conf_vary,
            1.0,
            12,
            1.0,
            MbQuality::Normal,
            "confidence-blended",
        ),
        // High: curved trails and half the tap spacing, so the whole
        // re-sample-along-the-streak branch is held to the oracle too.
        (
            &varying,
            &conf_vary,
            1.0,
            32,
            1.0,
            MbQuality::High,
            "high, curved",
        ),
        (
            &constant,
            &full,
            0.5,
            32,
            1.0,
            MbQuality::High,
            "high, constant",
        ),
    ];
    for (field, conf, shutter_frac, samples, mix, quality, name) in cases {
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
            quality,
        );
        let flow_t = upload_flow_field(&ctx, u, v, conf, w, h);
        let op = MotionBlurOp {
            shutter_frac,
            samples,
            mix,
            view: MbView::Rendered.code(),
            quality: quality.code(),
            vector_scale: 0.0,
        };
        let out = fx.motion_blur(&ctx, &src, &flow_t, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("motion blur {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        let out2 = fx.motion_blur(&ctx, &src, &flow_t, w, h, None, &op);
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
    for view in [MbView::MotionVectors, MbView::Confidence, MbView::TileMax] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::motion_blur(
            &mut cpu,
            w,
            h,
            u,
            v,
            &conf_vary,
            0.5,
            16,
            1.0,
            view,
            MbQuality::Normal,
        );
        let op = MotionBlurOp {
            shutter_frac: 0.5,
            samples: 16,
            mix: 1.0,
            view: view.code(),
            quality: MbQuality::Normal.code(),
            vector_scale: 0.0,
        };
        let out = fx.motion_blur(&ctx, &src, &flow_t, w, h, None, &op);
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
        None,
        &MotionBlurOp {
            shutter_frac: 0.5,
            samples: 16,
            mix: 1.0,
            view: MbView::Rendered.code(),
            quality: MbQuality::Normal.code(),
            vector_scale: 0.0,
        },
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "a still tile must be a bit-exact passthrough"
    );
    let moving = upload_flow_field(&ctx, &constant.0, &constant.1, &full, w, h);
    let out = fx.motion_blur(
        &ctx,
        &src,
        &moving,
        w,
        h,
        None,
        &MotionBlurOp {
            shutter_frac: 0.0,
            samples: 16,
            mix: 1.0,
            view: MbView::Rendered.code(),
            quality: MbQuality::Normal.code(),
            vector_scale: 0.0,
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
/// WGSL manual-trilinear lookup matches `lumit_core::lut::Lut3d::sample_in`
/// wrapped as unpremultiply -> sample -> re-premultiply -> Mix, on a spread
/// of RGBA pixels **including partial-alpha and out-of-domain HDR ones** and
/// several cubes (identity, a per-channel gamma, an R/B swap). A cheap
/// pointwise effect, so CPU and GPU agree to ≤ 2 fp16 ULP; the GPU is
/// bit-stable (§2.4); Mix 0 is the bit-exact input; every **Input space**
/// (K-543) is covered, and Linear is the case list's own default so a
/// transfer leaking into it would fail the same comparison; and the identity cube
/// round-trips every in-domain pixel to itself (a strong end-to-end check
/// that the red-fastest indexing, the domain scale and the premult handling
/// are all right — if it did not, one of those three is wrong).
#[test]
fn wgsl_lut_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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

    use lumit_core::lut::LutSpace;
    let cases: [(&str, &lumit_core::lut::Lut3d, f32, LutSpace); 14] = [
        ("identity-full", &identity, 1.0, LutSpace::Linear),
        ("identity-mix0", &identity, 0.0, LutSpace::Linear),
        ("gamma-full", &gamma, 1.0, LutSpace::Linear),
        ("gamma-mixed", &gamma, 0.5, LutSpace::Linear),
        ("swap-rb", &swap, 1.0, LutSpace::Linear),
        ("domained-full", &domained, 1.0, LutSpace::Linear),
        ("domained-mixed", &domained, 0.5, LutSpace::Linear),
        ("zero-span-domain", &zero_span, 1.0, LutSpace::Linear),
        // Input space (K-543): the picture converts into the space the cube was
        // authored for, the table applies, the result converts back. Every case
        // runs against the same pixels as its Linear sibling, so a shader that
        // dropped or mis-ordered a transfer misses the oracle rather than
        // producing a plausible-looking different grade.
        ("srgb-identity", &identity, 1.0, LutSpace::Srgb),
        ("srgb-gamma", &gamma, 1.0, LutSpace::Srgb),
        ("srgb-mixed", &gamma, 0.5, LutSpace::Srgb),
        ("srgb-domained", &domained, 1.0, LutSpace::Srgb),
        ("rec709-identity", &identity, 1.0, LutSpace::Rec709),
        ("rec709-swap", &swap, 1.0, LutSpace::Rec709),
    ];

    let mut rendered: Vec<(&str, Vec<f32>)> = Vec::new();
    for (name, lut, mix, space) in cases {
        // CPU expected: unpremultiply -> Lut3d::sample -> re-premultiply ->
        // Mix, using the same lerp form the shader uses for the final blend.
        let mut cpu = vec![0.0f32; img.len()];
        for px in 0..(w * h) as usize {
            let i = px * 4;
            let o = [img[i], img[i + 1], img[i + 2], img[i + 3]];
            let graded = lut.sample_in(space, unpremult(o));
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
            space.code(),
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
            space.code(),
            lut.domain_min,
            lut.domain_max,
        );
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "{name}: GPU LUT must be bit-stable");
        rendered.push((name, gpu));
    }

    // The oracle above would also be satisfied by a shader that ignored the
    // Input space entirely, because the reference would then be wrong in the
    // same way. So: each space must render a *different* picture from Linear
    // through the same cube, and Linear must still be what it always was.
    let of = |n: &str| -> &Vec<f32> {
        &rendered
            .iter()
            .find(|(k, _)| *k == n)
            .expect("case rendered")
            .1
    };
    assert_ne!(
        of("srgb-gamma"),
        of("gamma-full"),
        "sRGB input space must change the grade"
    );
    assert_ne!(
        of("rec709-swap"),
        of("swap-rb"),
        "Rec. 709 input space must change the grade"
    );
    assert_ne!(
        of("srgb-gamma"),
        of("rec709-identity"),
        "the two spaces are not the same curve"
    );

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
            LutSpace::Linear.code(),
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
            // Both views answer to Gamma, and answer to it the same way
            // (K-615) — so the twin has to rescale the depth axis where the
            // oracle does.
            "depth map squeezed",
            lumit_core::fx::cpu::DofParams {
                display: 1,
                gamma: 4.0,
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
        // Every element left as the lens file describes it (K-371).
        coating_elements: [lumit_core::fx::lens_flare::COATING_AS_FILE;
            lumit_core::fx::lens_flare::MAX_COATING_ELEMENTS],
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
        // px@comp since K-558, and half a pixel rounds to the no-blur this
        // fixture has always rendered.
        ghost_softness: 0.5,
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
        matte_invert: false,
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
        // One entry per light, extent and all, exactly as `fxops` builds it
        // for the production path (K-367).
        manual_lights: lf::manual_light(p, w, h)
            .iter()
            .map(|l| {
                [
                    l.pos[0],
                    l.pos[1],
                    l.rgb[0],
                    l.rgb[1],
                    l.rgb[2],
                    l.extent[0],
                    l.extent[1],
                ]
            })
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
        matte_invert: p.matte_invert,
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
        sb_fields: lf::STARBURST_FIELDS as u32,
    }
}

/// **The fixed-point accumulator's scale is spelled twice, and its ceiling is
/// far above any frame** (K-375).
///
/// The deposit sums in fixed point because integer addition is associative and
/// a float scatter is not — the same document has to render the same bytes
/// (K-353). That buys determinism at the cost of a ceiling: above
/// `ACCUM_CEILING` a channel's u32 wraps, and it wraps rather than saturates,
/// because detecting the overflow would need the compare-and-swap whose order
/// dependence this design exists to avoid. So the margin is the safety, and
/// this measures it on the CPU reference rather than asserting it.
#[test]
fn lens_flare_accumulator_scale_matches_the_shader_and_clears_any_real_frame() {
    use lumit_core::fx::lens_flare as lf;
    let src = include_str!("../fx_lens_flare_deposit.wgsl");
    assert!(
        src.contains(&format!("const ACCUM_SCALE: f32 = {:.1};", ACCUM_SCALE)),
        "the deposit shader must declare the same scale as the Rust twin"
    );
    assert!(
        ((u32::MAX as f32 / ACCUM_SCALE) - ACCUM_CEILING).abs() < 0.01,
        "the ceiling is the scale's own consequence"
    );
    // The K-380 pyramid's two spellings: the level-changeover span and the
    // uniform's level-table capacity.
    assert!(
        src.contains(&format!(
            "const DEPOSIT_SPAN_PX: f32 = {:.1};",
            lf::DEPOSIT_SPAN_PX
        )),
        "the deposit shader must declare lumit-core's DEPOSIT_SPAN_PX"
    );
    assert_eq!(
        crate::fx::lens_flare::MAX_DEPOSIT_LEVELS,
        lf::MAX_DEPOSIT_LEVELS,
        "the two crates' level caps must agree"
    );

    // The brightest pixel a bundled lens actually makes, at the settings that
    // make it brightest: full intensity, no ghost blur to spread it, and the
    // light in frame. If this ever approached the ceiling the scale would have
    // to come down — and the test would say so before a user saw a wrapped
    // highlight.
    let p = lf::LensFlareParams {
        ghost_softness: 0.0,
        intensity: 4.0,
        ghost_intensity: 4.0,
        ..flare_params()
    };
    let baked = lf::bake(&p);
    let (w, h) = (192u32, 108u32);
    let flare = lf::cpu_flare(&p, &baked, w, h, &lf::manual_light(&p, w, h));
    let peak = flare.iter().cloned().fold(0.0_f32, f32::max);
    assert!(peak > 0.0, "the reference must render something to measure");
    assert!(
        peak < ACCUM_CEILING / 100.0,
        "the accumulator's ceiling ({ACCUM_CEILING}) must stay orders above a \
         real frame's brightest pixel ({peak}); at less than 100x the margin \
         the fixed-point scale wants revisiting"
    );
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
    // The two splat constants (K-366) are spelled twice for the same reason:
    // a drift in the anti-alias floor or the density cap changes what every
    // ray deposits, so the GPU would stop being the CPU reference's twin in
    // a way only the fold tolerances would show.
    for (name, want) in [
        ("MIN_SPLAT_AXIS_PX", lf::MIN_SPLAT_AXIS_PX),
        ("MIN_AREA_FRAC", lf::MIN_AREA_FRAC),
    ] {
        let want = format!("const {name}: f32 = {want};");
        assert!(
            src.contains(&want),
            "the trace shader must declare `{want}`"
        );
    }
    // The source-integration irrationals (K-367). These decide WHERE in a
    // source each ray takes its light from, so a drift of one bit would give
    // the GPU a different source integral from the CPU reference on every
    // area light — parsed and compared as bits rather than as text, because
    // the two languages print floats differently.
    for (name, want) in [
        ("PHI_U", lf::PHI_U),
        ("PHI_V", lf::PHI_V),
        ("PHI_BAND", lf::PHI_BAND),
    ] {
        let decl = format!("const {name}: f32 = ");
        let at = src
            .find(&decl)
            .unwrap_or_else(|| panic!("the trace shader must declare `{decl}…`"));
        let tail = &src[at + decl.len()..];
        let end = tail.find(';').expect("a terminated constant declaration");
        let got: f32 = tail[..end]
            .trim()
            .parse()
            .expect("a parseable float literal");
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "the trace shader's {name} ({got}) must be lumit-core's ({want})"
        );
    }
}

/// The starburst atlas's slice count is spelled twice as well (K-365) —
/// lumit-core bakes the slices and stacks them, the combine shader divides
/// the atlas height by its own constant to find a slice's rows. A drift
/// would read every light's starburst from the wrong part of the atlas, in
/// smooth-looking wrong colours nobody would trace back to a constant.
#[test]
fn lens_flare_wgsl_starburst_fields_match_lumit_core() {
    use lumit_core::fx::lens_flare as lf;
    let src = include_str!("../fx_lens_flare_combine.wgsl");
    let want = format!("const STARBURST_FIELDS: u32 = {}u;", lf::STARBURST_FIELDS);
    assert!(
        src.contains(&want),
        "the combine shader must declare `{want}`"
    );
    // The starburst stamp grid (K-367): a drift would smear an area source's
    // spike over a different span on the GPU than on the CPU reference, which
    // only the combine oracle's mean would notice and only faintly.
    for want in [
        format!("const SB_MIN_EXTENT: f32 = {};", lf::SB_MIN_EXTENT),
        format!("const SB_STAMPS: u32 = {}u;", lf::SB_STAMPS),
    ] {
        assert!(
            src.contains(&want),
            "the combine shader must declare `{want}`"
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
    // The K-380 deposit pyramid's level table is mirrored the same way: a
    // drift would put the two twins' levels at different offsets and every
    // coarse splat in the wrong place.
    for (w, h) in [
        (1u32, 1u32),
        (33, 20),
        (192, 108),
        (960, 540),
        (1920, 1080),
        (3840, 2160),
        (8192, 4320),
    ] {
        assert_eq!(
            crate::fx::lens_flare::deposit_levels_of(w, h),
            lumit_core::fx::lens_flare::deposit_levels(w, h),
            "w {w} h {h}"
        );
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
        combo_deposit_cost, plan_batches, RAY_BYTES, SCRATCH_BYTE_BUDGET, SPLAT_BYTES,
        STEPS_PER_SUBMIT,
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
            // A mix of compact and frame-filling ghosts on a padded 1080p
            // buffer, so the K-379 deposit cap is exercised alongside the
            // scratch budget; the coverage invariants must hold under both.
            let costs: Vec<u64> = (0..table.len())
                .map(|i| combo_deposit_cost(if i % 3 == 0 { 1.5 } else { 0.1 }, 2203.0))
                .collect();
            let plan = plan_batches(table, lights, &costs);
            // Every (combo, light) appears exactly once.
            let mut seen = vec![0u32; table.len() * lights as usize];
            for b in &plan {
                assert_eq!(
                    b.grid, table[b.combo_offset as usize],
                    "a batch must dispatch at its combos' own grid"
                );
                let rays = u64::from(b.grid) * u64::from(b.grid);
                let slots = u64::from(b.lights) * u64::from(b.combos);
                assert_eq!(b.ray_bytes, slots * rays * RAY_BYTES);
                // One splat per RAY since K-366, not one per quad.
                assert_eq!(b.splat_bytes, slots * rays * SPLAT_BYTES);
                assert!(
                    b.ray_bytes + b.splat_bytes <= SCRATCH_BYTE_BUDGET,
                    "batch at grid {} × {} combos × {} lights wants {} bytes",
                    b.grid,
                    b.combos,
                    b.lights,
                    b.ray_bytes + b.splat_bytes
                );
                // The K-379 bound: no batch asks for more than about one
                // submission of deposit work, down to the one-slot floor.
                assert!(
                    b.deposit_px(&costs) <= STEPS_PER_SUBMIT || (b.combos == 1 && b.lights == 1),
                    "batch at grid {} × {} combos × {} lights deposits {} px",
                    b.grid,
                    b.combos,
                    b.lights,
                    b.deposit_px(&costs)
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    use lumit_core::fx::lens_flare as lf;
    // Big enough that the flare buffer is many tiles wide and the blur radius
    // is a real one; few enough ghosts that it stays a quick test.
    let (w, h) = (768u32, 432u32);
    let p = lf::LensFlareParams {
        light: [380.0, 130.0],
        // px@comp since K-558: the old 2 % of this frame's diagonal, which is
        // the radius in the tens of pixels the tile cache is here to prove.
        ghost_softness: 18.0,
        max_ghosts: 3,
        ..flare_params()
    };
    let (_, _, div) = lf::quality_ladder(p.quality);
    let (fw, fh) = ((w / div).max(1), (h / div).max(1));
    let radius = lf::ghost_blur_radius(p.ghost_softness, div);
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
    use crate::fx::lens_flare::{combo_deposit_cost, plan_batches, plan_flushes, STEPS_PER_SUBMIT};
    let surfaces = 20u32;
    // A working-tier frame: Normal's base grid across a default ghost count,
    // compact ghosts (5% of the diagonal), so the trace dominates the cost.
    let compact = vec![combo_deposit_cost(0.05, 2203.0); 480];
    let heavy = plan_batches(&vec![64u32; 480], 1, &compact);
    let flushes = plan_flushes(&heavy, surfaces, &compact);
    assert!(
        flushes.iter().filter(|f| **f).count() >= 2,
        "a default Normal frame must not be one giant submission"
    );
    // No submission holds more work than the budget plus the one batch that
    // crossed it — the bound the watchdog guard rests on.
    let biggest_batch = heavy
        .iter()
        .map(|b| b.steps(surfaces) + b.deposit_px(&compact))
        .max()
        .unwrap_or_default();
    let mut run = 0u64;
    for (b, flush) in heavy.iter().zip(&flushes) {
        run += b.steps(surfaces) + b.deposit_px(&compact);
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
    let light_costs = vec![combo_deposit_cost(0.05, 550.0); 24];
    let light = plan_batches(&[32u32; 24], 1, &light_costs);
    assert!(
        plan_flushes(&light, surfaces, &light_costs)
            .iter()
            .all(|f| !f),
        "a light frame should submit once"
    );

    // The K-379 case the trace steps cannot see: a few combos of coarse
    // grid whose ghosts FILL a 1080p flare buffer. The trace is a rounding
    // error — 24 combos × 32² rays — but the deposit is nine times the
    // frame per combo, seconds of atomic scatter, and it was exactly this
    // shape of frame that froze the owner's machine and took the device
    // with it. It must flush, repeatedly.
    let full_frame = vec![combo_deposit_cost(1.5, 2203.0); 24];
    let defocused = plan_batches(&[32u32; 24], 1, &full_frame);
    let flushes = plan_flushes(&defocused, surfaces, &full_frame);
    assert!(
        flushes.iter().filter(|f| **f).count() >= 2,
        "a frame of frame-filling ghosts must split by its deposit cost"
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    assert_eq!(
        fx.flare_substitutions(),
        0,
        "an exact frame drew the lens it names"
    );

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
    // And says which frame it was (K-431): this one drew the first lens under
    // the second one's name, so it is the one frame nobody may bank.
    assert_eq!(
        fx.flare_substitutions(),
        1,
        "one frame stood the previous lens in"
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
    let stood_in = fx.flare_substitutions();
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
    assert_eq!(
        fx.flare_substitutions(),
        stood_in,
        "and stands nothing in, so the frame is one to keep (K-431)"
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    use lumit_core::fx::lens_flare as lf;
    let (w, h) = (192u32, 108u32);
    // Lens 17 (the Zeiss Biotar) carries the FOUR-BOUNCE phases (K-368):
    // its two-bounce family runs out at rank 45, so a deep enough combo
    // window reaches paths the extra two phases actually walk. The other
    // two lenses rank no four-bounce path anywhere near the top and check
    // the two-bounce walk at the shallow window they always did.
    for (lens, max_ghosts, combo_limit) in [(16u32, 10u32, 12u32), (5, 10, 12), (17, 60, 180)] {
        for light_frac in [[0.33f32, 0.30f32], [0.85, 0.75]] {
            let p = lf::LensFlareParams {
                lens,
                max_ghosts,
                light: [light_frac[0] * w as f32, light_frac[1] * h as f32],
                ..flare_params()
            };
            let baked = lf::bake(&p);
            let op = flare_op(&p, w, h);
            let dir = lf::light_direction(light_frac, h as f32 / w as f32, baked.focal_mm);
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
            // Which ranked path each combo came from — the K-369 ring slice
            // is a property of the path, not of the band.
            let mut pair_of = Vec::new();
            'outer: for (pi, &pair) in baked.pairs.iter().take(p.max_ghosts as usize).enumerate() {
                for band in &bands {
                    if combos.len() >= combo_limit as usize {
                        break 'outer;
                    }
                    combos.push((pair, band));
                    pair_of.push(pi);
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
            // Rays compared on a four-bounce path (K-368): the phases the
            // other two lenses never reach.
            let mut four_live = 0u32;
            // Rays compared on a path whose iris mask is a K-369 ring slice.
            let mut ringed_live = 0u32;
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
                        // The iris mask with this ghost's own edge
                        // diffraction on it (K-370) — the same call the
                        // shader makes, with the same Fresnel number and the
                        // same ray-grid step.
                        let fresnel = lf::ghost_fresnel_number(
                            baked.spreads[pair_of[ci]] * stop_scale,
                            p.fstop,
                        );
                        let mask = lf::ghost_mask(
                            u,
                            v,
                            p.blades,
                            rot,
                            roundness,
                            p.aperture_softness,
                            fresnel,
                            2.0 / g1f,
                        );
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
                                if pair[2] != lf::NO_BOUNCE {
                                    four_live += 1;
                                }
                                if fresnel > 0.0 {
                                    ringed_live += 1;
                                }
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
            // Without this the ring branch could be wrong in the shader and
            // every bound above would still pass, because nothing would have
            // taken it (K-369).
            assert!(
                ringed_live > 0,
                "no ringed-mask ray was compared — the K-369 branch went unchecked"
            );
            if lens == 17 {
                // Without this the extra phases could be wrong in the
                // shader and every bound above would still pass, because
                // nothing would have walked them.
                assert!(
                    four_live > 0,
                    "no four-bounce ray was compared — the K-368 phases went unchecked"
                );
            }
        }
    }
}

/// **Sprite flare** (docs/08 §3.29, K-359): the WGSL agrees with the CPU
/// reference, the neutral points pass through bit-exactly, and — the property
/// the whole effect exists for — moving the light moves the flare *smoothly*,
/// with no threshold to pop across.
#[test]
fn wgsl_sprite_flare_matches_the_cpu_oracle_and_never_pops() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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

/// **An area light flares like an area, not like a point** (K-355, K-367).
///
/// Source size gives the light a real emitting area, and the flare of one is
/// the integral of the point flares across it — which since K-367 every ray
/// evaluates for itself rather than the pipeline running once per sample. So
/// the picture must genuinely change — a wider source spreads its ghosts —
/// while the total light it adds stays put, because the pupil grid averages
/// over the source instead of adding lights to it. A source that grew brighter
/// as it grew wider would be the obvious way to get this wrong, and is what
/// the energy bound below catches.
#[test]
fn an_area_source_spreads_its_flare_without_gaining_energy() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    assert_ne!(
        lf::manual_light(&area, w, h)[0].extent,
        [0.0, 0.0],
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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

    // The same bound with a real Source size (K-367). An area source is no
    // longer a list of point lights both twins can be handed: each ray works
    // out its own point of the emitting rectangle, on the CPU in
    // `source_jitter` and in the shader's own copy of it. If those two ever
    // drift the pictures diverge for area sources ALONE, which no point-source
    // oracle would see.
    let area = lf::LensFlareParams {
        source_size: [18.0, 4.0],
        ..p
    };
    assert_ne!(
        lf::manual_light(&area, w, h)[0].extent,
        [0.0, 0.0],
        "this case must actually be an area source"
    );
    let a_baked = lf::bake(&area);
    let a_lights = lf::manual_light(&area, w, h);
    let a_flare = lf::cpu_flare(&area, &a_baked, fw, fh, &a_lights);
    let mut a_cpu = img.clone();
    lf::cpu_combine(
        &mut a_cpu, w, h, &area, &a_baked, &a_flare, fw, fh, &a_lights,
    );
    let a_out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &flare_op(&area, w, h),
        None,
        &(std::sync::Arc::new(move || flare_bake_data(&area)) as crate::fx::FlareBake),
        &flare_probe(&area, w, h),
    );
    let a_gpu = readback_linear_f32(&ctx, &a_out, w, h).unwrap();
    let a_mean: f32 = a_cpu
        .iter()
        .zip(&a_gpu)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / a_cpu.len() as f32;
    assert!(a_mean < 2e-3, "area-source mean |Δ| {a_mean}");
    let a_ratio = a_gpu.iter().sum::<f32>() / a_cpu.iter().sum::<f32>().max(1e-9);
    assert!(
        (0.99..=1.01).contains(&a_ratio),
        "area-source energy ratio {a_ratio}"
    );
}

/// K-267: an anamorphic squeeze below 1 renders into a PADDED flare buffer,
/// so the widened field carries real flare where K-266's zero-outside tap
/// showed black — and the padded pipeline still matches the CPU reference.
/// Fails without the padding: the region past the base buffer's edge is
/// exactly zero on the GPU, and the edge-energy floor below trips.
#[test]
fn wgsl_lens_flare_padded_anamorphic_matches_and_fills_the_edge() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
    // **Where any shortfall sits.** This oracle has a standing 1.2-1.5%
    // energy gap (docs/TODO.md), and the numbers below are what a session
    // without a GPU has to work from, so they are printed on every run rather
    // than reconstructed each time somebody picks the entry up. Split three
    // ways: the outermost ring of the padded buffer, where a clipping
    // difference would live; the edge fifths the assertion above is about; and
    // the middle.
    {
        let (mut ec, mut eg) = ([0.0f64; 3], [0.0f64; 3]);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let border = x == 0 || y == 0 || x + 1 == w as usize || y + 1 == h as usize;
                let zone = if border {
                    0
                } else if x < fifth || x >= w as usize - fifth {
                    1
                } else {
                    2
                };
                for c in 0..3 {
                    let i = (y * w as usize + x) * 4 + c;
                    ec[zone] += f64::from(cpu[i]);
                    eg[zone] += f64::from(gpu[i]);
                }
            }
        }
        for (zone, name) in ["border ring", "edge fifths", "middle"].iter().enumerate() {
            eprintln!(
                "energy {name}: cpu {:.4} gpu {:.4} delta {:.4} ({:.3}%)",
                ec[zone],
                eg[zone],
                eg[zone] - ec[zone],
                100.0 * (eg[zone] - ec[zone]) / ec[zone].max(1e-9)
            );
        }
        let mut worst = (0.0f32, 0usize);
        for (i, (a, b)) in cpu.iter().zip(&gpu).enumerate() {
            let d = (a - b).abs();
            if d > worst.0 {
                worst = (d, i);
            }
        }
        let px = worst.1 / 4;
        let avg = e_cpu / cpu.len() as f32;
        eprintln!(
            "worst sample at pixel {:?}: delta {} (cpu {} gpu {}); {} of {} samples differ by more than the mean sample {}",
            (px % w as usize, px / w as usize),
            worst.0,
            cpu[worst.1],
            gpu[worst.1],
            cpu.iter()
                .zip(&gpu)
                .filter(|(a, b)| (*a - *b).abs() > avg)
                .count(),
            cpu.len(),
            avg
        );
    }
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
    assert_eq!(
        MAX_SOURCES as usize,
        lumit_core::fx::lens_flare::MAX_SOURCES
    );
    // The combine kernel's `flare_blend` implements exactly the menu
    // lumit-core declares (K-289) — a mode added to one and not the other
    // would silently clamp to Divide.
    assert_eq!(
        crate::fx::lens_flare::BLEND_COUNT as usize,
        lumit_core::fx::lens_flare::BLEND_OPTIONS.len()
    );

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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
        p.matte_invert,
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
        tinted.matte_invert,
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

    // The Matte row's Invert (K-395), in the detect kernel: reading a matte
    // inverted must be reading its complement straight, which is the WGSL
    // twin of the lumit-core assertion — and here it is checked on the
    // rendered frame, so a `1 − rgb` applied at one of the two loads and not
    // the other would show up as a different flare.
    let complement: Vec<f32> = matte
        .chunks_exact(4)
        .flat_map(|px| [1.0 - px[0], 1.0 - px[1], 1.0 - px[2], px[3]])
        .map(|v| f16_to_f32(f16_bits(v)))
        .collect();
    let complement_tex = upload_linear_f32(&ctx, &complement, w, h);
    let inverted_op = crate::fx::lens_flare::LensFlareOp {
        matte_invert: true,
        ..flare_op(&p, w, h)
    };
    let i_out = fx.lens_flare(
        &ctx,
        &tex,
        w,
        h,
        &inverted_op,
        Some(&complement_tex),
        &(std::sync::Arc::new(move || flare_bake_data(&p)) as crate::fx::FlareBake),
        &flare_probe(&p, w, h),
    );
    let i_gpu = readback_linear_f32(&ctx, &i_out, w, h).unwrap();
    assert_eq!(
        i_gpu, gpu,
        "the complement read inverted must be the matte read straight"
    );
}
/// Render every bundled lens through the real GPU pipeline into one tiled
/// montage (K-264) — the harness the curation was chosen with, kept because
/// "do the twenty look different" is a question only eyes answer.
/// `LUMIT_FLARE_DUMP` names the output PPM.
#[test]
#[ignore = "a diagnostic image dump, not a gate"]
fn lens_flare_montage() {
    let Some(ctx) = crate::test_support::lease() else {
        return;
    };
    let fx = ctx.fx();
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

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
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

/// The §1.6 corpus with partial-alpha pixels injected: straight colour stored
/// premultiplied and quantised to f16, so both paths begin identical. The four
/// colour-family kernels below all run on unpremultiplied colour, where the
/// premultiply round trip is load-bearing — a kernel that graded the
/// premultiplied values would diverge exactly here and nowhere else.
fn alpha_corpus(w: u32, h: u32) -> Vec<f32> {
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

/// The §1.6 oracle for Curves (docs/08 §3.30, K-412): a pointwise table
/// lookup, so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable,
/// and the identity curve set or Mix 0 is the bit-exact identity on both
/// paths.
///
/// The parameters are built through the effect's own `packed()`, not by hand,
/// which is the point: the spline fit is host maths, and a test that fitted
/// its own would prove the kernel agrees with the test rather than with the
/// effect. What is left for the oracle to check is the lookup itself, which is
/// exactly what K-412 wanted of the baking.
#[test]
fn wgsl_curves_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::curves::Curves;
    use lumit_core::fx::{CurvePoints, EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    let neutral = Curves::read(Params::EMPTY);
    // A film S-curve on Master: shadows down, highlights up.
    let mut s_curve = neutral;
    s_curve.master = CurvePoints::sanitised(&[[0.0, 0.0], [0.25, 0.15], [0.75, 0.88], [1.0, 1.0]]);
    // A long flat run into a sudden rise — the shape a plain cubic overshoots
    // on, and so the one that exercises the bake's clamp.
    let mut crushed = neutral;
    crushed.master = CurvePoints::sanitised(&[[0.0, 0.1], [0.5, 0.1], [0.6, 0.95], [1.0, 1.0]]);
    // Per-channel: a warm grade, blue pulled down, red lifted.
    let mut warm = neutral;
    warm.red = CurvePoints::sanitised(&[[0.0, 0.0], [0.5, 0.62], [1.0, 1.0]]);
    warm.blue = CurvePoints::sanitised(&[[0.0, 0.0], [0.5, 0.38], [1.0, 1.0]]);
    // Alpha is its own channel now (K-412): a curve on it must move coverage
    // and take the premultiplied colour with it, identically on both paths.
    let mut alpha = neutral;
    alpha.alpha = CurvePoints::sanitised(&[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]);
    // Sixteen points, the declared maximum, so the tridiagonal solve is
    // exercised at its widest rather than at three points.
    let mut wobble = neutral;
    wobble.master = CurvePoints::sanitised(
        &(0..16)
            .map(|i| {
                let x = i as f32 / 15.0;
                [x, (x + 0.08 * (x * 9.0).sin()).clamp(0.0, 1.0)]
            })
            .collect::<Vec<_>>(),
    );

    for (name, curves, mix) in [
        ("neutral", neutral, 1.0f32),
        ("s-curve", s_curve, 1.0),
        ("crushed", crushed, 1.0),
        ("warm", warm, 1.0),
        ("alpha", alpha, 1.0),
        ("wobble", wobble, 1.0),
        ("mixed", s_curve, 0.6),
        ("mix-zero", s_curve, 0.0),
    ] {
        let mut c = curves;
        c.mix = mix * 100.0;
        let packed = c.packed();
        let op = CurvesOp {
            t: packed.t,
            neutral: packed.neutral,
            mix: packed.mix,
        };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::curves(&mut cpu, &packed);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.curves(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("curves {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the curve must actually do something");
        }

        let out2 = fx.curves(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU curves must be bit-stable");
    }
}

/// The §1.6 oracle for Levels (docs/08 §3.31), the same shape as Curves': the
/// reciprocals are host maths, so the op is built through `packed()`.
#[test]
fn wgsl_levels_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::levels::Levels;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    let neutral = Levels::read(Params::EMPTY);
    let mut stretched = neutral;
    stretched.master_in_black = 0.1;
    stretched.master_in_white = 0.8;
    let mut bent = neutral;
    bent.master_gamma = 1.8;
    let mut lifted = neutral;
    lifted.master_out_black = 0.12;
    lifted.master_out_white = 0.9;
    let mut per_channel = neutral;
    per_channel.blue_in_white = 0.75;
    per_channel.red_gamma = 0.6;
    // A white point dragged below the black point: the span floors instead of
    // dividing by zero, and the picture saturates.
    let mut inverted_span = neutral;
    inverted_span.master_in_black = 0.7;
    inverted_span.master_in_white = 0.2;

    for (name, levels, mix) in [
        ("neutral", neutral, 1.0f32),
        ("stretched", stretched, 1.0),
        ("bent", bent, 1.0),
        ("lifted", lifted, 1.0),
        ("per-channel", per_channel, 1.0),
        ("inverted-span", inverted_span, 1.0),
        ("mixed", stretched, 0.6),
        ("mix-zero", stretched, 0.0),
    ] {
        let mut l = levels;
        l.mix = mix * 100.0;
        let (r, mix_amt) = l.packed();
        let op = LevelsOp { r, mix: mix_amt };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::levels(&mut cpu, op.r, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.levels(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("levels {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the map must actually do something");
        }

        let out2 = fx.levels(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU levels must be bit-stable");
    }
}

/// The §1.6 oracle for Brightness (docs/08 §3.32): an affine grade, so the
/// tolerance is the ≤ 2 fp16 ULP of the cheap class, and the neutral pair or
/// Mix 0 is the bit-exact identity on both paths.
#[test]
fn wgsl_brightness_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    for (name, op) in [
        (
            "neutral",
            BrightnessOp {
                b: 0.0,
                k: 1.0,
                mix: 1.0,
            },
        ),
        (
            "brighter",
            BrightnessOp {
                b: 0.2,
                k: 1.0,
                mix: 1.0,
            },
        ),
        (
            "darker",
            BrightnessOp {
                b: -0.25,
                k: 1.0,
                mix: 1.0,
            },
        ),
        (
            "punchy",
            BrightnessOp {
                b: 0.05,
                k: 1.6,
                mix: 1.0,
            },
        ),
        (
            "flat",
            BrightnessOp {
                b: 0.0,
                k: 0.35,
                mix: 1.0,
            },
        ),
        (
            "mixed",
            BrightnessOp {
                b: 0.2,
                k: 1.6,
                mix: 0.6,
            },
        ),
        (
            "mix-zero",
            BrightnessOp {
                b: 0.2,
                k: 1.6,
                mix: 0.0,
            },
        ),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::brightness(&mut cpu, op.b, op.k, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.brightness(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("brightness {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }

        let out2 = fx.brightness(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU brightness must be bit-stable");
    }
}

/// The §1.6 oracle for Hue and saturation (docs/08 §3.33). The HSV round trip
/// has branches — which sector the hue falls in, which channel is the maximum
/// — so this is the one of the four where CPU and GPU could plausibly take
/// different paths; the corpus sweeps a full gradient, which crosses every
/// sector boundary.
#[test]
fn wgsl_hue_saturation_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::hue_saturation::HueSaturation;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    let neutral = HueSaturation::read(Params::EMPTY);
    let mut turned = neutral;
    turned.master_hue = 40.0;
    let mut vivid = neutral;
    vivid.master_saturation = 45.0;
    let mut dim = neutral;
    dim.master_lightness = -35.0;
    // The range half: the greens pulled toward teal and lifted, which is the
    // grade this effect exists for, and the blues desaturated.
    let mut ranged = neutral;
    ranged.greens_hue = 25.0;
    ranged.greens_lightness = 20.0;
    ranged.blues_saturation = -60.0;
    // A hue wound past a whole turn: the fold has to land in the same place
    // on both paths.
    let mut wound = neutral;
    wound.master_hue = 400.0;

    for (name, hs, mix) in [
        ("neutral", neutral, 1.0f32),
        ("turned", turned, 1.0),
        ("vivid", vivid, 1.0),
        ("dim", dim, 1.0),
        ("ranged", ranged, 1.0),
        ("wound", wound, 1.0),
        ("mixed", ranged, 0.6),
        ("mix-zero", ranged, 0.0),
    ] {
        let mut v = hs;
        v.mix = mix * 100.0;
        let (bands, mix_amt) = v.packed();
        let op = HueSaturationOp {
            bands,
            mix: mix_amt,
        };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::hue_saturation(&mut cpu, op.bands, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.hue_saturation(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("hue_saturation {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "neutral" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the grade must actually do something");
        }

        let out2 = fx.hue_saturation(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU hue and saturation must be bit-stable");
    }
}

/// The §1.6 oracle for Fill (docs/08 §3.34): a trivial pointwise flood, so CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Mix 0 is the
/// bit-exact identity on both paths.
#[test]
fn wgsl_fill_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::fill::Fill;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    let white = Fill::read(Params::EMPTY);
    let mut orange = white;
    orange.colour = [1.0, 0.4, 0.05, 1.0];
    let mut hdr = white;
    hdr.colour = [3.0, 2.5, 0.5, 1.0];
    let mut black = white;
    black.colour = [0.0, 0.0, 0.0, 1.0];

    for (name, fill, mix) in [
        ("white", white, 1.0f32),
        ("orange", orange, 1.0),
        ("hdr", hdr, 1.0),
        ("black", black, 1.0),
        ("mixed", orange, 0.6),
        ("mix-zero", orange, 0.0),
    ] {
        let mut f = fill;
        f.mix = mix * 100.0;
        let (colour, mix_amt) = f.packed();
        let op = FillOp {
            colour,
            mix: mix_amt,
        };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::fill(&mut cpu, op.colour, op.mix);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.fill(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("fill {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the fill must actually do something");
        }
        // Alpha is untouched — the whole point of the effect (§3.34).
        for (g, o) in gpu.chunks_exact(4).zip(img.chunks_exact(4)) {
            assert_eq!(g[3], o[3], "{name}: alpha must pass through");
        }

        let out2 = fx.fill(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU fill must be bit-stable");
    }
}

/// The §1.6 oracle for Gradient (docs/08 §3.35): a cheap pointwise generator, so
/// CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Mix 0 is
/// the bit-exact identity on both paths. The degenerate case (Start and End at
/// the same point) is in the sweep on purpose: the floored reciprocal must fill
/// with the End colour on both paths rather than faulting on either.
#[test]
fn wgsl_gradient_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::gradient::Gradient;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    // The declared points are px@comp; the resolve step would have scaled them
    // to this 32×24 raster, so the test does the same by hand.
    let base = {
        let mut g = Gradient::read(Params::EMPTY);
        g.start_x = 0.0;
        g.start_y = 0.0;
        g.end_x = 32.0;
        g.end_y = 24.0;
        g
    };
    let mut radial = base;
    radial.shape = 1;
    radial.start_x = 16.0;
    radial.start_y = 12.0;
    let mut scattered = base;
    scattered.scatter = 40.0;
    scattered.seed = 7;
    let mut hdr = base;
    hdr.start_colour = [4.0, 2.0, 0.5, 1.0];
    let mut degenerate = base;
    degenerate.end_x = 0.0;
    degenerate.end_y = 0.0;

    for (name, gradient, mix) in [
        ("linear", base, 1.0f32),
        ("radial", radial, 1.0),
        ("scattered", scattered, 1.0),
        ("hdr", hdr, 1.0),
        ("degenerate", degenerate, 1.0),
        ("mixed", base, 0.6),
        ("mix-zero", base, 0.0),
    ] {
        let mut g = gradient;
        g.mix = mix * 100.0;
        let p = g.packed();
        let op = GradientOp {
            radial: p.radial,
            start: p.start,
            axis: p.axis,
            inv_len2: p.inv_len2,
            inv_len: p.inv_len,
            c0: p.c0,
            c1: p.c1,
            scatter: p.scatter,
            seed: p.seed,
            mix: p.mix,
            clip_to_alpha: p.clip_to_alpha,
        };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::gradient(&mut cpu, w, h, &p);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.gradient(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("gradient {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the ramp must actually do something");
        }
        if mix == 1.0 {
            // A generator writes opaque pixels edge to edge (§3.35).
            for g in gpu.chunks_exact(4) {
                assert_eq!(g[3], 1.0, "{name}: the ramp must be opaque");
            }
        }
        if name == "degenerate" {
            // The floored reciprocal collapses the ramp to one flat colour
            // rather than faulting (§3.35): every pixel is the same.
            let first = &gpu[..4];
            for px in gpu.chunks_exact(4) {
                assert_eq!(px, first, "degenerate: the ramp must collapse flat");
            }
        }

        let out2 = fx.gradient(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU gradient must be bit-stable");
    }
}

/// The §1.6 oracle for Noise (docs/08 §3.36): a cheap pointwise modifier, so CPU
/// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Amount 0 or
/// Mix 0 is the bit-exact identity on both paths.
///
/// The integer hash is the load-bearing part: mono and colour, uniform and
/// gaussian, and two different ticks are all in the sweep, because a draw that
/// disagreed between the paths would show up as a whole-image mismatch rather
/// than a last-bit one.
#[test]
fn wgsl_noise_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::noise::Noise;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    let uniform = Noise::read(Params::EMPTY);
    let mut gaussian = uniform;
    gaussian.distribution = 1;
    let mut colour = uniform;
    colour.colour_noise = true;
    let mut colour_gaussian = uniform;
    colour_gaussian.distribution = 1;
    colour_gaussian.colour_noise = true;
    let mut frozen = uniform;
    frozen.animate = false;
    let mut zero = uniform;
    zero.amount = 0.0;

    for (name, noise, tick, mix) in [
        ("uniform", uniform, 0i32, 1.0f32),
        ("uniform-later", uniform, 41, 1.0),
        ("gaussian", gaussian, 41, 1.0),
        ("colour", colour, 41, 1.0),
        ("colour-gaussian", colour_gaussian, 41, 1.0),
        ("frozen", frozen, 41, 1.0),
        ("amount-zero", zero, 41, 1.0),
        ("mixed", uniform, 41, 0.6),
        ("mix-zero", uniform, 41, 0.0),
    ] {
        let mut n = noise;
        n.mix = mix * 100.0;
        n.seed = 12345;
        let (amount, gauss, colour_noise, seed, resolved_tick, mix_amt) = n.packed(tick);
        let op = NoiseOp {
            amount,
            gaussian: gauss,
            colour_noise,
            seed,
            tick: resolved_tick,
            mix: mix_amt,
        };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::noise(
            &mut cpu,
            w,
            h,
            op.amount,
            op.gaussian,
            op.colour_noise,
            op.seed,
            op.tick,
            op.mix,
        );

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.noise(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("noise {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "amount-zero" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the grain must actually do something");
        }
        // A modifier never touches alpha (§3.36).
        for (g, o) in gpu.chunks_exact(4).zip(img.chunks_exact(4)) {
            assert_eq!(g[3], o[3], "{name}: alpha must pass through");
        }

        let out2 = fx.noise(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU noise must be bit-stable");
    }
}

/// The §1.6 oracle for Fractal noise (docs/08 §3.37) — and, through it, for the
/// shared `lumit_core::fx::noise` core the displacement family will reuse.
///
/// Every branch of the core is in the sweep (value and Perlin, basic and
/// turbulent, one octave and ten, cycling and not), because this is the one
/// place the CPU and WGSL twins of the whole noise module are held together. The
/// tolerance is the `moderate` class's, scaled for a ten-octave sum: the
/// arithmetic order is identical on both paths, but ten multiply-accumulates in
/// fp32 followed by an fp16 store leave more room than a pointwise grade does.
#[test]
fn wgsl_fractal_noise_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::fractal_noise::FractalNoise;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);

    // The declared sizes are px@comp; the resolve step would have scaled them to
    // this 32×24 raster, so the test does the same by hand — a 200 px cell on a
    // 32 px raster would be one flat blob.
    let base = {
        let mut f = FractalNoise::read(Params::EMPTY);
        f.scale = 12.0;
        f.scale_width = 12.0;
        f.scale_height = 12.0;
        f.offset_x = 16.0;
        f.offset_y = 12.0;
        f.seed = 4242;
        f
    };
    let mut value_basic = base;
    value_basic.noise_type = 0;
    value_basic.fractal_type = 0;
    let mut value_turbulent = base;
    value_turbulent.noise_type = 0;
    let mut perlin_basic = base;
    perlin_basic.fractal_type = 0;
    let mut one_octave = base;
    one_octave.complexity = 1;
    let mut ten_octaves = base;
    ten_octaves.complexity = 10;
    let mut turned = base;
    turned.rotation = 37.0;
    let mut stretched = base;
    stretched.uniform_scaling = false;
    stretched.scale_width = 30.0;
    stretched.scale_height = 5.0;
    let mut shaped = base;
    shaped.contrast = 250.0;
    shaped.brightness = -30.0;
    let mut inverted = base;
    inverted.invert = true;
    let mut evolved = base;
    evolved.evolution = 400.0;
    let mut cycled = base;
    cycled.evolution = 900.0;
    cycled.cycle_evolution = true;
    cycled.cycle = 2;

    for (name, fractal, mix) in [
        ("perlin-turbulent", base, 1.0f32),
        ("value-basic", value_basic, 1.0),
        ("value-turbulent", value_turbulent, 1.0),
        ("perlin-basic", perlin_basic, 1.0),
        ("one-octave", one_octave, 1.0),
        ("ten-octaves", ten_octaves, 1.0),
        ("rotated", turned, 1.0),
        ("stretched", stretched, 1.0),
        ("shaped", shaped, 1.0),
        ("inverted", inverted, 1.0),
        ("evolved", evolved, 1.0),
        ("cycled", cycled, 1.0),
        ("mixed", base, 0.6),
        ("mix-zero", base, 0.0),
    ] {
        let mut f = fractal;
        f.mix = mix * 100.0;
        let p = f.packed();
        let op = FractalNoiseOp {
            seed: p.field.seed,
            octaves: p.field.octaves,
            gain: p.field.gain,
            lacunarity: p.field.lacunarity,
            perlin: p.field.perlin,
            turbulent: p.field.turbulent,
            cycle: p.field.cycle,
            cos_sin: p.cos_sin,
            offset: p.offset,
            inv_scale: p.inv_scale,
            z: p.z,
            contrast: p.contrast,
            brightness: p.brightness,
            invert: p.invert,
            mix: p.mix,
        };

        let mut cpu = img.clone();
        lumit_core::fx::cpu::fractal_noise(&mut cpu, w, h, &p);

        let tex = upload_linear_f32(&ctx, &img, w, h);
        let out = fx.fractal_noise(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("fractal_noise {name}: worst {worst} ulp");
        assert!(worst <= 4, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the field must actually do something");
            if mix == 1.0 {
                // A generator writes opaque pixels edge to edge (§3.37).
                for g in gpu.chunks_exact(4) {
                    assert_eq!(g[3], 1.0, "{name}: the field must be opaque");
                }
            }
            // ... and it must be a field, not a flat grey: a kernel that
            // silently produced a constant would pass every ULP check above.
            let lo = gpu.chunks_exact(4).map(|p| p[0]).fold(f32::MAX, f32::min);
            let hi = gpu.chunks_exact(4).map(|p| p[0]).fold(f32::MIN, f32::max);
            assert!(hi - lo > 0.05, "{name}: the field is flat ({lo}..{hi})");
        }

        let out2 = fx.fractal_noise(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU fractal noise must be bit-stable");
    }
}

/// Cycle evolution is an exact loop (docs/08 §3.37 decision 4): the field at
/// Evolution 0 and at Evolution `cycle × 360` are the same picture, at every
/// complexity — which is only true because the depth coordinate is not scaled
/// by frequency. A CPU test, because it is a claim about the shared noise core
/// rather than about a kernel.
#[test]
fn fractal_noise_cycle_is_an_exact_loop() {
    use lumit_core::fx::effects::fractal_noise::FractalNoise;
    use lumit_core::fx::{EffectMetadata, Params};

    let (w, h) = (24u32, 16u32);
    let flat = vec![0.0f32; (w * h * 4) as usize];
    for complexity in [1i32, 4, 10] {
        let mut f = FractalNoise::read(Params::EMPTY);
        f.scale = 9.0;
        f.scale_width = 9.0;
        f.scale_height = 9.0;
        f.complexity = complexity;
        f.cycle_evolution = true;
        f.cycle = 3;
        let mut at_zero = flat.clone();
        lumit_core::fx::cpu::fractal_noise(&mut at_zero, w, h, &f.packed());

        let mut looped = f;
        looped.evolution = 3.0 * 360.0;
        let mut at_loop = flat.clone();
        lumit_core::fx::cpu::fractal_noise(&mut at_loop, w, h, &looped.packed());
        assert_eq!(
            at_zero, at_loop,
            "complexity {complexity}: the cycle must close exactly"
        );

        // ... and it must not be trivially closed by the field being constant
        // in depth: half a cycle along is a different picture.
        let mut half = f;
        half.evolution = 1.5 * 360.0;
        let mut at_half = flat.clone();
        lumit_core::fx::cpu::fractal_noise(&mut at_half, w, h, &half.packed());
        assert_ne!(
            at_zero, at_half,
            "complexity {complexity}: the field must actually evolve"
        );
    }
}

/// The distort batch's corpus (docs/08 §3.38–§3.42): **smooth on purpose**.
///
/// Every effect in the batch decides *where to sample from* and then takes one
/// bilinear tap. That makes the parity question "do the two paths compute the
/// same position?", and a position that differs in its last few bits shows up in
/// the picture multiplied by the local gradient — so a corpus with a hard edge in
/// it measures the size of the edge, not the accuracy of the kernel. This one has
/// no step anywhere: two smooth colour ramps, a smooth alpha falloff, and a broad
/// HDR hump instead of a one-pixel spike. Already fp16-quantised, so both paths
/// begin identical.
fn smooth_corpus(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let u = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            // A broad hump, so the picture has real structure without a step.
            let hump = 1.0 + 2.0 * (1.0 - (u - 0.5).abs() * 2.0) * (1.0 - (v - 0.5).abs() * 2.0);
            let a = 0.4 + 0.6 * v;
            img[i] = u * hump * a;
            img[i + 1] = v * hump * a;
            img[i + 2] = (1.0 - u) * 0.5 * a;
            img[i + 3] = a;
        }
    }
    img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

/// The §1.6 oracle for Turbulent displace (docs/08 §3.38), and with it the
/// second reader of the shared noise core — a Fractal noise and a Turbulent
/// displace that disagreed about the field would be two effects that cannot be
/// used together.
///
/// Judged on absolute difference rather than fp16 ULPs, on the smooth corpus
/// above, for the reason that corpus documents: this kernel's output is a
/// *sample position*, and the tolerance has to be stated in the picture rather
/// than in the last bit of a value neither path computes directly.
///
/// Three claims beyond parity, because each is a way the kernel could be wrong
/// while agreeing with a wrong oracle: Amount 0 is the exact identity, the warp
/// actually moves pixels, and a pinned edge does not move while an unpinned one
/// does.
#[test]
fn wgsl_turbulent_displace_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::turbulent_displace::TurbulentDisplace;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);

    // The declared lengths are px@comp; the resolve step would have scaled them
    // to this 32×24 raster, so the test does the same by hand.
    let base = {
        let mut t = TurbulentDisplace::read(Params::EMPTY);
        t.size = 12.0;
        t.amount = 4.0;
        t.offset_x = 16.0;
        t.offset_y = 12.0;
        t.seed = 4242;
        t
    };
    let op_of = |t: TurbulentDisplace| {
        let p = t.packed();
        TurbulentDisplaceOp {
            seed_x: p.field.seed,
            seed_y: p.seed_y,
            octaves: p.field.octaves,
            gain: p.field.gain,
            lacunarity: p.field.lacunarity,
            cycle: p.field.cycle,
            offset: p.offset,
            inv_size: p.inv_size,
            z: p.z,
            amount: p.amount,
            axes: p.axes,
            pin: p.pin,
            inv_pin_band: p.inv_pin_band,
            mix: p.mix,
        }
    };

    let mut horizontal = base;
    horizontal.displacement = 1;
    let mut vertical = base;
    vertical.displacement = 2;
    let mut unpinned = base;
    unpinned.pinning = 0;
    let mut sideways = base;
    sideways.pinning = 2;
    let mut negative = base;
    negative.amount = -4.0;
    let mut one_octave = base;
    one_octave.complexity = 1;
    let mut ten_octaves = base;
    ten_octaves.complexity = 10;
    let mut evolved = base;
    evolved.evolution = 400.0;
    let mut cycled = base;
    cycled.evolution = 900.0;
    cycled.cycle_evolution = true;
    cycled.cycle = 2;
    let mut still = base;
    still.amount = 0.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut off = base;
    off.mix = 0.0;

    let tex = upload_linear_f32(&ctx, &img, w, h);
    for (name, t) in [
        ("default", base),
        ("horizontal", horizontal),
        ("vertical", vertical),
        ("unpinned", unpinned),
        ("left-and-right", sideways),
        ("negative", negative),
        ("one-octave", one_octave),
        ("ten-octaves", ten_octaves),
        ("evolved", evolved),
        ("cycled", cycled),
        ("amount-zero", still),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = t.packed();
        let op = op_of(t);
        let mut cpu = img.clone();
        lumit_core::fx::cpu::turbulent_displace(&mut cpu, w, h, &p, &[]);

        let out = fx.turbulent_displace(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("turbulent_displace {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");

        match name {
            "amount-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the warp must actually move something"),
        }

        let out2 = fx.turbulent_displace(&ctx, &tex, w, h, None, &op);
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "{name}: the warp must be bit-stable"
        );
    }

    // **Pinning holds the border still.** With Pin all edges the frame's top row
    // must be exactly what arrived; with Pin none it must not be.
    let row_moved = |t: TurbulentDisplace| -> bool {
        let out = fx.turbulent_displace(&ctx, &tex, w, h, None, &op_of(t));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        (0..w).any(|x| {
            let i = (x * 4) as usize;
            gpu[i..i + 4] != img[i..i + 4]
        })
    };
    assert!(!row_moved(base), "a pinned top edge must not move");
    assert!(row_moved(unpinned), "an unpinned top edge must move");
}

/// **The matte scales the displacement** (K-395, docs/08 §3.38): the override
/// half of Turbulent displace, and the claim that it is worth having.
///
/// Parity first, at both Invert settings, because the matted path multiplies a
/// vector the unmatted one does not. Then the picture claim: under a flat matte
/// at a quarter, a pixel must move about a quarter as far as it does at full
/// matte — *less warp*, not a full warp faded back. A generic dissolve cannot
/// produce that, and this measures the distance rather than the difference,
/// which is the only way to tell the two apart.
#[test]
fn wgsl_matted_turbulent_displace_scales_the_displacement() {
    use lumit_core::fx::effects::turbulent_displace::TurbulentDisplace;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut t = TurbulentDisplace::read(Params::EMPTY);
        t.size = 12.0;
        t.amount = 6.0;
        t.offset_x = 16.0;
        t.offset_y = 12.0;
        t.pinning = 0; // the pin ramp would confound the distance probe below
        t.seed = 99;
        t
    };
    let op_of = |t: TurbulentDisplace| {
        let p = t.packed();
        TurbulentDisplaceOp {
            seed_x: p.field.seed,
            seed_y: p.seed_y,
            octaves: p.field.octaves,
            gain: p.field.gain,
            lacunarity: p.field.lacunarity,
            cycle: p.field.cycle,
            offset: p.offset,
            inv_size: p.inv_size,
            z: p.z,
            amount: p.amount,
            axes: p.axes,
            pin: p.pin,
            inv_pin_band: p.inv_pin_band,
            mix: p.mix,
        }
    };

    // A left-to-right ramp of matte, so the parity corpus covers every level.
    let mut ramp = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = x as f32 / (w - 1) as f32;
            ramp[i] = k;
            ramp[i + 1] = k;
            ramp[i + 2] = k;
            ramp[i + 3] = 1.0;
        }
    }
    let qramp: Vec<f32> = ramp.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    let ramp_tex = upload_linear_f32(&ctx, &ramp, w, h);
    // Invert arrives through the seam's prepare pass (K-425), both paths.
    for invert in [false, true] {
        let p = base.packed();
        let mut qm = qramp.clone();
        let mtex = if invert {
            lumit_core::fx::cpu::matte_prepare(&mut qm, 0, true);
            fx.matte_prepare(&ctx, &ramp_tex, w, h, 0, true)
        } else {
            ramp_tex.clone()
        };
        let mut cpu = img.clone();
        lumit_core::fx::cpu::turbulent_displace(&mut cpu, w, h, &p, &qm);
        let out = fx.turbulent_displace(&ctx, &tex, w, h, Some(&mtex), &op_of(base));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        assert!(worst < 2e-3, "invert {invert}: worst diff {worst}");
    }

    // **The distance probe.** A picture whose only lit pixel is at (16, 12) is
    // warped under three flat mattes; where the light ends up is what the matte
    // is supposed to scale. Measured on the CPU reference, which the parity check
    // above has already tied the kernel to — the question here is what the
    // arithmetic *means*, not which path ran it.
    let travel = |k: f32| -> f32 {
        let mut dot = vec![0.0f32; (w * h * 4) as usize];
        let src = ((12 * w + 16) * 4) as usize;
        dot[src..src + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        let flat: Vec<f32> = (0..(w * h) as usize).flat_map(|_| [k, k, k, 1.0]).collect();
        lumit_core::fx::cpu::turbulent_displace(&mut dot, w, h, &base.packed(), &flat);
        // The lit pixel's new centre of mass, distance from where it began.
        let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
        for y in 0..h {
            for x in 0..w {
                let v = dot[((y * w + x) * 4) as usize];
                sx += v * x as f32;
                sy += v * y as f32;
                sw += v;
            }
        }
        if sw <= 0.0 {
            return 0.0;
        }
        let (cx, cy) = (sx / sw - 16.0, sy / sw - 12.0);
        (cx * cx + cy * cy).sqrt()
    };
    let full = travel(1.0);
    let quarter = travel(0.25);
    let none = travel(0.0);
    eprintln!("turbulent displace travel: full {full}, quarter {quarter}, none {none}");
    assert!(full > 1.0, "the unmatted warp must move the light: {full}");
    assert!(
        none < 1e-3,
        "a black matte must leave the picture alone: {none}"
    );
    // A quarter matte is a quarter of the vector, not a quarter-strength blend
    // of a full one — which would leave the light in the SAME place, only fainter.
    assert!(
        quarter < full * 0.5 && quarter > full * 0.05,
        "a quarter matte must warp about a quarter as far: {quarter} vs {full}"
    );
}

/// The §1.6 oracle for Tile (docs/08 §3.39), on the smooth corpus and by
/// absolute difference, for the reason [`smooth_corpus`] gives: the kernel's real
/// output is a sample position, and where the two paths' arithmetic is
/// contracted differently (a multiply-add fused on one and not the other) the
/// position moves in its last bits — which a hard edge magnifies into a whole
/// pixel of colour and a smooth picture does not.
///
/// Every branch is in the sweep — mirrored and not, both phase axes, a window
/// smaller than the frame — plus the two claims a passing parity check would not
/// make: the effect genuinely repeats the picture, and the area outside the
/// output window is genuinely transparent.
#[test]
fn wgsl_tile_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::tile::Tile;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // The shipped defaults, centred on this raster the way
    // `instantiate_for_raster` centres them on a real comp (K-542). This is the
    // identity, and the cases below build on it.
    // The four sizes are px@comp since K-558, and the identity is one whole
    // frame of *this* raster, which is what `instantiate_for_raster` writes.
    let base = {
        let mut t = Tile::read(Params::EMPTY);
        t.tile_centre_x = 16.0;
        t.tile_centre_y = 12.0;
        t.tile_width = w as f32;
        t.tile_height = h as f32;
        t.output_width = w as f32;
        t.output_height = h as f32;
        t
    };
    let tiled = {
        let mut t = base;
        t.tile_width = w as f32 * 0.5;
        t.tile_height = h as f32 * 0.5;
        t
    };
    let mut mirrored = tiled;
    mirrored.mirror_edges = true;
    let mut phased = tiled;
    phased.phase = 180.0;
    let mut phased_h = tiled;
    phased_h.phase = 180.0;
    phased_h.horizontal_phase_shift = true;
    let mut windowed = tiled;
    windowed.output_width = w as f32 * 0.6;
    windowed.output_height = h as f32 * 0.6;
    let mut wide = tiled;
    wide.tile_width = w as f32 * 0.25;
    wide.tile_height = h as f32 * 2.0;
    // The growing case (K-542): an output window wider than the frame writes a
    // wider raster.
    let mut grown = tiled;
    grown.output_width = w as f32 * 2.0;
    grown.output_height = h as f32 * 1.5;
    // A tile cut from a quarter in, with a window wider than it: the window is
    // centred on the tile centre (K-613), so the kernels have to agree about
    // where it reaches on both sides.
    let mut off_centre = tiled;
    off_centre.tile_centre_x = w as f32 * 0.25;
    off_centre.tile_centre_y = h as f32 * 0.25;
    off_centre.output_width = w as f32;
    off_centre.output_height = h as f32;
    let mut off = tiled;
    off.mix = 0.0;

    // `cpu::tile_into` at the raster `cpu::tile_raster` sizes, against
    // `FxEngine::tile` at the raster it sizes for itself from the same rule.
    let run = |t: Tile| {
        let p = t.packed(w as f32, h as f32);
        let (ow, oh) = lumit_core::fx::cpu::tile_raster(w, h, &p);
        let mut cpu = vec![0.0f32; (ow * oh * 4) as usize];
        lumit_core::fx::cpu::tile_into(&img, w, h, &mut cpu, ow, oh, &p);
        let out = fx.tile(
            &ctx,
            &tex,
            w,
            h,
            &TileOp {
                centre: p.centre,
                tile_frac: p.tile_frac,
                output_frac: p.output_frac,
                phase: p.phase,
                mirror_edges: p.mirror_edges,
                horizontal_phase_shift: p.horizontal_phase_shift,
                mix: p.mix,
                out_raster: (ow, oh),
            },
        );
        assert_eq!(
            (out.width(), out.height()),
            (ow, oh),
            "the kernel must write the raster the oracle sized"
        );
        let gpu = readback_linear_f32(&ctx, &out, ow, oh).unwrap();
        (ow, oh, cpu, gpu)
    };

    for (name, t) in [
        ("identity", base),
        ("tiled", tiled),
        ("mirrored", mirrored),
        ("phased", phased),
        ("phased-columns", phased_h),
        ("windowed", windowed),
        ("stretched", wide),
        ("grown", grown),
        ("off-centre", off_centre),
        ("mix-zero", off),
    ] {
        let (ow, oh, cpu, gpu) = run(t);
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("tile {name}: {ow}x{oh}, worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            // **The default is the identity, to the bit** (§1.2, K-542): a fresh
            // Tile dropped on a layer must change nothing, and "nothing" here is
            // not "nothing you can see" — the short-circuit both kernels take
            // means the pixels are the same pixels.
            "identity" | "mix-zero" => {
                assert_eq!((ow, oh), (w, h), "{name}: the raster must not grow");
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != cpu[..] || gpu != img, "{name}: must do something"),
        }
    }

    // **It repeats.** At a 2×2 tiling the pixel at (4, 3) and the one a tile
    // away at (20, 15) come from the same place, so they must match.
    let (_, _, cpu_tiled, _) = run(tiled);
    let a = ((3 * w + 4) * 4) as usize;
    let b = ((15 * w + 20) * 4) as usize;
    assert_eq!(
        cpu_tiled[a..a + 4],
        cpu_tiled[b..b + 4],
        "tiles one period apart must be the same picture"
    );

    // **The window clips.** At 60 % output the frame's corner is outside it.
    let (_, _, cpu_windowed, _) = run(windowed);
    assert_eq!(
        cpu_windowed[0..4],
        [0.0; 4],
        "outside the output window must be transparent"
    );

    // **Above 100 % the picture grows, and the margin is picture** (K-542).
    // The raster is Output width and height of the frame, the frame's own
    // window inside it is what the ungrown tiling produced — the growth adds,
    // it does not move anything — and the margin holds copies rather than the
    // transparency a layer's edge used to be.
    let (ow, oh, cpu_grown, gpu_grown) = run(grown);
    assert_eq!((ow, oh), (64, 36), "output 200/150 % of 32×24");
    let (ox, oy) = ((ow - w) / 2, (oh - h) / 2);
    for y in 0..h {
        for x in 0..w {
            let inner = (((y + oy) * ow + x + ox) * 4) as usize;
            let flat = ((y * w + x) * 4) as usize;
            assert_eq!(
                cpu_grown[inner..inner + 4],
                cpu_tiled[flat..flat + 4],
                "the frame's own window must be unmoved at ({x}, {y})"
            );
        }
    }
    let margin: f32 = (0..ow)
        .map(|x| gpu_grown[((2 * ow + x) * 4 + 3) as usize])
        .sum();
    assert!(
        margin > 0.5 * ow as f32,
        "the grown margin must hold picture, not transparency (alpha sum {margin})"
    );
}

/// The §1.6 oracle for Offset (docs/08 §3.40): exact arithmetic and a wrapping
/// sampler, so ULPs on the hard-edged corpus. Beyond parity: a whole-pixel shift
/// must be an exact rotation of the picture (nothing lost, nothing blurred), and
/// a zero shift must be the bit-exact identity.
#[test]
fn wgsl_offset_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    for (name, sx, sy, mix) in [
        ("whole-pixels", 7.0f32, 5.0f32, 1.0f32),
        ("fractional", 3.5, -2.25, 1.0),
        ("negative", -11.0, -19.0, 1.0),
        ("past-the-frame", 70.0, 51.0, 1.0),
        ("mixed", 7.0, 5.0, 0.6),
        ("mix-zero", 7.0, 5.0, 0.0),
        ("still", 0.0, 0.0, 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::offset(&mut cpu, w, h, [sx, sy], mix);
        let out = fx.offset(&ctx, &tex, w, h, None, [sx, sy], mix);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("offset {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" || name == "still" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the shift must actually move something");
        }
    }

    // **A whole-pixel shift is a rotation of the picture**: every output pixel is
    // some input pixel, unblurred, and the wrap means none is lost.
    let mut shifted = img.clone();
    lumit_core::fx::cpu::offset(&mut shifted, w, h, [7.0, 5.0], 1.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let s = ((((y + h - 5) % h) * w + (x + w - 7) % w) * 4) as usize;
            assert_eq!(shifted[d..d + 4], img[s..s + 4], "at {x},{y}");
        }
    }
}

/// The §1.6 oracle for Mirror (docs/08 §3.41), on the smooth corpus and by
/// absolute difference — see [`smooth_corpus`]: the reflected position is a
/// dot product, and a dot product is exactly the expression one path fuses into
/// a multiply-add and the other does not.
///
/// Beyond parity: at Angle 0 the frame must be genuinely symmetric about the
/// centre column, which is the whole claim of the effect and is not something a
/// parity check can make.
#[test]
fn wgsl_mirror_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::mirror::Mirror;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut m = Mirror::read(Params::EMPTY);
        m.centre_x = 16.0;
        m.centre_y = 12.0;
        m
    };
    let mut turned = base;
    turned.angle = 90.0;
    let mut diagonal = base;
    diagonal.angle = 45.0;
    let mut back = base;
    back.angle = 180.0;
    let mut off_centre = base;
    off_centre.centre_x = 6.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, m) in [
        ("vertical-axis", base),
        ("horizontal-axis", turned),
        ("diagonal", diagonal),
        ("reversed", back),
        ("off-centre", off_centre),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let (centre, normal, mix) = m.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::mirror(&mut cpu, w, h, centre, normal, mix);
        let out = fx.mirror(&ctx, &tex, w, h, centre, normal, mix);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("mirror {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the reflection must change something");
        }
    }

    // **The result is symmetric.** Angle 0 about the centre column: column x and
    // column (w − 1 − x) must be the same picture.
    let (centre, normal, mix) = base.packed();
    let mut done = img.clone();
    lumit_core::fx::cpu::mirror(&mut done, w, h, centre, normal, mix);
    for y in 0..h {
        for x in 0..w {
            let a = ((y * w + x) * 4) as usize;
            let b = ((y * w + (w - 1 - x)) * 4) as usize;
            assert_eq!(done[a..a + 4], done[b..b + 4], "asymmetric at {x},{y}");
        }
    }
}

/// The §1.6 oracle for Lens distort (docs/08 §3.42), on the smooth corpus and by
/// absolute difference — §3.42's fourth note says why: the two transcendentals
/// are per pixel and cannot be lifted out, so the paths differ by their own
/// platforms' `tan`, and the tolerance belongs in the picture rather than in the
/// last bit.
///
/// Two claims beyond parity. **Reverse is the true inverse**: distorting and
/// then undistorting at the same settings returns the picture, which a signed
/// coefficient would not. And **Field of view 0 is the exact identity**, which
/// is what the short-circuit exists for.
#[test]
fn wgsl_lens_distort_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::lens_distort::LensDistort;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut l = LensDistort::read(Params::EMPTY);
        l.centre_x = 16.0;
        l.centre_y = 12.0;
        l.edge = 1; // Repeat, so the sweep measures the mapping, not the border
        l
    };
    let mut reversed = base;
    reversed.reverse = true;
    let mut wide = base;
    wide.fov = 120.0;
    let mut vertical = base;
    vertical.orientation = 1;
    let mut diagonal = base;
    diagonal.orientation = 2;
    let mut shifted = base;
    shifted.centre_x = 8.0;
    shifted.centre_y = 6.0;
    let mut transparent = base;
    transparent.edge = 0;
    let mut mirrored = base;
    mirrored.edge = 2;
    let mut flat = base;
    flat.fov = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, l) in [
        ("barrel", base),
        ("pincushion", reversed),
        ("wide", wide),
        ("vertical-fov", vertical),
        ("diagonal-fov", diagonal),
        ("off-centre", shifted),
        ("transparent-edges", transparent),
        ("mirrored-edges", mirrored),
        ("fov-zero", flat),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = l.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::lens_distort(&mut cpu, w, h, &p);
        let out = fx.lens_distort(
            &ctx,
            &tex,
            w,
            h,
            None,
            &LensDistortOp {
                active: p.active,
                tan_half_fov: p.tan_half_fov,
                reverse: p.reverse,
                half_kind: p.half_kind,
                centre: p.centre,
                edge: p.edge,
                mix: p.mix,
            },
        );
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("lens_distort {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "fov-zero" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(
                gpu != img,
                "{name}: the bend must actually change something"
            );
        }
    }

    // **Reverse undoes it.** Distort, then undistort at the same settings, and
    // the middle of the frame — where no sample came from outside — must be the
    // picture again. The tolerance is a resampling one: two bilinear taps of a
    // smooth picture, not a lossless round trip.
    let mut there = img.clone();
    lumit_core::fx::cpu::lens_distort(&mut there, w, h, &base.packed());
    lumit_core::fx::cpu::lens_distort(&mut there, w, h, &reversed.packed());
    let mut worst = 0.0f32;
    for y in 6..h - 6 {
        for x in 8..w - 8 {
            let i = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                worst = worst.max((there[i + c] - img[i + c]).abs());
            }
        }
    }
    assert!(
        worst < 5e-2,
        "the round trip must return the picture: {worst}"
    );
}

/// The §1.6 oracle for Channel blur (docs/08 §3.45): the gaussian four times
/// over, so the blur family's absolute-difference tolerance on the hard-edged
/// corpus, and the blur family's determinism check.
///
/// Two claims beyond parity, because each is a way this kernel could agree with
/// a wrong oracle. **A channel with a zero radius is bit-identical to the
/// input's channel** — the whole point of the per-channel form is that the
/// untouched channels are genuinely untouched, not blurred by a kernel of width
/// one. And **a wider radius spreads further**: a lone bright pixel's light must
/// reach further in blue at radius 6 than at radius 2, which is what says the
/// four radii are actually read per channel rather than shared.
#[test]
fn wgsl_channel_blur_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    for (name, radii, edge, mix) in [
        ("blue-only", [0.0f32, 0.0, 4.0, 0.0], 1u32, 1.0f32),
        ("all-different", [1.0, 3.0, 6.5, 2.0], 1, 1.0),
        ("alpha-only", [0.0, 0.0, 0.0, 5.0], 1, 1.0),
        ("transparent-edges", [2.0, 2.0, 2.0, 2.0], 0, 1.0),
        ("mixed", [0.0, 0.0, 4.0, 0.0], 1, 0.6),
        ("mix-zero", [0.0, 0.0, 4.0, 0.0], 1, 0.0),
        ("all-zero", [0.0, 0.0, 0.0, 0.0], 1, 1.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::channel_blur(&mut cpu, w, h, radii, edge, mix);
        let op = ChannelBlurOp { radii, edge, mix };
        let out = fx.channel_blur(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("channel_blur {name}: worst {worst}");
        assert!(worst < 2e-2, "{name}: worst diff {worst}");

        if name == "mix-zero" || name == "all-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }
        // **The zero-radius channels are untouched, to the bit.**
        if name == "blue-only" {
            for i in (0..gpu.len()).step_by(4) {
                assert_eq!(gpu[i], img[i], "red moved at pixel {}", i / 4);
                assert_eq!(gpu[i + 1], img[i + 1], "green moved at pixel {}", i / 4);
                assert_eq!(gpu[i + 3], img[i + 3], "alpha moved at pixel {}", i / 4);
            }
            assert!(
                (0..gpu.len()).step_by(4).any(|i| gpu[i + 2] != img[i + 2]),
                "blue must actually have been blurred"
            );
        }

        let out2 = fx.channel_blur(&ctx, &tex, w, h, None, &op);
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "{name}: the blur must be bit-stable"
        );
    }

    // **A wider radius reaches further.** One bright pixel in blue, blurred at
    // two radii; the narrow one's light must die out sooner along the row.
    let mut dot = vec![0.0f32; (w * h * 4) as usize];
    let centre = ((12 * w + 16) * 4) as usize;
    dot[centre..centre + 4].copy_from_slice(&[0.0, 0.0, 1.0, 1.0]);
    let dot_tex = upload_linear_f32(&ctx, &dot, w, h);
    let reach = |r: f32| -> usize {
        let out = fx.channel_blur(
            &ctx,
            &dot_tex,
            w,
            h,
            None,
            &ChannelBlurOp {
                radii: [0.0, 0.0, r, 0.0],
                edge: 1,
                mix: 1.0,
            },
        );
        let px = readback_linear_f32(&ctx, &out, w, h).unwrap();
        (0..w)
            .filter(|x| px[((12 * w + x) * 4 + 2) as usize] > 1e-4)
            .count()
    };
    let (narrow, wide) = (reach(2.0), reach(6.0));
    assert!(
        wide > narrow,
        "radius 6 must reach further than radius 2: {wide} vs {narrow}"
    );
}

/// The §1.6 oracle for Drop shadow (docs/08 §3.43): a gaussian and one bilinear
/// tap, so the blur family's absolute-difference tolerance on the alpha corpus —
/// which is the right corpus here, since a shadow is *made of* the alpha edge.
///
/// Three claims beyond parity. **The shadow lands where the light is not**: a
/// transparent pixel down and to the right of the shape must gain coverage at
/// the default 135°. **Shadow only throws the layer away**, so the shape's own
/// opaque middle loses its colour. And **Opacity 0 and Mix 0 are the exact
/// identity**, which is what makes the effect safe to keyframe up from nothing.
#[test]
fn wgsl_drop_shadow_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::drop_shadow::DropShadow;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut d = DropShadow::read(Params::EMPTY);
        d.distance = 6.0;
        d.softness = 3.0;
        d
    };
    let op_of = |d: DropShadow| {
        let p = d.packed();
        DropShadowOp {
            colour: p.colour,
            opacity: p.opacity,
            offset: p.offset,
            softness_px: p.softness_px,
            shadow_only: p.shadow_only,
            mix: p.mix,
            spread_scale: p.spread_scale,
            knockout: p.knockout,
            invert: p.invert,
            inner: p.inner,
        }
    };

    let mut hard = base;
    hard.softness = 0.0;
    let mut far = base;
    far.distance = 14.0;
    far.direction = 315.0;
    let mut only = base;
    only.shadow_only = true;
    let mut coloured = base;
    coloured.shadow_colour = [0.2, 0.05, 0.4, 1.0];
    coloured.opacity = 90.0;
    let mut clear = base;
    clear.opacity = 0.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut off = base;
    off.mix = 0.0;

    for (name, d) in [
        ("default", base),
        ("hard", hard),
        ("far", far),
        ("shadow-only", only),
        ("coloured", coloured),
        ("opacity-zero", clear),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::drop_shadow(&mut cpu, w, h, &d.packed());
        let out = fx.drop_shadow(&ctx, &tex, w, h, None, &op_of(d));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("drop_shadow {name}: worst {worst}");
        assert!(worst < 2e-2, "{name}: worst diff {worst}");
        match name {
            "opacity-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the shadow must actually appear"),
        }
        let out2 = fx.drop_shadow(&ctx, &tex, w, h, None, &op_of(d));
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "{name}: the shadow must be bit-stable"
        );
    }

    // The corpus is opaque on the left half and transparent on the right, so the
    // alpha edge runs down the middle. At 135° the shadow falls down-and-right,
    // which puts coverage just right of that edge.
    let out = fx.drop_shadow(&ctx, &tex, w, h, None, &op_of(base));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    let alpha_at = |x: u32, y: u32| gpu[((y * w + x) * 4 + 3) as usize];
    assert!(
        alpha_at(w / 2 + 3, 14) > 0.05,
        "the shadow must fall to the right of the edge"
    );
    assert!(
        alpha_at(w / 2 - 3, 14) >= 1.0,
        "inside the shape the layer itself is still opaque"
    );

    // Shadow only keeps the shadow and nothing else: the shape's own colour is
    // gone from the middle of the opaque half.
    let only_out = fx.drop_shadow(&ctx, &tex, w, h, None, &op_of(only));
    let only_px = readback_linear_f32(&ctx, &only_out, w, h).unwrap();
    let i = ((14 * w + 6) * 4) as usize;
    assert!(
        only_px[i] < img[i] || only_px[i + 1] < img[i + 1],
        "Shadow only must remove the layer's own colour"
    );
}

/// The §1.6 oracle for Set matte (docs/08 §3.44): a pointwise effect, so fp16
/// ULPs on the alpha corpus — and the partial-alpha pixels in it are the point,
/// since the whole effect is an unpremultiply/re-premultiply round trip.
///
/// Three claims beyond parity, because this effect's matte *is* its output.
/// **The alpha becomes the matte's chosen channel**, exactly. **Invert reads it
/// the other way round.** And **an unbound matte is the bit-exact identity** —
/// the labelled no-op every layer-input effect follows (K-258).
#[test]
fn wgsl_set_matte_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // A matte with a different value in every channel, so the Channel row can be
    // told apart: red ramps across, green down, blue is flat, alpha is a block.
    let mut matte = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            matte[i] = x as f32 / (w - 1) as f32;
            matte[i + 1] = y as f32 / (h - 1) as f32;
            matte[i + 2] = 0.25;
            matte[i + 3] = f32::from(x > 8 && x < 24 && y > 6 && y < 18);
        }
    }
    let matte: Vec<f32> = matte.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    let matte_tex = upload_linear_f32(&ctx, &matte, w, h);

    for (name, channel, invert, combine, mix) in [
        ("luma", 0u32, false, false, 1.0f32),
        ("alpha", 1, false, false, 1.0),
        ("red", 2, false, false, 1.0),
        ("green", 3, false, false, 1.0),
        ("blue", 4, false, false, 1.0),
        ("inverted", 2, true, false, 1.0),
        ("combined", 2, false, true, 1.0),
        ("combined-inverted", 0, true, true, 1.0),
        ("mixed", 2, false, false, 0.6),
        ("mix-zero", 2, false, false, 0.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::set_matte(&mut cpu, &matte, channel, invert, combine, mix);
        let op = SetMatteOp {
            channel,
            combine,
            invert,
            mix,
        };
        let out = fx.set_matte(&ctx, &tex, w, h, Some(&matte_tex), &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("set_matte {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }
    }

    // **The alpha IS the matte's channel.** Red at full mix, no combine: every
    // output alpha is the matte's red, to fp16.
    let out = fx.set_matte(
        &ctx,
        &tex,
        w,
        h,
        Some(&matte_tex),
        &SetMatteOp {
            channel: 2,
            combine: false,
            invert: false,
            mix: 1.0,
        },
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    for i in (0..gpu.len()).step_by(4) {
        assert!(
            (gpu[i + 3] - matte[i]).abs() < 1e-3,
            "alpha at {} is {} not {}",
            i / 4,
            gpu[i + 3],
            matte[i]
        );
    }

    // **An unbound matte is the bit-exact identity**, whatever the other rows say.
    let none = fx.set_matte(
        &ctx,
        &tex,
        w,
        h,
        None,
        &SetMatteOp {
            channel: 4,
            combine: true,
            invert: true,
            mix: 1.0,
        },
    );
    assert_eq!(
        readback_linear_f32(&ctx, &none, w, h).unwrap(),
        img,
        "an unset Matte row must render exactly the input"
    );
}

/// The §1.6 oracle for Set channels (docs/08 §3.94): a pointwise effect, so
/// fp16 ULPs on the alpha corpus — and its partial-alpha pixels are the point,
/// since every pick is read through an unpremultiply/re-premultiply round trip.
///
/// Three claims beyond parity. **A pick fetches the channel it names**, from
/// the picture it names. **An unbound Source row is not a passthrough** — the
/// four `This layer` picks still shuffle, which is what separates this effect
/// from Set matte — but every `Source …` pick then reads zero. And **Mix 0 is
/// the bit-exact identity**.
#[test]
fn wgsl_set_channels_matches_the_cpu_oracle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // A source with a different value in every channel, so a pick can be told
    // apart: red ramps across, green down, blue is flat, alpha is a block. It
    // is premultiplied, as a rendered layer is, so the kernel and the oracle
    // both have to undo that before reading a channel.
    let mut source = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = if x > 8 && x < 24 && y > 6 && y < 18 {
                1.0
            } else {
                0.5
            };
            source[i] = (x as f32 / (w - 1) as f32) * a;
            source[i + 1] = (y as f32 / (h - 1) as f32) * a;
            source[i + 2] = 0.25 * a;
            source[i + 3] = a;
        }
    }
    let source: Vec<f32> = source.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    let source_tex = upload_linear_f32(&ctx, &source, w, h);

    for (name, picks, mix) in [
        ("identity", [0u32, 1, 2, 3], 1.0f32),
        ("swap-rb", [2, 1, 0, 3], 1.0),
        ("luma-to-alpha", [0, 1, 2, 4], 1.0),
        ("source-rgb", [5, 6, 7, 3], 1.0),
        ("source-alpha", [0, 1, 2, 8], 1.0),
        ("source-luma-to-alpha", [0, 1, 2, 9], 1.0),
        ("all-from-source", [5, 6, 7, 8], 1.0),
        ("full-on-off", [10, 11, 10, 11], 1.0),
        ("mixed", [2, 1, 0, 9], 0.6),
        ("mix-zero", [11, 11, 11, 11], 0.0),
    ] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::set_channels(&mut cpu, &source, picks, mix);
        let out = fx.set_channels(
            &ctx,
            &tex,
            w,
            h,
            Some(&source_tex),
            &SetChannelsOp { picks, mix },
        );
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("set_channels {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }
    }

    // **A pick fetches what it names.** Alpha from the source's straight red,
    // at full mix: every output alpha is that number, to fp16.
    let out = fx.set_channels(
        &ctx,
        &tex,
        w,
        h,
        Some(&source_tex),
        &SetChannelsOp {
            picks: [0, 1, 2, 5],
            mix: 1.0,
        },
    );
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    for i in (0..gpu.len()).step_by(4) {
        let want = source[i] / source[i + 3];
        assert!(
            (gpu[i + 3] - want).abs() < 1e-3,
            "alpha at {} is {} not {want}",
            i / 4,
            gpu[i + 3]
        );
    }

    // **An unbound Source row still shuffles this layer**, and reads the source
    // as zero — the two halves of the row's documented no-source behaviour.
    let mut cpu = img.clone();
    lumit_core::fx::cpu::set_channels(&mut cpu, &[], [2, 1, 0, 8], 1.0);
    let none = fx.set_channels(
        &ctx,
        &tex,
        w,
        h,
        None,
        &SetChannelsOp {
            picks: [2, 1, 0, 8],
            mix: 1.0,
        },
    );
    let gpu = readback_linear_f32(&ctx, &none, w, h).unwrap();
    assert!(
        worst_f16_ulp(&cpu, &gpu) <= 2,
        "an unset Source row must still shuffle this layer's own channels"
    );
    for px in gpu.chunks_exact(4) {
        assert_eq!(px[3], 0.0, "a Source pick with no layer bound reads zero");
    }
}

/// The §1.6 oracle for Linear wipe (docs/08 §3.46), on the smooth corpus and by
/// absolute difference.
///
/// The metric is K-399's, extended one step. That entry's rule is about a kernel
/// whose real output is a *sample position*; this kernel's real output is a
/// *threshold on a position*, and it magnifies a last-bit disagreement the same
/// way — the signed distance is a dot product, exactly the expression one path
/// fuses into a multiply-add and the other does not, and dividing it by a
/// feather narrower than a pixel turns a difference of 10⁻⁶ into a visible one
/// at the edge. So the corpus has no step in it and the tolerance is stated in
/// the picture.
///
/// Three claims beyond parity: Completion 0 is the exact identity, Completion
/// 100 is an exactly empty frame, and at the default 90° angle the left half is
/// gone while the right half is untouched — the direction convention, which no
/// parity check can establish.
#[test]
fn wgsl_linear_wipe_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::linear_wipe::LinearWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut l = LinearWipe::read(Params::EMPTY);
        l.centre_x = w as f32 * 0.5;
        l.centre_y = h as f32 * 0.5;
        l
    };
    let op_of = |l: LinearWipe| {
        let p = l.packed();
        LinearWipeOp {
            centre: p.centre,
            normal: p.normal,
            completion: p.completion,
            band: p.band,
            mix: p.mix,
        }
    };

    let mut feathered = base;
    feathered.feather = 8.0;
    let mut turned = base;
    turned.angle = 30.0;
    turned.feather = 5.0;
    let mut down = base;
    down.angle = 0.0;
    let mut off_centre = base;
    off_centre.centre_x = 4.0;
    off_centre.feather = 3.0;
    let mut nothing = base;
    nothing.completion = 0.0;
    let mut everything = base;
    everything.completion = 100.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, l) in [
        ("default", base),
        ("feathered", feathered),
        ("turned", turned),
        ("downward", down),
        ("off-centre", off_centre),
        ("completion-zero", nothing),
        ("completion-full", everything),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = l.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::linear_wipe(&mut cpu, w, h, &p);
        let out = fx.linear_wipe(&ctx, &tex, w, h, None, &op_of(l));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("linear_wipe {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "completion-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            "completion-full" => {
                assert!(
                    gpu.iter().all(|v| *v == 0.0),
                    "{name}: the frame must be exactly empty"
                );
            }
            _ => assert!(gpu != img, "{name}: the wipe must remove something"),
        }
    }

    // **Which side goes first.** At the default 90° the edge is vertical and the
    // LEFT half is removed; the right half is untouched to the bit.
    let out = fx.linear_wipe(&ctx, &tex, w, h, None, &op_of(base));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if x < w / 2 - 1 {
                assert_eq!(gpu[i + 3], 0.0, "the left half must be gone at {x},{y}");
            } else if x > w / 2 {
                assert_eq!(
                    gpu[i..i + 4],
                    img[i..i + 4],
                    "the right half must be untouched at {x},{y}"
                );
            }
        }
    }
}

/// The §1.6 oracle for Radial wipe (docs/08 §3.47), on the smooth corpus and by
/// absolute difference — [`wgsl_linear_wipe_matches_the_cpu_oracle`]'s reasoning
/// with an `atan2` in front of it (§3.42's admission, K-399).
///
/// Four claims beyond parity: Completion 0 and 100 are the exact identity and
/// the exactly empty frame; **Clockwise and Anticlockwise remove opposite sides**
/// of the start ray, which is what says the one-expression form actually reads
/// the direction; and **Both is symmetric about it**, which neither single
/// direction is.
#[test]
fn wgsl_radial_wipe_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::radial_wipe::RadialWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut r = RadialWipe::read(Params::EMPTY);
        r.centre_x = w as f32 * 0.5;
        r.centre_y = h as f32 * 0.5;
        r
    };
    let op_of = |r: RadialWipe| {
        let p = r.packed();
        RadialWipeOp {
            centre: p.centre,
            start: p.start,
            dir: p.dir,
            completion: p.completion,
            feather: p.feather,
            mix: p.mix,
        }
    };

    let mut anti = base;
    anti.wipe = 1;
    let mut both = base;
    both.wipe = 2;
    let mut feathered = base;
    feathered.feather = 6.0;
    let mut started = base;
    started.start_angle = 120.0;
    let mut quarter = base;
    quarter.completion = 25.0;
    let mut nothing = base;
    nothing.completion = 0.0;
    let mut everything = base;
    everything.completion = 100.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, r) in [
        ("clockwise", base),
        ("anticlockwise", anti),
        ("both", both),
        ("feathered", feathered),
        ("started", started),
        ("quarter", quarter),
        ("completion-zero", nothing),
        ("completion-full", everything),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = r.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::radial_wipe(&mut cpu, w, h, &p);
        let out = fx.radial_wipe(&ctx, &tex, w, h, None, &op_of(r));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("radial_wipe {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "completion-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            "completion-full" => {
                assert!(
                    gpu.iter().all(|v| *v == 0.0),
                    "{name}: the frame must be exactly empty"
                );
            }
            _ => assert!(gpu != img, "{name}: the wipe must remove something"),
        }
    }

    // **The three directions really differ.** At Completion 50 from straight up,
    // Clockwise takes the right of the frame and Anticlockwise the left; Both
    // takes the top and leaves the bottom, symmetric about the vertical.
    let read = |r: RadialWipe| {
        let out = fx.radial_wipe(&ctx, &tex, w, h, None, &op_of(r));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };
    let alpha = |px: &[f32], x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
    let cw = read(base);
    let ccw = read(anti);
    let bo = read(both);
    assert_eq!(alpha(&cw, w - 4, 4), 0.0, "clockwise must take the right");
    assert!(alpha(&cw, 3, 20) > 0.0, "clockwise must leave the left");
    assert_eq!(alpha(&ccw, 3, 4), 0.0, "anticlockwise must take the left");
    assert!(
        alpha(&ccw, w - 4, 20) > 0.0,
        "anticlockwise must leave the right"
    );
    assert_eq!(alpha(&bo, w / 2 - 6, 2), 0.0, "both must take the top left");
    assert_eq!(
        alpha(&bo, w / 2 + 6, 2),
        0.0,
        "both must take the top right"
    );
    assert!(alpha(&bo, w / 2, h - 2) > 0.0, "both must leave the bottom");
}

/// The §1.6 oracle for Venetian blinds (docs/08 §3.70), on the smooth corpus and
/// by absolute difference — [`wgsl_linear_wipe_matches_the_cpu_oracle`]'s
/// reasoning, since this is that kernel with the distance folded into a slat.
///
/// Three claims beyond parity: Completion 0 is the exact identity and 100 the
/// exactly empty frame, and **the slats are really a rank** — at the default 0°
/// every column of the frame reads the same and the rows alternate, which a
/// single edge could not produce.
#[test]
fn wgsl_venetian_blinds_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::venetian_blinds::VenetianBlinds;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = VenetianBlinds::read(Params::EMPTY);
    let op_of = |v: VenetianBlinds| {
        let p = v.packed();
        VenetianBlindsOp {
            normal: p.normal,
            period: p.period,
            completion: p.completion,
            band: p.band,
            mix: p.mix,
        }
    };

    let mut wide = base;
    wide.width = 8.0;
    let mut feathered = base;
    feathered.width = 10.0;
    feathered.feather = 3.0;
    let mut turned = base;
    turned.direction = 35.0;
    turned.width = 9.0;
    let mut vertical = base;
    vertical.direction = 90.0;
    vertical.width = 6.0;
    let mut nothing = base;
    nothing.completion = 0.0;
    let mut everything = base;
    everything.completion = 100.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, v) in [
        ("default", base),
        ("wide", wide),
        ("feathered", feathered),
        ("turned", turned),
        ("vertical", vertical),
        ("completion-zero", nothing),
        ("completion-full", everything),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = v.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::venetian_blinds(&mut cpu, w, h, &p);
        let out = fx.venetian_blinds(&ctx, &tex, w, h, None, &op_of(v));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("venetian_blinds {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "completion-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            "completion-full" => {
                assert!(
                    gpu.iter().all(|v| *v == 0.0),
                    "{name}: the frame must be exactly empty"
                );
            }
            _ => assert!(gpu != img, "{name}: the blinds must remove something"),
        }
    }

    // **A rank, not one edge.** At Direction 0 the coverage depends only on the
    // row, and over an 8-pixel slat there must be rows that are gone and rows
    // that are whole — a single wipe edge has exactly one crossing.
    let out = fx.venetian_blinds(&ctx, &tex, w, h, None, &op_of(wide));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    let alpha = |y: u32| gpu[((y * w + 5) * 4 + 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4 + 3) as usize;
            assert!(
                (gpu[i] / img[i] - alpha(y) / img[((y * w + 5) * 4 + 3) as usize]).abs() < 2e-3,
                "the coverage must depend only on the row at {x},{y}"
            );
        }
    }
    let crossings = (1..h)
        .filter(|y| (alpha(*y) == 0.0) != (alpha(y - 1) == 0.0))
        .count();
    assert!(
        crossings >= 4,
        "an 8-pixel slat across 24 rows must open and close several times, saw {crossings}"
    );
}

/// The §1.6 oracle for Iris wipe (docs/08 §3.71), on the smooth corpus and by
/// absolute difference — [`wgsl_radial_wipe_matches_the_cpu_oracle`]'s reasoning,
/// `atan2` and all.
///
/// Four claims beyond parity: Outer radius 0 is the exact identity; **the middle
/// is removed and the corners are kept**, which says the sign of the distance is
/// the right way round; **more points means more area**, since a hexagon
/// inscribed in a circle covers less of it than a 32-gon does; and **a star
/// removes less than the polygon it came from**, which is what the inner radius
/// is for and is invisible to any parity check.
#[test]
fn wgsl_iris_wipe_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::iris_wipe::IrisWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut i = IrisWipe::read(Params::EMPTY);
        i.centre_x = w as f32 * 0.5;
        i.centre_y = h as f32 * 0.5;
        // The schema's radius is a per cent of the comp diagonal; the resolve
        // step has already turned it into raster pixels by the time `packed`
        // sees it, so the test writes pixels directly.
        i.outer_radius = 8.0;
        i.inner_radius = 4.0;
        i
    };
    let op_of = |i: IrisWipe| {
        let p = i.packed();
        IrisWipeOp {
            centre: p.centre,
            vertex: p.vertex,
            normal: p.normal,
            period: p.period,
            rotation: p.rotation,
            band: p.band,
            active: p.active,
            mix: p.mix,
        }
    };

    let mut many = base;
    many.points = 32;
    let mut star = base;
    star.use_inner_radius = true;
    let mut turned = base;
    turned.rotation = 25.0;
    let mut feathered = base;
    feathered.feather = 4.0;
    let mut off_centre = base;
    off_centre.centre_x = 6.0;
    let mut nothing = base;
    nothing.outer_radius = 0.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, i) in [
        ("default", base),
        ("many-points", many),
        ("star", star),
        ("turned", turned),
        ("feathered", feathered),
        ("off-centre", off_centre),
        ("radius-zero", nothing),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = i.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::iris_wipe(&mut cpu, w, h, &p);
        let out = fx.iris_wipe(&ctx, &tex, w, h, None, &op_of(i));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("iris_wipe {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "radius-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the iris must remove something"),
        }
    }

    // **Inside gone, outside kept** — the sign of the distance, which no parity
    // check can establish.
    let removed = |i: IrisWipe| {
        let out = fx.iris_wipe(&ctx, &tex, w, h, None, &op_of(i));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        (
            gpu[((h / 2 * w + w / 2) * 4 + 3) as usize],
            gpu[3],
            gpu.iter().skip(3).step_by(4).filter(|a| **a == 0.0).count(),
        )
    };
    let (middle, corner, hexagon) = removed(base);
    assert_eq!(middle, 0.0, "the middle must be gone");
    assert_eq!(corner, img[3], "the corner must be untouched");

    // **More points, more area**, and **a star removes less than its polygon**.
    let (_, _, thirty_two) = removed(many);
    let (_, _, starred) = removed(star);
    assert!(
        thirty_two > hexagon,
        "a 32-gon must cover more than a hexagon of the same radius ({thirty_two} vs {hexagon})"
    );
    assert!(
        starred < hexagon,
        "pulling every other corner in must remove less ({starred} vs {hexagon})"
    );
}

/// The §1.6 oracle for Card wipe (docs/08 §3.72), on the smooth corpus and by
/// absolute difference: this kernel's real output is a **sample position** taken
/// through a projective divide, which is exactly the expression one path fuses
/// into a multiply-add and the other does not (K-399, the distort batch's
/// metric).
///
/// Five claims beyond parity, none of which a parity check could reach.
/// Completion 0 is the exact identity and 100 the exactly empty frame — the two
/// clamped ends, tested for rather than arrived at through a cosine. **The grid
/// is really a grid**: at a mid Completion with a narrow Transition width, some
/// cards are whole and others are gone. **Flip order reverses**: Left to right
/// and Right to left must empty opposite sides. And **a card never bleeds into
/// its neighbour** — a card that has not started must be the input to the bit,
/// including the pixels along its edges.
#[test]
fn wgsl_card_wipe_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::card_wipe::CardWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = {
        let mut c = CardWipe::read(Params::EMPTY);
        c.seed = 12_345;
        // Transition width is px@comp (K-558) and the declared default is half
        // a nominal 1080p frame, which on a 32-pixel corpus would clamp to the
        // whole frame and flatten the ramp this oracle exists to check. Half
        // of *this* raster is what the default means, so that is what it gets.
        c.transition_width = w as f32 * 0.5;
        c
    };
    let op_of = |c: CardWipe| {
        // Transition width is px@comp (K-558), so `packed` takes the raster it
        // is being drawn on to turn the band into a share of the frame.
        let p = c.packed(w as f32, h as f32);
        CardWipeOp {
            grid: p.grid,
            completion: p.completion,
            inv_width: p.inv_width,
            one_minus_width: p.one_minus_width,
            order_axis: p.order_axis,
            order_bias: p.order_bias,
            order_scale: p.order_scale,
            axis: p.axis,
            direction: p.direction,
            randomness: p.randomness,
            seed: p.seed,
            mix: p.mix,
        }
    };
    let read = |c: CardWipe| {
        let out = fx.card_wipe(&ctx, &tex, w, h, None, &op_of(c));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };

    let mut vertical = base;
    vertical.flip_axis = 1;
    let mut backwards = base;
    backwards.flip_direction = 1;
    let mut mixed_axes = base;
    mixed_axes.flip_axis = 2;
    mixed_axes.flip_direction = 2;
    let mut shuffled = base;
    shuffled.randomness = 100.0;
    let mut rightward = base;
    rightward.flip_order = 1;
    rightward.transition_width = w as f32 * 0.1;
    let mut leftward = rightward;
    leftward.flip_order = 0;
    let mut downward = base;
    downward.flip_order = 2;
    let mut upward = base;
    upward.flip_order = 3;
    let mut dense = base;
    dense.rows = 3;
    dense.columns = 5;
    let mut nothing = base;
    nothing.completion = 0.0;
    let mut everything = base;
    everything.completion = 100.0;
    let mut faded = base;
    faded.mix = 60.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, c) in [
        ("default", base),
        ("vertical", vertical),
        ("backwards", backwards),
        ("mixed-axes", mixed_axes),
        ("shuffled", shuffled),
        ("rightward", rightward),
        ("downward", downward),
        ("upward", upward),
        ("dense", dense),
        ("completion-zero", nothing),
        ("completion-full", everything),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = c.packed(w as f32, h as f32);
        let mut cpu = img.clone();
        lumit_core::fx::cpu::card_wipe(&mut cpu, w, h, &p);
        let gpu = read(c);
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("card_wipe {name}: worst {worst}");
        assert!(worst < 4e-3, "{name}: worst diff {worst}");
        match name {
            "completion-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            "completion-full" => {
                assert!(
                    gpu.iter().all(|v| *v == 0.0),
                    "{name}: the frame must be exactly empty"
                );
            }
            _ => assert!(gpu != img, "{name}: the cards must turn"),
        }
    }

    // **A grid, and a card that has not started is untouched to the bit.** With
    // a narrow Transition width and Left to right, the leftmost column is gone
    // and the rightmost has not moved at all — every pixel of it, edges
    // included, which is what says no card reads outside its own cell.
    let gpu = read(leftward);
    let alpha = |px: &[f32], x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
    for y in 0..h {
        for x in (w - 4)..w {
            let i = ((y * w + x) * 4) as usize;
            assert_eq!(
                gpu[i..i + 4],
                img[i..i + 4],
                "the last column must be untouched at {x},{y}"
            );
        }
        assert_eq!(
            alpha(&gpu, 1, y),
            0.0,
            "the first column must be gone at {y}"
        );
    }
    // **Flip order reverses.** The same settings the other way round empty the
    // right-hand side and leave the left.
    let other = read(rightward);
    for y in 0..h {
        assert_eq!(
            alpha(&other, w - 2, y),
            0.0,
            "right to left must take the right at {y}"
        );
        let i = ((y * w + 1) * 4) as usize;
        assert_eq!(
            other[i..i + 4],
            img[i..i + 4],
            "right to left must leave the left at {y}"
        );
    }
}

/// The §1.6 oracle for Corner pin (docs/08 §3.48), on the smooth corpus and by
/// absolute difference, for [`smooth_corpus`]'s reason: this kernel's real output
/// is a sample position, and a perspective divide contracted differently on the
/// two paths moves it in its last bits.
///
/// Three claims beyond parity, each a way the kernel could be wrong while
/// agreeing with a wrong oracle. **The frame's own corners return the picture**,
/// so the homography derivation is not silently transposed. **A degenerate quad
/// renders the input**, which is the short-circuit `packed` exists for. And **a
/// keystone genuinely converges**: the pinned picture must be narrower at the top
/// than at the bottom, which a merely affine map could not produce.
#[test]
fn wgsl_corner_pin_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::corner_pin::CornerPin;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // The declared points are px@comp; the resolve step would have scaled them to
    // this 32x24 raster, so the test does the same by hand.
    let quad = |ul: [f32; 2], ur: [f32; 2], ll: [f32; 2], lr: [f32; 2]| {
        let mut c = CornerPin::read(Params::EMPTY);
        c.upper_left_x = ul[0];
        c.upper_left_y = ul[1];
        c.upper_right_x = ur[0];
        c.upper_right_y = ur[1];
        c.lower_left_x = ll[0];
        c.lower_left_y = ll[1];
        c.lower_right_x = lr[0];
        c.lower_right_y = lr[1];
        c
    };
    let identity = quad([0.0, 0.0], [32.0, 0.0], [0.0, 24.0], [32.0, 24.0]);
    let keystone = quad([8.0, 2.0], [24.0, 2.0], [0.0, 22.0], [32.0, 22.0]);
    let mut repeated = keystone;
    repeated.edge = 1;
    let mut mirrored = keystone;
    mirrored.edge = 2;
    let leaning = quad([2.0, 6.0], [30.0, 0.0], [4.0, 20.0], [28.0, 24.0]);
    // Two corners crossed: part of the frame is behind the horizon.
    let crossed = quad([28.0, 2.0], [4.0, 2.0], [0.0, 22.0], [32.0, 22.0]);
    // Three corners in a line: no map at all.
    let degenerate = quad([0.0, 0.0], [16.0, 0.0], [32.0, 0.0], [32.0, 24.0]);
    let mut faded = keystone;
    faded.mix = 50.0;
    let mut off = keystone;
    off.mix = 0.0;

    let op_of = |c: CornerPin| {
        let p = c.packed();
        CornerPinOp {
            inv: p.inv,
            active: p.active,
            edge: p.edge,
            mix: p.mix,
        }
    };
    for (name, c) in [
        ("identity", identity),
        ("keystone", keystone),
        ("keystone-repeat", repeated),
        ("keystone-mirror", mirrored),
        ("leaning", leaning),
        ("crossed", crossed),
        ("degenerate", degenerate),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = c.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::corner_pin(&mut cpu, w, h, &p);
        let out = fx.corner_pin(&ctx, &tex, w, h, None, &op_of(c));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("corner_pin {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");

        match name {
            "degenerate" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            // Not asserted bit-exact: the map IS the frame's own corners, but the
            // sample still travels through a perspective divide, so it lands
            // within an ulp of the pixel's centre rather than on it.
            "identity" => assert!(
                worst_diff(&gpu, &img) < 1e-3,
                "{name}: must return the picture"
            ),
            _ => assert!(gpu != img, "{name}: the pin must actually move something"),
        }

        let out2 = fx.corner_pin(&ctx, &tex, w, h, None, &op_of(c));
        assert_eq!(
            gpu,
            readback_linear_f32(&ctx, &out2, w, h).unwrap(),
            "{name}: the pin must be bit-stable"
        );
    }

    // **A keystone converges.** The pinned picture's covered span is narrower at
    // the top than at the bottom — the projective part doing its work, which an
    // affine map cannot do.
    let out = fx.corner_pin(&ctx, &tex, w, h, None, &op_of(keystone));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    let span = |y: u32| -> u32 {
        (0..w)
            .filter(|x| gpu[((y * w + x) * 4 + 3) as usize] > 0.0)
            .count() as u32
    };
    let (top, bottom) = (span(4), span(20));
    eprintln!("corner pin span: top {top}, bottom {bottom}");
    assert!(top > 0 && bottom > top, "the keystone must converge upward");
}

/// The §1.6 oracle for Displacement map (docs/08 §3.49), and with it the claim
/// that the K-395 override was worth taking: **the matte is the map**.
///
/// Parity first, over every channel picker, both signs, all three edge policies
/// and both Invert settings. Then three claims a passing parity check would not
/// make. **An unbound map is the exact identity** — the labelled no-op. **A flat
/// mid-grey map moves nothing**, which is what pins the ½ neutral. And **the push
/// follows the map's own value**: under a flat map at a known level a lit pixel
/// must land exactly where `(k − ½)·2·Amount` says, sign included, which a
/// strength dissolve could never produce.
#[test]
fn wgsl_displacement_map_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::displacement_map::DisplacementMap;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // A map with a different gradient in every channel, so the channel pickers
    // are actually distinguishable from one another.
    let mut map = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let u = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            map[i] = u;
            map[i + 1] = v;
            map[i + 2] = 1.0 - u;
            map[i + 3] = 0.25 + 0.5 * v;
        }
    }
    let qmap: Vec<f32> = map.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    let map_tex = upload_linear_f32(&ctx, &map, w, h);

    let base = {
        let mut d = DisplacementMap::read(Params::EMPTY);
        d.horizontal_amount = 5.0;
        d.vertical_amount = 4.0;
        d
    };
    let op_of = |d: DisplacementMap, invert: bool| {
        let p = d.packed();
        DisplacementMapOp {
            channels: p.channels,
            amount: p.amount,
            edge: p.edge,
            mix: p.mix,
            matte_invert: invert,
        }
    };

    let mut luma = base;
    luma.horizontal_channel = 0;
    luma.vertical_channel = 0;
    let mut alpha = base;
    alpha.horizontal_channel = 1;
    alpha.vertical_channel = 1;
    let mut blue = base;
    blue.horizontal_channel = 4;
    blue.vertical_channel = 4;
    let mut negative = base;
    negative.horizontal_amount = -5.0;
    negative.vertical_amount = -4.0;
    let mut transparent = base;
    transparent.edge = 0;
    let mut mirrored = base;
    mirrored.edge = 2;
    let mut still = base;
    still.horizontal_amount = 0.0;
    still.vertical_amount = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut off = base;
    off.mix = 0.0;

    for (name, d) in [
        ("red-green", base),
        ("luma", luma),
        ("alpha", alpha),
        ("blue", blue),
        ("negative", negative),
        ("transparent-edges", transparent),
        ("mirrored-edges", mirrored),
        ("amount-zero", still),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        for invert in [false, true] {
            let p = d.packed();
            let mut cpu = img.clone();
            lumit_core::fx::cpu::displacement_map(&mut cpu, w, h, &p, &qmap, invert);
            let out = fx.displacement_map(&ctx, &tex, w, h, Some(&map_tex), &op_of(d, invert));
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let worst = worst_diff(&cpu, &gpu);
            eprintln!("displacement_map {name} invert {invert}: worst {worst}");
            assert!(worst < 2e-3, "{name} invert {invert}: worst diff {worst}");
            match name {
                "amount-zero" | "mix-zero" => {
                    assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
                }
                _ => assert!(gpu != img, "{name}: the map must actually push something"),
            }
        }
    }

    // **No map bound is the exact identity** — the labelled no-op §1.2 sanctions.
    let out = fx.displacement_map(&ctx, &tex, w, h, None, &op_of(base, false));
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "an unbound map must render the input untouched"
    );

    // **Mid-grey is the neutral.** A flat 0.5 map moves nothing at any Amount,
    // which is what makes one map able to push both ways.
    let grey: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [0.5f32, 0.5, 0.5, 1.0])
        .collect();
    let grey_tex = upload_linear_f32(&ctx, &grey, w, h);
    let out = fx.displacement_map(&ctx, &tex, w, h, Some(&grey_tex), &op_of(base, false));
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "a mid-grey map must move nothing"
    );

    // **The push follows the map.** One lit pixel and a flat map at a known
    // level: the light must land exactly where the formula says. Measured on the
    // CPU reference, which the parity sweep above has already tied the kernel to
    // — the question here is what the arithmetic *means*.
    let travel = |k: f32| -> f32 {
        let mut dot = vec![0.0f32; (w * h * 4) as usize];
        let src = ((12 * w + 16) * 4) as usize;
        dot[src..src + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        let flat: Vec<f32> = (0..(w * h) as usize)
            .flat_map(|_| [k, 0.5, k, 1.0])
            .collect();
        let mut d = DisplacementMap::read(Params::EMPTY);
        d.horizontal_amount = 6.0;
        d.vertical_amount = 6.0;
        lumit_core::fx::cpu::displacement_map(&mut dot, w, h, &d.packed(), &flat, false);
        let (mut sx, mut sw) = (0.0f32, 0.0f32);
        for y in 0..h {
            for x in 0..w {
                let v = dot[((y * w + x) * 4) as usize];
                sx += v * x as f32;
                sw += v;
            }
        }
        if sw <= 0.0 {
            return 0.0;
        }
        sx / sw - 16.0
    };
    // A white map says "read from 6 px to the right", so the light arrives 6 px
    // to the LEFT of where it was: a displacement map moves the *source*, not the
    // pixel. The sign is what a reader of this test most needs pinned.
    let white = travel(1.0);
    let black = travel(0.0);
    eprintln!("displacement travel: white {white}, black {black}");
    assert!(
        (white + 6.0).abs() < 0.2,
        "a white map must pull the light 6 px: {white}"
    );
    assert!(
        (black - 6.0).abs() < 0.2,
        "a black map must push it the other way: {black}"
    );
}

/// The §1.6 oracle for Polar coordinates (docs/08 §3.50), on the smooth corpus
/// and by absolute difference — §3.42's fourth note and K-399's rule: three
/// transcendentals a pixel, none of them liftable host-side.
///
/// Two claims beyond parity. **Interpolation 0 is the exact identity**, which is
/// what makes it a morph rather than a dissolve. And **the two directions invert
/// one another**: a Rectangular to polar followed by a Polar to rectangular
/// returns the picture away from the singular middle.
#[test]
fn wgsl_polar_coordinates_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::polar_coordinates::PolarCoordinates;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = PolarCoordinates::read(Params::EMPTY);
    let mut back = base;
    back.conversion = 1;
    let mut half = base;
    half.interpolation = 50.0;
    let mut none = base;
    none.interpolation = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut off = base;
    off.mix = 0.0;

    for (name, c) in [
        ("to-polar", base),
        ("to-rect", back),
        ("half-bent", half),
        ("interp-zero", none),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = c.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::polar_coordinates(&mut cpu, w, h, &p);
        let out = fx.polar_coordinates(&ctx, &tex, w, h, p.to_polar, p.interp, p.mix);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("polar_coordinates {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "interp-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the bend must actually move something"),
        }
    }

    // **The two directions invert one another.** Bend the frame into a circle and
    // unroll it again; away from the very centre — where a whole ring collapses
    // onto one pixel and no resampler can put it back — the picture returns. The
    // tolerance is a two-tap resampling one.
    let mut there = img.clone();
    lumit_core::fx::cpu::polar_coordinates(&mut there, w, h, &base.packed());
    lumit_core::fx::cpu::polar_coordinates(&mut there, w, h, &back.packed());
    // Only where the round trip is *defined*: the unrolling step reads the
    // intermediate at radius `py ÷ H · R`, and R is half the diagonal, so beyond
    // the frame's own edge there is nothing to read (§3.50 — those pixels are
    // transparent by design). The middle few pixels are excluded too: there a
    // whole ring collapses onto one texel and no resampler can put it back.
    let radius = 0.5 * ((w * w + h * h) as f32).sqrt();
    let mut worst = 0.0f32;
    let mut counted = 0u32;
    for y in 0..h {
        for x in 0..w {
            let theta = (x as f32 + 0.5) / w as f32 * std::f32::consts::TAU;
            let r = (y as f32 + 0.5) / h as f32 * radius;
            let (qx, qy) = (16.0 + r * theta.sin(), 12.0 - r * theta.cos());
            if r < 5.0 || !(1.0..w as f32 - 1.0).contains(&qx) {
                continue;
            }
            if !(1.0..h as f32 - 1.0).contains(&qy) {
                continue;
            }
            counted += 1;
            let i = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                worst = worst.max((there[i + c] - img[i + c]).abs());
            }
        }
    }
    assert!(
        counted > 100,
        "the round trip must be defined somewhere: {counted}"
    );
    eprintln!("polar round trip: worst {worst}");
    assert!(
        worst < 0.2,
        "the round trip must return the picture: {worst}"
    );
}

/// The §1.6 oracle for Twirl (docs/08 §3.51), on the smooth corpus and by
/// absolute difference — one sine and cosine a pixel, K-399's rule.
///
/// Three claims beyond parity. **Angle 0 and Radius 0 are the exact identity.**
/// **The twirl stays inside its circle**: the frame's corners are bit-identical
/// to the input, which is what says the falloff is bounded rather than merely
/// small. And **the middle turns further than the rim**, which is the squared
/// falloff doing its work.
#[test]
fn wgsl_twirl_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::twirl::Twirl;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Radius is px@comp; the resolve step would have scaled it to raster
    // pixels, so the test hands `packed` the pixels directly.
    let base = {
        let mut t = Twirl::read(Params::EMPTY);
        t.centre_x = 16.0;
        t.centre_y = 12.0;
        t.radius = 10.0;
        t
    };
    let mut hard = base;
    hard.angle = 320.0;
    let mut backwards = base;
    backwards.angle = -140.0;
    let mut wide = base;
    wide.radius = 24.0;
    let mut offcentre = base;
    offcentre.centre_x = 8.0;
    offcentre.centre_y = 18.0;
    let mut straight = base;
    straight.angle = 0.0;
    let mut tiny = base;
    tiny.radius = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut off = base;
    off.mix = 0.0;

    let op_of = |t: Twirl| {
        let p = t.packed();
        TwirlOp {
            centre: p.centre,
            radius: p.radius,
            inv_radius: p.inv_radius,
            angle: p.angle,
            mix: p.mix,
        }
    };
    for (name, t) in [
        ("default", base),
        ("hard", hard),
        ("backwards", backwards),
        ("wide", wide),
        ("off-centre", offcentre),
        ("angle-zero", straight),
        ("radius-zero", tiny),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = t.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::twirl(&mut cpu, w, h, &p);
        let out = fx.twirl(&ctx, &tex, w, h, None, &op_of(t));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("twirl {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "angle-zero" | "radius-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the twirl must actually turn something"),
        }
    }

    // **Nothing outside the circle moves.** The frame's corners sit well beyond a
    // radius of 10 from (16, 12), and must arrive untouched.
    let out = fx.twirl(&ctx, &tex, w, h, None, &op_of(hard));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    for (x, y) in [(0u32, 0u32), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        let i = ((y * w + x) * 4) as usize;
        assert_eq!(gpu[i..i + 4], img[i..i + 4], "the corner at {x},{y} moved");
    }

    // **The middle turns further than the rim.** One lit pixel at two radii, and
    // the inner one must travel further — the squared falloff, measured.
    let travel = |from: u32| -> f32 {
        let mut dot = vec![0.0f32; (w * h * 4) as usize];
        let x0 = 16 + from;
        let src = ((12 * w + x0) * 4) as usize;
        dot[src..src + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        lumit_core::fx::cpu::twirl(&mut dot, w, h, &hard.packed());
        let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
        for y in 0..h {
            for x in 0..w {
                let v = dot[((y * w + x) * 4) as usize];
                sx += v * x as f32;
                sy += v * y as f32;
                sw += v;
            }
        }
        if sw <= 0.0 {
            return 0.0;
        }
        let (cx, cy) = (sx / sw - x0 as f32, sy / sw - 12.0);
        (cx * cx + cy * cy).sqrt()
    };
    let inner = travel(2);
    let outer = travel(8);
    eprintln!("twirl travel: inner {inner}, outer {outer}");
    assert!(inner > 0.5, "the middle must actually turn: {inner}");
    assert!(
        inner > outer,
        "the middle must turn further than the rim: {inner} vs {outer}"
    );
}

/// The §1.6 oracle for Spherize (docs/08 §3.52), on the smooth corpus and by
/// absolute difference — one arc sine or sine a pixel, K-399's rule.
///
/// Three claims beyond parity. **Bulge 0 and Radius 0 are the exact identity.**
/// **The ball stays inside its circle**, so the corners are untouched. And **the
/// two directions invert one another**: a +100 bulge undone by a −100 one returns
/// the picture, which is what makes the pair honest rather than a sign flip on a
/// coefficient.
#[test]
fn wgsl_spherize_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::spherize::Spherize;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Radius is px@comp; see the Twirl oracle's note.
    let base = {
        let mut s = Spherize::read(Params::EMPTY);
        s.centre_x = 16.0;
        s.centre_y = 12.0;
        s.radius = 10.0;
        s
    };
    let mut pinch = base;
    pinch.bulge = -100.0;
    let mut gentle = base;
    gentle.bulge = 40.0;
    let mut wide = base;
    wide.radius = 24.0;
    let mut offcentre = base;
    offcentre.centre_x = 9.0;
    offcentre.centre_y = 17.0;
    let mut flat = base;
    flat.bulge = 0.0;
    let mut tiny = base;
    tiny.radius = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut off = base;
    off.mix = 0.0;

    let op_of = |s: Spherize| {
        let p = s.packed();
        SpherizeOp {
            centre: p.centre,
            radius: p.radius,
            inv_radius: p.inv_radius,
            bulge: p.bulge,
            mix: p.mix,
        }
    };
    for (name, s) in [
        ("bulge", base),
        ("pinch", pinch),
        ("gentle", gentle),
        ("wide", wide),
        ("off-centre", offcentre),
        ("bulge-zero", flat),
        ("radius-zero", tiny),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = s.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::spherize(&mut cpu, w, h, &p);
        let out = fx.spherize(&ctx, &tex, w, h, None, &op_of(s));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("spherize {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "bulge-zero" | "radius-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the ball must actually bend something"),
        }
    }

    // **Nothing outside the ball moves.**
    let out = fx.spherize(&ctx, &tex, w, h, None, &op_of(base));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    for (x, y) in [(0u32, 0u32), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        let i = ((y * w + x) * 4) as usize;
        assert_eq!(gpu[i..i + 4], img[i..i + 4], "the corner at {x},{y} moved");
    }

    // **The bulge and the pinch invert one another.** Not a sign flip on a
    // coefficient: two genuinely inverse maps, so the round trip returns the
    // picture to resampling error. The rim is excluded — that is where an
    // infinite-slope map loses the most to two bilinear taps.
    let mut there = img.clone();
    lumit_core::fx::cpu::spherize(&mut there, w, h, &base.packed());
    lumit_core::fx::cpu::spherize(&mut there, w, h, &pinch.packed());
    let mut worst = 0.0f32;
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 12.0);
            if dx * dx + dy * dy > 64.0 {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                worst = worst.max((there[i + c] - img[i + c]).abs());
            }
        }
    }
    eprintln!("spherize round trip: worst {worst}");
    assert!(
        worst < 0.2,
        "the round trip must return the picture: {worst}"
    );
}

/// The §1.6 oracle for Ripple (docs/08 §3.53), on the smooth corpus and by
/// absolute difference — one sine and cosine a pixel, K-399's rule.
///
/// Three claims beyond parity. **Wave height 0 and Radius 0 are the exact
/// identity.** **The rings stay inside their circle**, so the frame's corners
/// are bit-identical. And **the envelope is zero at the epicentre**, which is
/// what removes the direction singularity there: the pixel the rings spread from
/// does not move at all.
#[test]
fn wgsl_ripple_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::ripple::Ripple;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Radius and the two wave lengths are px@comp; the resolve step would have
    // scaled them to raster pixels, so the test hands `packed` the pixels.
    let base = {
        let mut r = Ripple::read(Params::EMPTY);
        r.centre_x = 16.0;
        r.centre_y = 12.0;
        r.radius = 12.0;
        r.wave_height = 1.5;
        r.wave_width = 5.0;
        r
    };
    let mut symmetric = base;
    symmetric.wave_type = 0;
    let mut turned = base;
    turned.evolution = 140.0;
    let mut tight = base;
    tight.wave_width = 2.0;
    let mut offcentre = base;
    offcentre.centre_x = 9.0;
    offcentre.centre_y = 17.0;
    let mut flat = base;
    flat.wave_height = 0.0;
    let mut tiny = base;
    tiny.radius = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut off = base;
    off.mix = 0.0;

    let op_of = |r: Ripple| {
        let p = r.packed();
        RippleOp {
            centre: p.centre,
            radius: p.radius,
            inv_radius: p.inv_radius,
            amount: p.amount,
            inv_width: p.inv_width,
            turns: p.turns,
            asymmetric: p.asymmetric,
            mix: p.mix,
        }
    };
    for (name, r) in [
        ("asymmetric", base),
        ("symmetric", symmetric),
        ("evolved", turned),
        ("tight", tight),
        ("off-centre", offcentre),
        ("height-zero", flat),
        ("radius-zero", tiny),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = r.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::ripple(&mut cpu, w, h, &p);
        let out = fx.ripple(&ctx, &tex, w, h, None, &op_of(r));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("ripple {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "height-zero" | "radius-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the rings must actually move something"),
        }
    }

    // **Nothing outside the circle moves**, and **the epicentre does not move**.
    // The second is the envelope's whole purpose: rho·(1 − rho)² is zero there,
    // so the one pixel with no radial direction is never asked for one.
    let out = fx.ripple(&ctx, &tex, w, h, None, &op_of(base));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    for (x, y) in [(0u32, 0u32), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        let i = ((y * w + x) * 4) as usize;
        assert_eq!(gpu[i..i + 4], img[i..i + 4], "the corner at {x},{y} moved");
    }
    let moved = |x: u32, y: u32| -> f32 {
        let i = ((y * w + x) * 4) as usize;
        (0..4).fold(0.0f32, |m, c| m.max((gpu[i + c] - img[i + c]).abs()))
    };
    // The pixel on the epicentre against the ring a third of the way out, where
    // the envelope peaks. A ripple with a flat amplitude would move both alike
    // and pinch the middle into a blob.
    let eye = moved(16, 12);
    let ring = moved(16, 8);
    eprintln!("ripple epicentre {eye} vs ring {ring}");
    assert!(
        eye < ring * 0.5,
        "the envelope must die at the epicentre: {eye} vs {ring}"
    );
}

/// The §1.6 oracle for Wave warp (docs/08 §3.54), on the smooth corpus and by
/// absolute difference — K-399's rule.
///
/// Three claims beyond parity. **Wave height 0 is the exact identity.** **Every
/// wave type moves the picture**, so a shape that fell through to a constant
/// would be caught. And **a pinned edge is exactly still**, which is what the
/// per-edge ramp is for: pinning the left edge alone leaves the left column
/// bit-identical and the right one moving.
#[test]
fn wgsl_wave_warp_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::wave_warp::WaveWarp;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Both lengths are px@comp; the resolve step would have scaled them by the
    // preview factor, so the test hands `packed` raster pixels directly.
    let base = {
        let mut v = WaveWarp::read(Params::EMPTY);
        v.wave_height = 3.0;
        v.wave_width = 14.0;
        v
    };
    let op_of = |v: WaveWarp| {
        let p = v.packed();
        WaveWarpOp {
            dir: p.dir,
            perp: p.perp,
            height: p.height,
            inv_width: p.inv_width,
            turns: p.turns,
            shape: p.shape,
            pin: p.pin,
            inv_pin_band: p.inv_pin_band,
            mix: p.mix,
        }
    };
    let mut cases: Vec<(String, WaveWarp)> = Vec::new();
    for (i, name) in ["sine", "square", "triangle", "sawtooth", "circle"]
        .into_iter()
        .enumerate()
    {
        let mut v = base;
        v.wave_type = i as u32;
        cases.push((name.to_owned(), v));
    }
    let mut phased = base;
    phased.phase = 100.0;
    cases.push(("phased".to_owned(), phased));
    let mut angled = base;
    angled.direction = 25.0;
    cases.push(("angled".to_owned(), angled));
    let mut pinned = base;
    pinned.pinning = 1;
    cases.push(("all-edges".to_owned(), pinned));
    let mut left = base;
    left.pinning = 4;
    cases.push(("left-edge".to_owned(), left));
    let mut flat = base;
    flat.wave_height = 0.0;
    cases.push(("height-zero".to_owned(), flat));
    let mut faded = base;
    faded.mix = 50.0;
    cases.push(("mixed".to_owned(), faded));
    let mut off = base;
    off.mix = 0.0;
    cases.push(("mix-zero".to_owned(), off));

    for (name, v) in &cases {
        let p = v.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::wave_warp(&mut cpu, w, h, &p);
        let out = fx.wave_warp(&ctx, &tex, w, h, None, &op_of(*v));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("wave_warp {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name.as_str() {
            "height-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the wave must actually move something"),
        }
    }

    // **Pinning the left edge alone stills the left column and nothing else.**
    let out = fx.wave_warp(&ctx, &tex, w, h, None, &op_of(left));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    let column = |img: &[f32], x: u32| -> Vec<f32> {
        (0..h)
            .flat_map(|y| {
                let i = ((y * w + x) * 4) as usize;
                img[i..i + 4].to_vec()
            })
            .collect()
    };
    assert_eq!(
        column(&gpu, 0),
        column(&img, 0),
        "the pinned left column must be exactly still"
    );
    assert_ne!(
        column(&gpu, w - 1),
        column(&img, w - 1),
        "the unpinned right column must still move"
    );
}

/// The §1.6 oracle for Bezier warp (docs/08 §3.55), on the smooth corpus and by
/// absolute difference — K-399's rule, and a solver rather than a formula, so
/// the two paths agree only if they take the same steps.
///
/// Three claims beyond parity. **The default patch is the bit-exact identity**,
/// which is what §3.55 decision 4's snap is for. **A bent patch actually
/// bends**, and outside it the frame is transparent. And **Quality converges**:
/// one Newton step and twelve agree to a fraction of a pixel on an ordinary
/// warp, which is what says the default 8 is a budget rather than a look.
#[test]
fn wgsl_bezier_warp_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::bezier_warp::BezierWarp;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Every point is px@comp; `instantiate_for_raster` would have put the
    // identity patch on the actual raster, so the test does the same by hand.
    let base = {
        let mut b = BezierWarp::read(Params::EMPTY);
        let (fw, fh) = (w as f32, h as f32);
        b.upper_left_x = 0.0;
        b.upper_left_y = 0.0;
        b.upper_right_x = fw;
        b.upper_right_y = 0.0;
        b.lower_right_x = fw;
        b.lower_right_y = fh;
        b.lower_left_x = 0.0;
        b.lower_left_y = fh;
        b.top_left_tangent_x = fw / 3.0;
        b.top_left_tangent_y = 0.0;
        b.top_right_tangent_x = fw * 2.0 / 3.0;
        b.top_right_tangent_y = 0.0;
        b.right_top_tangent_x = fw;
        b.right_top_tangent_y = fh / 3.0;
        b.right_bottom_tangent_x = fw;
        b.right_bottom_tangent_y = fh * 2.0 / 3.0;
        b.bottom_left_tangent_x = fw / 3.0;
        b.bottom_left_tangent_y = fh;
        b.bottom_right_tangent_x = fw * 2.0 / 3.0;
        b.bottom_right_tangent_y = fh;
        b.left_top_tangent_x = 0.0;
        b.left_top_tangent_y = fh / 3.0;
        b.left_bottom_tangent_x = 0.0;
        b.left_bottom_tangent_y = fh * 2.0 / 3.0;
        b
    };
    // A barrel: both horizontal edges bowed outward.
    let mut bowed = base;
    bowed.top_left_tangent_y = -6.0;
    bowed.top_right_tangent_y = -6.0;
    bowed.bottom_left_tangent_y = h as f32 + 6.0;
    bowed.bottom_right_tangent_y = h as f32 + 6.0;
    // A corner dragged in, so the patch is a proper quadrilateral as well.
    let mut pulled = bowed;
    pulled.upper_right_x = 24.0;
    pulled.upper_right_y = 4.0;
    let mut coarse = bowed;
    coarse.quality = 1;
    let mut faded = bowed;
    faded.mix = 50.0;
    let mut off = bowed;
    off.mix = 0.0;

    let op_of = |b: BezierWarp| {
        let p = b.packed();
        BezierWarpOp {
            pts: p.pts,
            steps: p.steps,
            mix: p.mix,
        }
    };
    for (name, b) in [
        ("identity", base),
        ("bowed", bowed),
        ("pulled", pulled),
        ("coarse", coarse),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = b.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::bezier_warp(&mut cpu, w, h, &p);
        let out = fx.bezier_warp(&ctx, &tex, w, h, None, &op_of(b));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("bezier_warp {name}: worst {worst}");
        // K-399's metric with the wipes' step added: the patch's own boundary is
        // a THRESHOLD on a solved position, so the handful of pixels sitting on
        // it come out opaque on one path and transparent on the other. Away from
        // it the two agree to a thousandth; the tolerance is the drop shadow's
        // for the drop shadow's reason.
        assert!(worst < 2e-2, "{name}: worst diff {worst}");
        match name {
            "identity" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the patch must actually bend something"),
        }
    }

    // **Quality is convergence, not a look.** One step and twelve differ only
    // where a single Newton step has not caught up, and on a warp this size that
    // is a fraction of a pixel of colour.
    let mut fine = bowed;
    fine.quality = 12;
    let a = readback_linear_f32(
        &ctx,
        &fx.bezier_warp(&ctx, &tex, w, h, None, &op_of(fine)),
        w,
        h,
    )
    .unwrap();
    let b = readback_linear_f32(
        &ctx,
        &fx.bezier_warp(&ctx, &tex, w, h, None, &op_of(coarse)),
        w,
        h,
    )
    .unwrap();
    let spread = worst_diff(&a, &b);
    eprintln!("bezier_warp quality spread: {spread}");
    assert!(
        spread < 0.35,
        "quality must converge, not restyle: {spread}"
    );
}

/// The §1.6 oracle for Warp (docs/08 §3.56): all thirteen styles, on the smooth
/// corpus and by absolute difference — K-399's rule.
///
/// Two claims beyond parity. **Bend 0 with both distortions 0 is the bit-exact
/// identity, for every style**, which is what building the sample from the
/// *difference* is for. And **every style is its own picture**: no two of the
/// thirteen render the same frame at the same Bend, which is what catches a
/// style that fell through to its neighbour's arm.
#[test]
fn wgsl_warp_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::warp::Warp;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let op_of = |a: Warp| {
        let p = a.packed();
        WarpOp {
            style: p.style,
            bend: p.bend,
            h_distort: p.h_distort,
            v_distort: p.v_distort,
            mix: p.mix,
        }
    };
    let base = Warp::read(Params::EMPTY);
    let mut renders = Vec::new();
    for style in 0..13u32 {
        let mut a = base;
        a.style = style;
        a.bend = 60.0;
        let mut cpu = img.clone();
        lumit_core::fx::cpu::warp(&mut cpu, w, h, &a.packed());
        let out = fx.warp(&ctx, &tex, w, h, None, &op_of(a));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("warp style {style}: worst {worst}");
        assert!(worst < 2e-3, "style {style}: worst diff {worst}");
        assert!(gpu != img, "style {style} must actually bend the picture");
        // **Bend 0 is the identity for this style**, exactly.
        let mut flat = a;
        flat.bend = 0.0;
        let straight =
            readback_linear_f32(&ctx, &fx.warp(&ctx, &tex, w, h, None, &op_of(flat)), w, h)
                .unwrap();
        assert_eq!(
            straight, img,
            "style {style} at Bend 0 must be the identity"
        );
        renders.push(gpu);
    }
    // **No two styles render the same frame.**
    for i in 0..renders.len() {
        for j in (i + 1)..renders.len() {
            assert!(
                renders[i] != renders[j],
                "styles {i} and {j} render the same picture"
            );
        }
    }

    // The two tapers, and Mix.
    let mut tapered = base;
    tapered.bend = 30.0;
    tapered.horizontal_distortion = 60.0;
    tapered.vertical_distortion = -40.0;
    let mut faded = tapered;
    faded.mix = 50.0;
    let mut off = tapered;
    off.mix = 0.0;
    for (name, a) in [("tapered", tapered), ("mixed", faded), ("mix-zero", off)] {
        let mut cpu = img.clone();
        lumit_core::fx::cpu::warp(&mut cpu, w, h, &a.packed());
        let out = fx.warp(&ctx, &tex, w, h, None, &op_of(a));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("warp {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        }
    }
}

/// A soft-edged disc on a coloured field: the corpus Roughen edges needs, since
/// what it works on is an **alpha outline** and neither the §1.6 corpus (whose
/// alpha is a step down the middle) nor the smooth one (whose alpha is a ramp
/// with one contour in it) has an outline to chew.
fn disc_corpus(w: u32, h: u32) -> Vec<f32> {
    let mut img = vec![0.0f32; (w * h * 4) as usize];
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let radius = (w.min(h) as f32) * 0.35;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let r = ((px - cx) * (px - cx) + (py - cy) * (py - cy)).sqrt();
            // A one-pixel antialiased rim, as a real shape has.
            let a = (1.0 - (r - radius)).clamp(0.0, 1.0);
            let u = px / w as f32;
            let v = py / h as f32;
            img[i] = (0.2 + 0.7 * u) * a;
            img[i + 1] = (0.9 - 0.5 * v) * a;
            img[i + 2] = (0.3 + 0.4 * v) * a;
            img[i + 3] = a;
        }
    }
    img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

/// The §1.6 oracle for Roughen edges (docs/08 §3.57), on a disc and by absolute
/// difference.
///
/// **K-399's metric, one step further again.** The wipes are a threshold on a
/// position; this is a threshold on a *coverage* — the blurred alpha, which both
/// paths compute with the shipped §3.8 gaussian and which the GPU stores as
/// fp16. A last-bit difference there is multiplied by one over twice the cut's
/// half-width, so the tolerance is the drop shadow's rather than the distort
/// batch's, for the same reason: the two paths agree about the picture, and the
/// picture is a cliff.
///
/// Three claims beyond parity. **Border 0 is the exact identity** (the host
/// short-circuits, so this is checked on the CPU reference). **The chewing stays
/// at the outline**: the middle of the disc arrives untouched. And **the three
/// edge types are three pictures**.
#[test]
fn wgsl_roughen_edges_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::roughen_edges::RoughenEdges;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = disc_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Border, Scale and the Offset are px@comp; the resolve step would have
    // scaled them, so the test hands `packed` raster pixels directly.
    let base = {
        let mut r = RoughenEdges::read(Params::EMPTY);
        r.border = 4.0;
        r.scale = 8.0;
        r.offset_x = 16.0;
        r.offset_y = 12.0;
        r.seed = 7;
        r
    };
    let op_of = |r: RoughenEdges| {
        let p = r.packed();
        RoughenEdgesOp {
            seed: p.field.seed,
            octaves: p.field.octaves,
            gain: p.field.gain,
            lacunarity: p.field.lacunarity,
            cycle: p.field.cycle,
            flags: u32::from(p.field.perlin) | (u32::from(p.field.turbulent) << 1),
            offset: p.offset,
            inv_scale: p.inv_scale,
            z: p.z,
            border_px: p.border_px,
            influence: p.influence,
            half_width: p.half_width,
            colour: p.colour,
            colour_on: p.colour_on,
            mix: p.mix,
        }
    };

    let mut cut = base;
    cut.edge_type = 1;
    let mut spiky = base;
    spiky.edge_type = 2;
    let mut soft = base;
    soft.edge_sharpness = 10.0;
    let mut evolved = base;
    evolved.evolution = 200.0;
    evolved.cycle_evolution = true;
    evolved.cycle = 2;
    let mut coloured = base;
    coloured.colour_edge = true;
    coloured.edge_colour = [0.9, 0.1, 0.2, 1.0];
    let mut plain = base;
    plain.fractal_influence = 0.0;
    let mut faded = base;
    faded.mix = 50.0;
    let mut off = base;
    off.mix = 0.0;

    let mut renders = Vec::new();
    for (name, r) in [
        ("roughen", base),
        ("cut", cut),
        ("spiky", spiky),
        ("soft", soft),
        ("evolved", evolved),
        ("coloured", coloured),
        ("influence-zero", plain),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = r.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::roughen_edges(&mut cpu, w, h, &p);
        let out = fx.roughen_edges(&ctx, &tex, w, h, None, &op_of(r));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("roughen_edges {name}: worst {worst}");
        assert!(worst < 8e-2, "{name}: worst diff {worst}");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the edge must actually change");
        }
        if matches!(name, "roughen" | "cut" | "spiky") {
            renders.push(gpu.clone());
        }
        // **The middle of the disc is untouched**: nothing inside the shape
        // moves, which is what makes this a Stylise effect and not a distortion.
        let mid = ((12 * w + 16) * 4) as usize;
        for c in 0..4 {
            assert!(
                (gpu[mid + c] - img[mid + c]).abs() < 2e-2,
                "{name}: the middle of the shape moved"
            );
        }
    }

    // **The three edge types are three pictures.**
    for i in 0..renders.len() {
        for j in (i + 1)..renders.len() {
            assert!(
                renders[i] != renders[j],
                "edge types {i} and {j} render the same picture"
            );
        }
    }

    // **Border 0 is the exact identity.** The host short-circuits before the
    // kernel, so the claim is checked where it is made.
    let mut none = base;
    none.border = 0.0;
    assert!(!none.packed().active, "Border 0 must short-circuit");
    let mut cpu = img.clone();
    lumit_core::fx::cpu::roughen_edges(&mut cpu, w, h, &none.packed());
    assert_eq!(cpu, img, "Border 0 must be the bit-exact identity");
}

/// The §1.6 oracle for Posterize (docs/08 §3.58). This one is judged on **fp16
/// ULP even though its output is a step**, and that is the whole point of the
/// effect's second decision: the rungs are placed by `sqrt`, which is one
/// correctly-rounded instruction on both paths, so the two cannot land on
/// different rungs. If this test ever comes back with a difference of a whole
/// rung, the transfer function has stopped being exact — not the kernel.
#[test]
fn wgsl_posterize_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::posterize::Posterize;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |levels: i32, mix: f32| {
        let mut q = Posterize::read(Params::EMPTY);
        q.levels = levels;
        q.mix = mix;
        q
    };
    for (name, q) in [
        ("two-tone", of(2, 100.0)),
        ("poster", of(8, 100.0)),
        ("fine", of(24, 100.0)),
        ("mixed", of(4, 60.0)),
        ("mix-zero", of(4, 0.0)),
    ] {
        let (n, mix) = q.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::posterize(&mut cpu, n, mix);
        let out = fx.posterize(&ctx, &tex, w, h, None, &PosterizeOp { n, mix });
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("posterize {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually band");
        }

        let out2 = fx.posterize(&ctx, &tex, w, h, None, &PosterizeOp { n, mix });
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU posterize must be bit-stable");
    }

    // **The ladder is not clipped at white** (§3.58 decision 3): the corpus
    // carries an HDR spike at 6.0, and a two-level posterize must leave it
    // above 1 rather than snapping it to white.
    let (n, mix) = of(2, 100.0).packed();
    let mut cpu = img.clone();
    lumit_core::fx::cpu::posterize(&mut cpu, n, mix);
    let spike = ((10 * w + 20) * 4) as usize;
    assert!(
        cpu[spike] > 1.5,
        "the HDR spike lost its headroom: {}",
        cpu[spike]
    );
}

/// The §1.6 oracle for Threshold (docs/08 §3.59). A threshold is exactly the
/// shape K-399 warns about, and the floored half-width is the answer: the
/// crossing is a smoothstep a thousandth of the range wide, so a last-bit
/// disagreement about the luma moves the answer by a ten-thousandth rather
/// than flipping a pixel from black to white.
#[test]
fn wgsl_threshold_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::threshold::Threshold;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |level: f32, softness: f32, mix: f32| {
        let mut t = Threshold::read(Params::EMPTY);
        t.level = level;
        t.softness = softness;
        t.mix = mix;
        t
    };
    for (name, t) in [
        ("mid", of(50.0, 0.0, 100.0)),
        ("low", of(20.0, 0.0, 100.0)),
        ("high", of(80.0, 0.0, 100.0)),
        ("soft", of(50.0, 40.0, 100.0)),
        ("mixed", of(50.0, 0.0, 50.0)),
        ("mix-zero", of(50.0, 0.0, 0.0)),
    ] {
        let (level, half_width, mix) = t.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::threshold(&mut cpu, level, half_width, mix);
        let op = ThresholdOp {
            level,
            half_width,
            mix,
        };
        let out = fx.threshold(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("threshold {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually cut");
        }

        let out2 = fx.threshold(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU threshold must be bit-stable");
    }

    // **Alpha survives the cut** (§3.59): a thresholded picture keeps its
    // shape, so the transparent half of the corpus stays transparent.
    let (level, hw, mix) = of(50.0, 0.0, 100.0).packed();
    let mut cpu = img.clone();
    lumit_core::fx::cpu::threshold(&mut cpu, level, hw, mix);
    for i in (0..cpu.len()).step_by(4) {
        assert_eq!(cpu[i + 3], img[i + 3], "alpha moved at {i}");
    }
}

/// The §1.6 oracle for Tritone (docs/08 §3.60): a pointwise ramp, so ≤ 2 fp16
/// ULP, plus the headroom claim — an HDR pixel must come back above white
/// rather than clamped to the Highlights colour.
#[test]
fn wgsl_tritone_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::tritone::Tritone;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = Tritone::read(Params::EMPTY);
    let mut cyan = base;
    cyan.shadows = [0.0, 0.05, 0.25, 1.0];
    cyan.midtones = [0.1, 0.4, 0.7, 1.0];
    cyan.highlights = [0.95, 1.0, 1.0, 1.0];
    let mut faded = base;
    faded.mix = 40.0;
    let mut off = base;
    off.mix = 0.0;

    for (name, t) in [
        ("default", base),
        ("cyanotype", cyan),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = t.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::tritone(&mut cpu, &p);
        let op = TritoneOp {
            shadows: p.shadows,
            midtones: p.midtones,
            highlights: p.highlights,
            mix: p.mix,
        };
        let out = fx.tritone(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("tritone {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually tone");
        }

        let out2 = fx.tritone(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU tritone must be bit-stable");
    }

    // **A specular keeps its headroom** (§3.60): the corpus spike is a luma
    // well above 1, and its answer must be the Highlights colour *scaled*.
    let p = base.packed();
    let mut cpu = img.clone();
    lumit_core::fx::cpu::tritone(&mut cpu, &p);
    let spike = ((10 * w + 20) * 4) as usize;
    assert!(
        cpu[spike] > 1.5,
        "the HDR spike was clamped to the highlight colour: {}",
        cpu[spike]
    );
}

/// The §1.6 oracle for Photo filter (docs/08 §3.61): a multiply and a luma
/// renormalisation, so ≤ 2 fp16 ULP, with Density 0 the bit-exact identity on
/// both paths.
#[test]
fn wgsl_photo_filter_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::photo_filter::PhotoFilter;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |filter: u32, density: f32, preserve: bool, mix: f32| {
        let mut f = PhotoFilter::read(Params::EMPTY);
        f.filter = filter;
        f.density = density;
        f.preserve_luminosity = preserve;
        f.mix = mix;
        f
    };
    let mut custom = of(PhotoFilter::CUSTOM, 80.0, true, 100.0);
    custom.colour = [0.2, 0.6, 0.9, 1.0];
    for (name, f) in [
        ("warming-85", of(0, 25.0, true, 100.0)),
        ("cooling-80-full", of(3, 100.0, true, 100.0)),
        ("deep-red-unpreserved", of(15, 100.0, false, 100.0)),
        ("custom", custom),
        ("mixed", of(0, 50.0, true, 50.0)),
        ("density-zero", of(0, 0.0, true, 100.0)),
        ("mix-zero", of(0, 25.0, true, 0.0)),
    ] {
        let p = f.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::photo_filter(&mut cpu, &p);
        let op = PhotoFilterOp {
            filter: p.filter,
            density: p.density,
            preserve: p.preserve,
            mix: p.mix,
        };
        let out = fx.photo_filter(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("photo_filter {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "density-zero" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the glass must actually colour");
        }

        let out2 = fx.photo_filter(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU photo filter must be bit-stable");
    }

    // **Preserve luminosity is the difference between a filter and a
    // multiply** (§3.61): the same deep red at the same density is a far
    // darker picture with it off.
    let lit = of(15, 100.0, true, 100.0).packed();
    let dark = of(15, 100.0, false, 100.0).packed();
    let (mut a, mut b) = (img.clone(), img.clone());
    lumit_core::fx::cpu::photo_filter(&mut a, &lit);
    lumit_core::fx::cpu::photo_filter(&mut b, &dark);
    // Measured on luma, not on a channel: a deep red filter takes green and
    // blue to nothing whether or not the exposure is restored, so a channel
    // probe would answer zero on both.
    let sum = |v: &[f32]| {
        v.chunks_exact(4)
            .map(|p| p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722)
            .sum::<f32>()
    };
    assert!(
        sum(&a) > sum(&b) * 3.0,
        "preserve luminosity did not restore the exposure"
    );
}

/// The §1.6 oracle for Black and white (docs/08 §3.62). The six-branch
/// decomposition is the one place in the batch where the two paths could
/// plausibly take *different branches*, so the corpus sweeps a full gradient,
/// which crosses every channel ordering — and the branches agree at their
/// boundaries by construction, which is what makes that safe.
#[test]
fn wgsl_black_and_white_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::black_and_white::BlackAndWhite;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let base = BlackAndWhite::read(Params::EMPTY);
    let mut red_filter = base;
    red_filter.reds = 180.0;
    red_filter.blues = -60.0;
    let mut tinted = base;
    tinted.tint = true;
    let mut faded = base;
    faded.mix = 35.0;
    let mut off = base;
    off.mix = 0.0;

    for (name, b) in [
        ("default", base),
        ("red-filter", red_filter),
        ("sepia", tinted),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = b.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::black_and_white(&mut cpu, &p);
        let op = BlackAndWhiteOp {
            weights: p.weights,
            tint: p.tint,
            tint_on: p.tint_on,
            mix: p.mix,
        };
        let out = fx.black_and_white(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("black_and_white {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually convert");
        }

        let out2 = fx.black_and_white(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU black and white must be bit-stable");
    }

    // **The six weights do nothing at all to a grey** (§3.62): every
    // difference in the decomposition is zero there, which is the property the
    // whole scheme rests on.
    let p = red_filter.packed();
    for v in [0.05_f32, 0.25, 0.6, 3.0] {
        let grey = lumit_core::fx::cpu::bw_grey([v, v, v], &p.weights);
        assert!((grey - v).abs() < 1e-6, "a grey of {v} came back as {grey}");
    }

    // **The decomposition is exact**: all six weights at 100 answer the
    // channel maximum, whatever the ordering.
    let mut ones = base;
    ones.reds = 100.0;
    ones.yellows = 100.0;
    ones.greens = 100.0;
    ones.cyans = 100.0;
    ones.blues = 100.0;
    ones.magentas = 100.0;
    let w1 = ones.packed().weights;
    for u in [
        [0.8_f32, 0.4, 0.1],
        [0.1, 0.9, 0.3],
        [0.2, 0.3, 0.7],
        [0.5, 0.5, 0.5],
        [1.0, 0.0, 1.0],
    ] {
        let grey = lumit_core::fx::cpu::bw_grey(u, &w1);
        let max = u[0].max(u[1]).max(u[2]);
        assert!(
            (grey - max).abs() < 1e-6,
            "{u:?} answered {grey}, not {max}"
        );
    }
}

/// The §1.6 oracle for Shadow highlight (docs/08 §3.63). Judged on **absolute
/// difference, not fp16 ULP**, for the reason K-399 records: the answer is a
/// gaussian's output read through a mask, and the shipped blur's two paths
/// already agree only to a tolerance — which the mask then carries into the
/// gain. The claims that matter are checked separately below.
#[test]
fn wgsl_shadow_highlight_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::shadow_highlight::ShadowHighlight;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // Radius is % diag; the resolve step would have made it raster pixels, so
    // the test hands `packed` a value already in them.
    let base = {
        let mut s = ShadowHighlight::read(Params::EMPTY);
        s.radius = 3.0;
        s
    };
    let mut lift = base;
    lift.shadow_amount = 100.0;
    lift.highlight_amount = 0.0;
    // The corpus is a gradient between two saturated colours rather than a
    // black-to-white ramp, so its darkest region sits around a perceptual 0.6.
    // A full tonal width is what reaches it.
    lift.shadow_tonal_width = 100.0;
    let mut pull = base;
    pull.shadow_amount = 0.0;
    pull.highlight_amount = 100.0;
    let mut narrow = base;
    narrow.shadow_tonal_width = 10.0;
    narrow.highlight_tonal_width = 10.0;
    let mut punchy = base;
    punchy.midtone_contrast = 60.0;
    let mut grey = base;
    grey.colour_correction = -100.0;
    // Radius 0 is not the identity and is not meant to be: the neighbourhood
    // collapses to the pixel, and the effect becomes a whole-picture tone
    // curve. Both paths must still agree, which is what this case pins.
    let mut global = base;
    global.radius = 0.0;
    let mut faded = base;
    faded.mix = 45.0;
    let mut off = base;
    off.mix = 0.0;

    let op_of = |s: ShadowHighlight| {
        let p = s.packed();
        ShadowHighlightOp {
            shadow: p.shadow,
            highlight: p.highlight,
            shadow_width: p.shadow_width,
            highlight_width: p.highlight_width,
            radius_px: p.radius_px,
            contrast: p.contrast,
            colour_correction: p.colour_correction,
            mix: p.mix,
        }
    };
    for (name, s) in [
        ("default", base),
        ("lift", lift),
        ("pull", pull),
        ("narrow", narrow),
        ("punchy", punchy),
        ("desaturating", grey),
        ("global", global),
        ("mixed", faded),
        ("mix-zero", off),
    ] {
        let p = s.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::shadow_highlight(&mut cpu, w, h, &p);
        let out = fx.shadow_highlight(&ctx, &tex, w, h, None, &op_of(s));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_diff(&cpu, &gpu);
        eprintln!("shadow_highlight {name}: worst {worst}");
        assert!(worst < 8e-2, "{name}: worst diff {worst}");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually move");
        }

        let out2 = fx.shadow_highlight(&ctx, &tex, w, h, None, &op_of(s));
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU shadow highlight must be bit-stable");
    }

    // **A neutral instance is the exact identity, and does not blur.** The
    // host short-circuits before either path runs, so the claim is checked
    // where it is made.
    let mut none = base;
    none.shadow_amount = 0.0;
    none.highlight_amount = 0.0;
    assert!(
        !none.packed().active,
        "no lift, no pull and no contrast must short-circuit"
    );
    let mut cpu = img.clone();
    lumit_core::fx::cpu::shadow_highlight(&mut cpu, w, h, &none.packed());
    assert_eq!(
        cpu, img,
        "a neutral instance must be the bit-exact identity"
    );

    // **The lift goes where the neighbourhood is dark, and the pull where it
    // is bright** — the local half of local-adaptive. The corpus gradient runs
    // from dark at the top left to bright at the bottom right, so the two must
    // move opposite corners.
    let dark = ((23 * w + 8) * 4) as usize;
    let bright = ((3 * w + 3) * 4) as usize;
    let mut lifted = img.clone();
    lumit_core::fx::cpu::shadow_highlight(&mut lifted, w, h, &lift.packed());
    let mut pulled = img.clone();
    lumit_core::fx::cpu::shadow_highlight(&mut pulled, w, h, &pull.packed());
    assert!(
        lifted[dark + 1] > img[dark + 1] * 1.2,
        "the dark corner was not lifted"
    );
    assert!(
        pulled[bright + 1] < img[bright + 1] * 0.95,
        "the bright corner was not pulled down"
    );
}

/// A picture with speckle in it, which is the only corpus that can tell a median
/// from a blur: the smooth corpus with one pixel in nine kicked to black or to
/// white. A blur smears a speck across its neighbourhood; a median removes it
/// entirely and leaves everything else where it was.
fn speckled_corpus(w: u32, h: u32) -> Vec<f32> {
    let mut img = smooth_corpus(w, h);
    for y in 0..h {
        for x in 0..w {
            if (x * 7 + y * 13) % 9 != 0 {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            let a = img[i + 3];
            let v = if (x + y) % 2 == 0 { 0.0 } else { 3.0 };
            img[i] = v * a;
            img[i + 1] = v * a;
            img[i + 2] = v * a;
        }
    }
    img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

/// The §1.6 oracle for Median (docs/08 §3.64).
///
/// **This one should be exact, and the test says so.** Every step of the
/// selection is a `min` or a `max`, both of which are exactly-rounded and
/// order-independent, so the two paths cannot land on different samples. The
/// tolerance is still stated in fp16 ULPs because the *inputs* differ — the GPU
/// unpremultiplies an fp16 texel and the CPU an f32 array — but a failure here
/// larger than a bit is the network having gone wrong, not the arithmetic.
///
/// Three claims beyond parity: **Radius 0 is the exact identity**, **the speckle
/// is actually removed** (which a blur would only soften), and **the coverage is
/// untouched unless Operate on alpha says otherwise**.
#[test]
fn wgsl_median_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::median::Median;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = speckled_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // The cap is one number in three places (the declaration, the CPU reference
    // and the WGSL kernel's window bound); two of the three are checkable here.
    assert_eq!(Median::MAX_RADIUS, lumit_core::fx::cpu::MEDIAN_MAX_RADIUS);
    assert_eq!(Median::KEEP, lumit_core::fx::cpu::MEDIAN_KEEP);
    let widest = (2 * Median::MAX_RADIUS + 1) * (2 * Median::MAX_RADIUS + 1);
    assert_eq!(
        Median::KEEP as i32,
        (widest + 1) / 2,
        "the array must be exactly the longest run the network carries"
    );

    let of = |radius: f32, alpha: bool, mix: f32| {
        let mut m = Median::read(Params::EMPTY);
        m.radius = radius;
        m.alpha = alpha;
        m.mix = mix;
        m
    };
    let op_of = |m: Median| {
        let p = m.packed();
        let n = (2 * p.radius + 1) * (2 * p.radius + 1);
        MedianOp {
            radius: p.radius,
            keep: (n + 1) / 2,
            alpha_on: f32::from(u8::from(p.alpha)),
            mix: p.mix,
        }
    };
    for (name, m) in [
        ("one", of(1.0, false, 100.0)),
        ("two", of(2.0, false, 100.0)),
        ("cap", of(3.0, false, 100.0)),
        ("with-alpha", of(2.0, true, 100.0)),
        ("mixed", of(2.0, false, 50.0)),
        ("mix-zero", of(2.0, false, 0.0)),
        ("radius-zero", of(0.0, false, 100.0)),
    ] {
        let p = m.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::median(&mut cpu, w, h, &p);
        let op = op_of(m);
        let out = fx.median(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("median {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" || name == "radius-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(
                gpu != img,
                "{name}: the median must actually change something"
            );
        }

        let out2 = fx.median(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU median must be bit-stable");
    }

    // **The speckle really goes.** A speck has no neighbours agreeing with it,
    // so it never wins the vote — where a blur would leave a smeared trace of
    // every one. Measured on the interior, away from the clamped border, and
    // against the *un*speckled picture the corpus was made from.
    let clean = smooth_corpus(w, h);
    let mut fixed = img.clone();
    lumit_core::fx::cpu::median(&mut fixed, w, h, &of(2.0, false, 100.0).packed());
    let interior = |v: &[f32]| {
        let mut worst = 0.0f32;
        for y in 4..h - 4 {
            for x in 4..w - 4 {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    worst = worst.max((v[i + c] - clean[i + c]).abs());
                }
            }
        }
        worst
    };
    let before = interior(&img);
    let after = interior(&fixed);
    eprintln!("median speckle: {before:.4} before, {after:.4} after");
    assert!(before > 1.0, "the corpus is not actually speckled");
    assert!(
        after < before * 0.25,
        "the median left the speckle behind: {after} of {before}"
    );

    // **The coverage is untouched unless asked.** The corpus has a real alpha
    // ramp; with Operate on alpha off, not one texel of it may move.
    let mut kept = img.clone();
    lumit_core::fx::cpu::median(&mut kept, w, h, &of(3.0, false, 100.0).packed());
    for i in (0..kept.len()).step_by(4) {
        assert_eq!(kept[i + 3], img[i + 3], "alpha moved at {i}");
    }
}

/// The §1.6 oracle for Mosaic (docs/08 §3.65).
///
/// **Judged on fp16 ULPs even though its output is a step**, for §3.58's reason
/// in another costume: every block boundary is an *integer* division, so the two
/// paths cannot disagree about which block a pixel is in, and the only floating
/// point left is the mean of at most 64 taps summed in one fixed order. A
/// failure of a whole block's colour here is the integer arithmetic having gone
/// wrong, not the average.
///
/// Three claims beyond parity: **the picture is actually blocked** (a whole
/// block is one colour), **the sharp mode is a different picture from the
/// averaged one**, and **a block finer than the sample grid is an exact mean**.
#[test]
fn wgsl_mosaic_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::mosaic::Mosaic;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |bx: i32, by: i32, sharp: bool, mix: f32| {
        let mut m = Mosaic::read(Params::EMPTY);
        m.horizontal_blocks = bx;
        m.vertical_blocks = by;
        m.sharp_colours = sharp;
        m.mix = mix;
        m
    };
    let op_of = |m: Mosaic| {
        let p = m.packed();
        MosaicOp {
            blocks: p.blocks,
            sharp: f32::from(u8::from(p.sharp)),
            mix: p.mix,
        }
    };
    for (name, m) in [
        ("coarse", of(4, 3, false, 100.0)),
        ("default", of(24, 14, false, 100.0)),
        ("sharp", of(4, 3, true, 100.0)),
        ("one-block", of(1, 1, false, 100.0)),
        ("finer-than-the-grid", of(16, 12, false, 100.0)),
        ("mixed", of(4, 3, false, 50.0)),
        ("mix-zero", of(4, 3, false, 0.0)),
    ] {
        let p = m.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::mosaic(&mut cpu, w, h, &p);
        let op = op_of(m);
        let out = fx.mosaic(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("mosaic {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually block");
        }

        let out2 = fx.mosaic(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU mosaic must be bit-stable");
    }

    // **A block really is one colour.** 4×3 blocks on a 32×24 frame is exactly
    // 8×8 pixels each, so every texel of the top-left block must equal its
    // first.
    let mut blocked = img.clone();
    lumit_core::fx::cpu::mosaic(&mut blocked, w, h, &of(4, 3, false, 100.0).packed());
    for y in 0..8u32 {
        for x in 0..8u32 {
            let i = ((y * w + x) * 4) as usize;
            assert_eq!(
                &blocked[i..i + 4],
                &blocked[0..4],
                "the block is not one colour at {x},{y}"
            );
        }
    }

    // **Sharp is a different picture.** The centre pixel of a block is not its
    // mean on any corpus with structure in it.
    let mut sharp = img.clone();
    lumit_core::fx::cpu::mosaic(&mut sharp, w, h, &of(4, 3, true, 100.0).packed());
    assert!(
        sharp != blocked,
        "Sharp colours changed nothing — the switch is not wired"
    );

    // **A block finer than the sample grid is an exact mean.** 16×12 blocks on a
    // 32×24 frame is 2×2 pixels, which the 8×8 grid samples completely, so the
    // answer must equal the mean computed here by hand.
    let mut fine = img.clone();
    lumit_core::fx::cpu::mosaic(&mut fine, w, h, &of(16, 12, false, 100.0).packed());
    let mut want = [0.0f32; 4];
    for (y, x) in [(0u32, 0u32), (0, 1), (1, 0), (1, 1)] {
        let i = ((y * w + x) * 4) as usize;
        for c in 0..4 {
            want[c] += img[i + c];
        }
    }
    for c in 0..4 {
        want[c] *= 0.25;
        assert!(
            (fine[c] - want[c]).abs() < 1e-6,
            "a 2×2 block is not its own exact mean: {} vs {}",
            fine[c],
            want[c]
        );
    }
}

/// The §1.6 oracle for Find edges (docs/08 §3.66): eight taps and a magnitude,
/// so ≤ 2 fp16 ULP on the smooth corpus.
///
/// Three claims beyond parity: **a flat picture has no edges** (the default must
/// come back white where nothing changes), **Invert is the complement** of the
/// default, and **the alpha survives** so the drawing keeps the layer's shape.
#[test]
fn wgsl_find_edges_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::find_edges::FindEdges;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = disc_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |invert: bool, mix: f32| {
        let mut f = FindEdges::read(Params::EMPTY);
        f.invert = invert;
        f.mix = mix;
        f
    };
    for (name, f) in [
        ("drawing", of(false, 100.0)),
        ("glow", of(true, 100.0)),
        ("mixed", of(false, 50.0)),
        ("mix-zero", of(false, 0.0)),
    ] {
        let (invert, mix) = f.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::find_edges(&mut cpu, w, h, invert, mix);
        let op = FindEdgesOp { invert, mix };
        let out = fx.find_edges(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("find_edges {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the picture must actually draw");
        }

        let out2 = fx.find_edges(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU find edges must be bit-stable");
    }

    // **A flat picture has no edges.** A frame of one colour must come back
    // white in the default and black inverted — the two ends of the drawing.
    let flat: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [0.4f32, 0.4, 0.4, 1.0])
        .collect();
    for (invert, want) in [(false, 1.0f32), (true, 0.0)] {
        let (iv, mix) = of(invert, 100.0).packed();
        let mut out = flat.clone();
        lumit_core::fx::cpu::find_edges(&mut out, w, h, iv, mix);
        let i = ((h / 2 * w + w / 2) * 4) as usize;
        assert!(
            (out[i] - want).abs() < 1e-5,
            "a flat picture with invert {invert} gave {} not {want}",
            out[i]
        );
    }

    // **The alpha survives the drawing**, so a title comes back as a drawing of
    // a title rather than as a full white frame.
    let (iv, mix) = of(false, 100.0).packed();
    let mut kept = img.clone();
    lumit_core::fx::cpu::find_edges(&mut kept, w, h, iv, mix);
    for i in (0..kept.len()).step_by(4) {
        assert_eq!(kept[i + 3], img[i + 3], "alpha moved at {i}");
    }
}

/// The §1.6 oracle for Emboss (docs/08 §3.67): two bilinear taps and a
/// difference, so the distort batch's absolute-difference metric on the smooth
/// corpus — the output is built from *sample positions*, and a fused
/// multiply-add one path takes moves a tap by its last bits.
///
/// Three claims beyond parity: **the relief is grey** (all three channels equal
/// wherever the layer is opaque), **Relief 0 is flat mid-grey rather than the
/// identity**, and **turning the light round flips the relief** about mid-grey.
#[test]
fn wgsl_emboss_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::emboss::Emboss;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |direction: f32, relief: f32, contrast: f32, mix: f32| {
        let mut e = Emboss::read(Params::EMPTY);
        e.direction = direction;
        e.relief = relief;
        e.contrast = contrast;
        e.mix = mix;
        e
    };
    let op_of = |e: Emboss| {
        let p = e.packed();
        EmbossOp {
            offset: p.offset,
            contrast: p.contrast,
            mix: p.mix,
        }
    };
    for (name, e) in [
        ("default", of(45.0, 2.0, 100.0, 100.0)),
        ("from-the-left", of(270.0, 3.0, 150.0, 100.0)),
        ("deep", of(45.0, 8.0, 200.0, 100.0)),
        ("relief-zero", of(45.0, 0.0, 100.0, 100.0)),
        ("mixed", of(45.0, 2.0, 100.0, 50.0)),
        ("mix-zero", of(45.0, 2.0, 100.0, 0.0)),
    ] {
        let p = e.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::emboss(&mut cpu, w, h, &p);
        let op = op_of(e);
        let out = fx.emboss(&ctx, &tex, w, h, None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_diff(&cpu, &gpu);
        eprintln!("emboss {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the relief must actually stamp");
        }

        let out2 = fx.emboss(&ctx, &tex, w, h, None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU emboss must be bit-stable");
    }

    // **The relief is grey**: AE's Emboss suppresses colour and so does this, so
    // the three channels must agree wherever the layer has coverage.
    let mut grey = img.clone();
    lumit_core::fx::cpu::emboss(&mut grey, w, h, &of(45.0, 2.0, 100.0, 100.0).packed());
    for px in grey.chunks_exact(4) {
        if px[3] < 1e-3 {
            continue;
        }
        assert!(
            (px[0] - px[1]).abs() < 1e-6 && (px[1] - px[2]).abs() < 1e-6,
            "the relief kept a colour: {px:?}"
        );
    }

    // **Relief 0 is flat mid-grey, not the identity** (§3.67's second note): the
    // two taps coincide, so every pixel comes back at ¼ of the light — the
    // square of a half.
    let mut flat = img.clone();
    lumit_core::fx::cpu::emboss(&mut flat, w, h, &of(45.0, 0.0, 100.0, 100.0).packed());
    for px in flat.chunks_exact(4) {
        if px[3] < 1e-3 {
            continue;
        }
        assert!(
            (px[0] / px[3] - 0.25).abs() < 1e-4,
            "Relief 0 is not flat mid-grey: {px:?}"
        );
    }

    // **Turning the light round flips the relief.** The two answers must sit on
    // opposite sides of mid-grey, which is what says the Direction dial is wired
    // to the sign of the difference and not merely to its size.
    let mut lit = img.clone();
    let mut back = img.clone();
    lumit_core::fx::cpu::emboss(&mut lit, w, h, &of(90.0, 3.0, 100.0, 100.0).packed());
    lumit_core::fx::cpu::emboss(&mut back, w, h, &of(270.0, 3.0, 100.0, 100.0).packed());
    let mut opposed = 0;
    for (a, b) in lit.chunks_exact(4).zip(back.chunks_exact(4)) {
        if a[3] < 0.5 {
            continue;
        }
        let (ga, gb) = (a[0] / a[3] - 0.25, b[0] / b[3] - 0.25);
        if ga.abs() > 1e-3 && ga * gb < 0.0 {
            opposed += 1;
        }
    }
    assert!(
        opposed > 100,
        "the light direction does not flip the relief: {opposed} pixels opposed"
    );
}

/// The §1.6 oracle for Texturize (docs/08 §3.68): the same two taps as Emboss,
/// on a second picture, so the same absolute-difference metric.
///
/// Four claims beyond parity: **an unset Texture is the exact identity**, **a
/// flat texture presses nothing**, **the three Placements are three pictures**,
/// and **Texture contrast 0 is the identity**.
#[test]
fn wgsl_texturize_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::texturize::Texturize;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    // A canvas: a coarse weave with real structure in both axes.
    let mut weave = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let u = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            let g = 0.5 + 0.4 * ((u * 18.0).sin() * (v * 14.0).cos());
            weave[i] = g;
            weave[i + 1] = g;
            weave[i + 2] = g;
            weave[i + 3] = 1.0;
        }
    }
    let qweave: Vec<f32> = weave.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
    let weave_tex = upload_linear_f32(&ctx, &weave, w, h);

    let of = |placement: u32, scale: f32, relief: f32, contrast: f32, mix: f32| {
        let mut t = Texturize::read(Params::EMPTY);
        t.placement = placement;
        t.scale = scale;
        t.relief = relief;
        t.texture_contrast = contrast;
        t.mix = mix;
        t
    };
    let op_of = |t: Texturize| {
        let p = t.packed();
        TexturizeOp {
            offset: p.offset,
            contrast: p.contrast,
            inv_scale: p.inv_scale,
            placement: p.placement,
            mix: p.mix,
        }
    };
    for (name, t) in [
        ("stretch", of(0, 100.0, 1.0, 100.0, 100.0)),
        ("tiled", of(1, 40.0, 1.0, 100.0, 100.0)),
        ("centred", of(2, 50.0, 1.0, 100.0, 100.0)),
        ("deep", of(0, 100.0, 4.0, 200.0, 100.0)),
        ("contrast-zero", of(0, 100.0, 1.0, 0.0, 100.0)),
        ("mixed", of(0, 100.0, 1.0, 100.0, 50.0)),
        ("mix-zero", of(0, 100.0, 1.0, 100.0, 0.0)),
    ] {
        let p = t.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::texturize(&mut cpu, &qweave, w, h, &p);
        let op = op_of(t);
        let out = fx.texturize(&ctx, &tex, w, h, Some(&weave_tex), None, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_diff(&cpu, &gpu);
        eprintln!("texturize {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "mix-zero" || name == "contrast-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the texture must actually press");
        }

        let out2 = fx.texturize(&ctx, &tex, w, h, Some(&weave_tex), None, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU texturize must be bit-stable");
    }

    // **An unset Texture is the exact identity** — the labelled no-op §1.2
    // sanctions, and here the host's answer rather than the kernel's.
    let out = fx.texturize(
        &ctx,
        &tex,
        w,
        h,
        None,
        None,
        &op_of(of(0, 100.0, 1.0, 100.0, 100.0)),
    );
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "an unbound texture must render the input untouched"
    );

    // **A flat texture presses nothing**: no gradient, no relief, whatever the
    // contrast. This is the property that makes an unset row and a blank layer
    // agree.
    let blank: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [0.5f32, 0.5, 0.5, 1.0])
        .collect();
    let mut pressed = img.clone();
    lumit_core::fx::cpu::texturize(
        &mut pressed,
        &blank,
        w,
        h,
        &of(0, 100.0, 2.0, 200.0, 100.0).packed(),
    );
    assert_eq!(pressed, img, "a flat texture must press nothing");

    // **The three Placements are three pictures** at a Scale that is not 100 —
    // and all three agree at Scale 100, which is what makes that the AE case.
    let render = |t: Texturize| {
        let mut out = img.clone();
        lumit_core::fx::cpu::texturize(&mut out, &qweave, w, h, &t.packed());
        out
    };
    let (s, ti, c) = (
        render(of(0, 40.0, 1.0, 100.0, 100.0)),
        render(of(1, 40.0, 1.0, 100.0, 100.0)),
        render(of(2, 40.0, 1.0, 100.0, 100.0)),
    );
    assert!(
        s != ti && ti != c && s != c,
        "the Placements are one picture"
    );

    // **At Scale 100 the three Placements coincide**, which is what makes that
    // case AE's Stretch Texture to Fit exactly (§3.68 decision 2) — away from
    // the frame's own border, where the relief's two taps step outside the
    // single copy and the three fittings are precisely the three different
    // answers to that.
    let full = [
        render(of(0, 100.0, 1.0, 100.0, 100.0)),
        render(of(1, 100.0, 1.0, 100.0, 100.0)),
        render(of(2, 100.0, 1.0, 100.0, 100.0)),
    ];
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let i = ((y * w + x) * 4) as usize;
            assert_eq!(
                &full[0][i..i + 4],
                &full[1][i..i + 4],
                "Stretch and Tile disagree inside the copy at {x},{y}"
            );
            assert_eq!(
                &full[0][i..i + 4],
                &full[2][i..i + 4],
                "Stretch and Centre disagree inside the copy at {x},{y}"
            );
        }
    }
}

/// The §1.6 oracle for Broadcast safe (docs/08 §3.69).
///
/// **K-399's rule, and the reason the kernel writes its luma out longhand.** Two
/// of the four modes turn the amplitude into a *threshold on the alpha*, so a
/// fused multiply-add taken by one path and not the other is a pixel keyed out
/// on one and kept on the other — which is why this kernel is the only one in
/// the family not to use `dot`.
///
/// Four claims beyond parity: **a legal pixel is untouched** by both repair
/// modes, **both repairs make an illegal frame legal**, **the two key modes are
/// exact complements**, and **the standard changes the answer** (NTSC's setup
/// pedestal leaves less room than PAL's none).
#[test]
fn wgsl_broadcast_safe_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::broadcast_safe::BroadcastSafe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let of = |standard: u32, mode: u32, max: f32, mix: f32| {
        let mut b = BroadcastSafe::read(Params::EMPTY);
        b.standard = standard;
        b.how_to_treat = mode;
        b.maximum_signal = max;
        b.mix = mix;
        b
    };
    let op_of = |b: BroadcastSafe| {
        let p = b.packed();
        BroadcastSafeOp {
            target: p.target,
            mode: p.mode,
            mix: p.mix,
        }
    };
    for (name, b) in [
        ("ntsc-brightness", of(0, 0, 110.0, 100.0)),
        ("ntsc-saturation", of(0, 1, 110.0, 100.0)),
        ("pal-brightness", of(1, 0, 100.0, 100.0)),
        ("key-unsafe", of(0, 2, 110.0, 100.0)),
        ("key-safe", of(0, 3, 110.0, 100.0)),
        ("strict", of(0, 0, 90.0, 100.0)),
        ("mixed", of(0, 0, 100.0, 50.0)),
        ("mix-zero", of(0, 0, 100.0, 0.0)),
    ] {
        let p = b.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::broadcast_safe(&mut cpu, &p);
        let op = op_of(b);
        let out = fx.broadcast_safe(&ctx, &tex, w, h, &op);
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

        let worst = worst_f16_ulp(&cpu, &gpu);
        eprintln!("broadcast_safe {name}: worst {worst} ulp");
        assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the clamp must actually bite");
        }

        let out2 = fx.broadcast_safe(&ctx, &tex, w, h, &op);
        let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
        assert_eq!(gpu, gpu2, "GPU broadcast safe must be bit-stable");
    }

    // The amplitude of one straight colour, as the effect measures it.
    let amp = |rgb: [f32; 3]| {
        let v = [rgb[0].sqrt(), rgb[1].sqrt(), rgb[2].sqrt()];
        let y = v[0] * 0.2126 + v[1] * 0.7152 + v[2] * 0.0722;
        y + lumit_core::fx::cpu::broadcast_chroma(v, y)
    };

    // **A legal pixel is untouched, and both repairs make an illegal frame
    // legal.** Fully saturated yellow is the classic offender — its luma is
    // nearly white and its chroma is large, so the two add to about 136 IRE,
    // which is the figure a colour-bar generator is measured against. A mid grey
    // is comfortably legal.
    let hot = [1.0f32, 1.0, 0.0];
    let calm = [0.18f32, 0.18, 0.18];
    let frame: Vec<f32> = [hot, calm]
        .iter()
        .flat_map(|c| [c[0], c[1], c[2], 1.0])
        .collect();
    assert!(amp(hot) > 1.1, "the test's hot pixel is already legal");
    for mode in [0u32, 1] {
        let p = of(0, mode, 110.0, 100.0).packed();
        let mut out = frame.clone();
        lumit_core::fx::cpu::broadcast_safe(&mut out, &p);
        assert!(
            amp([out[0], out[1], out[2]]) <= p.target + 1e-4,
            "mode {mode} left the hot pixel illegal"
        );
        assert!(
            (out[4] - calm[0]).abs() < 1e-5 && (out[7] - 1.0).abs() < 1e-6,
            "mode {mode} moved a pixel that was already legal"
        );
    }

    // **The two key modes are exact complements**: every pixel is removed by one
    // and kept by the other, which is what makes Key out safe an overlay of the
    // problem rather than a second guess at it.
    let (mut unsafe_out, mut safe_out) = (img.clone(), img.clone());
    lumit_core::fx::cpu::broadcast_safe(&mut unsafe_out, &of(0, 2, 110.0, 100.0).packed());
    lumit_core::fx::cpu::broadcast_safe(&mut safe_out, &of(0, 3, 110.0, 100.0).packed());
    for i in (0..img.len()).step_by(4) {
        let kept = unsafe_out[i + 3] > 0.0;
        let other = safe_out[i + 3] > 0.0;
        assert!(
            kept != other || img[i + 3] == 0.0,
            "the two views overlap at {i}"
        );
    }

    // **The standard changes the answer, and it is worth knowing which way.**
    // NTSC's black sits 7.5 IRE up, so the picture's own swing is 92.5 IRE
    // rather than 100 — and a peak quoted as 110 IRE is therefore 102.5⁄92.5 of
    // that swing, a *larger* fraction than PAL's 110⁄100. At the same Maximum
    // signal NTSC clamps a shade less hard, not more. What the test is really
    // holding is that the dropdown reaches the arithmetic at all.
    assert!(
        of(0, 0, 110.0, 100.0).packed().target > of(1, 0, 110.0, 100.0).packed().target,
        "the standard's setup pedestal is not being spent"
    );
}

/// The two crates do not depend on one another at build time, so the segment cap
/// is written down twice (docs/08 §3.74's first decision). This is the pin: a
/// bolt the kernel could not hold would silently lose its far end.
#[test]
fn the_lightning_segment_cap_matches_the_core() {
    assert_eq!(
        LIGHTNING_SEGMENTS,
        lumit_core::fx::cpu::LIGHTNING_SEGMENTS,
        "the uniform's segment array and the builder's disagree"
    );
}

/// The §1.6 oracle for Beam (docs/08 §3.73), on the smooth corpus and by
/// absolute difference — the coverage is a *threshold on a distance*, which
/// magnifies a fused multiply-add exactly as a sample position does (K-399).
///
/// Three claims beyond parity: Time 0 is the exact identity; **the beam tapers**,
/// so the lit width at the tail exceeds the lit width at the head; and **both
/// colours appear**, which is what the crossover taking the rim's inner half is
/// for and is invisible to any parity check.
#[test]
fn wgsl_beam_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::beam::Beam;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let mut base = Beam::read(Params::EMPTY);
    base.start_x = 6.0;
    base.start_y = 26.0;
    base.end_x = 42.0;
    base.end_y = 6.0;
    base.start_thickness = 7.0;
    base.end_thickness = 2.0;
    let op_of = |b: Beam| {
        let p = b.packed();
        BeamOp {
            start: p.start,
            axis: p.axis,
            inv_len2: p.inv_len2,
            u0: p.u0,
            u1: p.u1,
            inv_span: p.inv_span,
            half0: p.half0,
            half1: p.half1,
            soft: p.soft,
            inside: p.inside,
            outside: p.outside,
            active: p.active,
            composite: p.composite,
            mix: p.mix,
        }
    };

    let mut short = base;
    short.length = 30.0;
    short.time = 65.0;
    let mut hard = base;
    hard.softness = 0.0;
    let mut wide = base;
    wide.softness = 90.0;
    let mut alone = base;
    alone.composite_on_original = false;
    let mut nothing = base;
    nothing.time = 0.0;
    let mut faded = base;
    faded.mix = 55.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, b) in [
        ("default", base),
        ("short", short),
        ("hard", hard),
        ("wide", wide),
        ("alone", alone),
        ("time-zero", nothing),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = b.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::beam(&mut cpu, w, h, &p);
        let out = fx.beam(&ctx, &tex, w, h, &op_of(b));
        let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("beam {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "time-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the beam must draw something"),
        }
    }

    // **It tapers.** Count the pixels the beam brightened on a column near the
    // tail and on one near the head; the first must be the wider.
    let out = fx.beam(&ctx, &tex, w, h, &op_of(base));
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
    let lit_in = |x: u32| {
        (0..h)
            .filter(|y| {
                let i = ((y * w + x) * 4 + 3) as usize;
                gpu[i] > img[i] + 1e-3
            })
            .count()
    };
    assert!(
        lit_in(10) > lit_in(38),
        "the beam must be fatter at its start: {} vs {}",
        lit_in(10),
        lit_in(38)
    );

    // **Both colours appear.** The inside colour is white and the outside a
    // blue, so somewhere on the beam blue must exceed red and somewhere else
    // red must reach the inside colour's level.
    let alone_out = fx.beam(&ctx, &tex, w, h, &op_of(alone));
    let solo = readback_linear_f32(&ctx, &alone_out, w, h).unwrap();
    let mut core_seen = false;
    let mut rim_seen = false;
    for px in solo.chunks_exact(4) {
        if px[3] > 0.9 && px[0] > 0.9 && px[2] > 0.9 {
            core_seen = true;
        }
        if px[3] > 0.9 && px[2] > px[0] + 0.3 {
            rim_seen = true;
        }
    }
    assert!(core_seen, "the inside colour must reach the core");
    assert!(rim_seen, "the outside colour must be visible as a rim");
}

/// The §1.6 oracle for Lightning (docs/08 §3.74), on the smooth corpus and by
/// absolute difference.
///
/// Four claims beyond parity: **the bolt is jagged**, so it covers more than the
/// straight line between its two points would; **forking adds material**;
/// **Conductivity state changes the bolt** rather than only its brightness; and
/// **the same state gives the same bolt back**, which is §2.4 and the whole point
/// of a coordinate rather than a clock.
#[test]
fn wgsl_lightning_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::lightning::Lightning;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let mut base = Lightning::read(Params::EMPTY);
    base.origin_x = 5.0;
    base.origin_y = 27.0;
    base.direction_x = 43.0;
    base.direction_y = 5.0;
    base.core_radius = 1.0;
    base.glow_radius = 5.0;
    base.seed = 20_260_820;
    let op_of = |l: Lightning| {
        let p = l.packed();
        LightningOp {
            segments: p.segments,
            fades: p.fades,
            count: p.count,
            core_radius: p.core_radius,
            glow_radius: p.glow_radius,
            glow_opacity: p.glow_opacity,
            core_colour: p.core_colour,
            glow_colour: p.glow_colour,
            composite: p.composite,
            mix: p.mix,
        }
    };
    let run = |l: Lightning| {
        let out = fx.lightning(&ctx, &tex, w, h, None, &op_of(l));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };

    let mut strike = base;
    strike.lightning_type = 1;
    let mut omni = base;
    omni.lightning_type = 2;
    omni.origin_x = 24.0;
    omni.origin_y = 16.0;
    let mut two_way = base;
    two_way.lightning_type = 3;
    let mut unforked = base;
    unforked.forking = 0.0;
    let mut evolved = base;
    evolved.conductivity = 40.0;
    let mut alone = base;
    alone.composite_on_original = false;
    let mut faded = base;
    faded.mix = 55.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, l) in [
        ("default", base),
        ("strike", strike),
        ("omni", omni),
        ("two-way", two_way),
        ("unforked", unforked),
        ("evolved", evolved),
        ("alone", alone),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = l.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::lightning(&mut cpu, w, h, &p);
        let gpu = run(l);
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("lightning {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else {
            assert!(gpu != img, "{name}: the bolt must draw something");
        }
    }

    // **It is jagged, not straight.** A straight bolt of the same radius would
    // never leave the corridor about the line between its two ends; measure the
    // furthest lit pixel from that line and require it to be well outside.
    let solo = {
        let mut l = alone;
        l.forking = 0.0;
        run(l)
    };
    let (ax, ay) = (5.0f32, 27.0f32);
    let (bx, by) = (43.0f32, 5.0f32);
    let (dx, dy) = (bx - ax, by - ay);
    let inv = 1.0 / (dx * dx + dy * dy).sqrt();
    let mut furthest = 0.0f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4 + 3) as usize;
            if solo[i] > 0.5 {
                let (px, py) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
                furthest = furthest.max((px * dy - py * dx).abs() * inv);
            }
        }
    }
    assert!(
        furthest > 3.0,
        "the bolt must wander off the straight line, saw {furthest}"
    );

    // **Forking adds material**, and **Conductivity state reshapes the bolt.**
    let lit = |img: &[f32]| (0..img.len() / 4).filter(|i| img[i * 4 + 3] > 0.5).count();
    let forked = {
        let mut l = alone;
        l.forking = 100.0;
        run(l)
    };
    assert!(
        lit(&forked) > lit(&solo),
        "forking must add branches: {} vs {}",
        lit(&forked),
        lit(&solo)
    );
    let a = run(base);
    let b = run(evolved);
    assert!(a != b, "Conductivity state must reshape the bolt");
    assert_eq!(run(evolved), b, "the same state must give the same bolt");
}

/// The §1.6 oracle for Radio waves (docs/08 §3.75), on the smooth corpus and by
/// absolute difference — §3.71's `atan2` admission again (K-399).
///
/// Four claims beyond parity: Time 0 is the exact identity; **there are several
/// rings, not one**, which a single expanding shape could not produce; **a later
/// Time puts them further out**; and **a star covers less than the polygon it
/// came from**, which is what Star depth is for.
#[test]
fn wgsl_radio_waves_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::radio_waves::RadioWaves;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 48u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let mut base = RadioWaves::read(Params::EMPTY);
    base.centre_x = 24.0;
    base.centre_y = 24.0;
    base.expansion = 8.0;
    base.stroke_width = 1.5;
    base.time = 2.5;
    let op_of = |r: RadioWaves| {
        let p = r.packed();
        RadioWavesOp {
            centre: p.centre,
            vertex: p.vertex,
            normal: p.normal,
            period: p.period,
            rotation: p.rotation,
            spin: p.spin,
            newest: p.newest,
            count: p.count,
            time: p.time,
            period_s: p.period_s,
            expansion: p.expansion,
            lifespan: p.lifespan,
            half_width: p.half_width,
            fade_in: p.fade_in,
            fade_out: p.fade_out,
            colour: p.colour,
            opacity: p.opacity,
            composite: p.composite,
            mix: p.mix,
        }
    };
    let run = |r: RadioWaves| {
        let out = fx.radio_waves(&ctx, &tex, w, h, None, &op_of(r));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };

    let mut polygon = base;
    polygon.sides = 6;
    let mut star = base;
    star.sides = 6;
    star.star = true;
    star.star_depth = 60.0;
    let mut spun = base;
    spun.sides = 5;
    spun.spin = 90.0;
    let mut alone = base;
    alone.composite_on_original = false;
    let mut nothing = base;
    nothing.time = 0.0;
    let mut faded = base;
    faded.mix = 55.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, r) in [
        ("default", base),
        ("polygon", polygon),
        ("star", star),
        ("spun", spun),
        ("alone", alone),
        ("time-zero", nothing),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = r.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::radio_waves(&mut cpu, w, h, &p);
        let gpu = run(r);
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("radio_waves {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "time-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the waves must draw something"),
        }
    }

    // **Several rings, not one.** Walk out along one row from the centre and
    // count how many times the stroke starts.
    let solo = run(alone);
    let hits = (25..w)
        .filter(|x| {
            let i = ((24 * w + x) * 4 + 3) as usize;
            let prev = (((24 * w) + x - 1) * 4 + 3) as usize;
            solo[i] > 0.25 && solo[prev] <= 0.25
        })
        .count();
    assert!(hits >= 2, "there must be several waves, saw {hits}");

    // **A later Time puts them further out.** Set up so no wave dies inside the
    // frame — otherwise the outermost ring is the one that *aged out*, and the
    // test would be measuring Lifespan rather than Expansion.
    let furthest = |v: &[f32]| {
        (25..w)
            .rfind(|x| v[((24 * w + x) * 4 + 3) as usize] > 0.25)
            .unwrap_or(0)
    };
    let mut slow = alone;
    slow.frequency = 1.0;
    slow.expansion = 4.0;
    slow.lifespan = 8.0;
    slow.fade_in = 0.0;
    slow.fade_out = 0.0;
    let mut earlier = slow;
    earlier.time = 2.0;
    let mut later = slow;
    later.time = 3.0;
    assert!(
        furthest(&run(later)) > furthest(&run(earlier)),
        "the waves must expand with Time: {} then {}",
        furthest(&run(earlier)),
        furthest(&run(later))
    );

    // **A star covers less than its polygon**, which no parity check can see.
    let area = |v: &[f32]| (0..v.len() / 4).filter(|i| v[i * 4 + 3] > 0.25).count();
    let mut poly_alone = polygon;
    poly_alone.composite_on_original = false;
    let mut star_alone = star;
    star_alone.composite_on_original = false;
    assert!(
        area(&run(star_alone)) < area(&run(poly_alone)),
        "a star's outline must be shorter than its polygon's"
    );
}

/// The §1.6 oracle for Vegas (docs/08 §3.76), on the smooth corpus and by
/// absolute difference — the stroke is a threshold on a distance, and the
/// gradient behind it is 25 taps of one.
///
/// Three claims beyond parity: Opacity 0 is the exact identity; **the stroke
/// follows the picture**, so moving the Threshold moves it; and **Length below
/// 100 breaks it into dashes**, which is the whole effect and is invisible to a
/// parity check.
#[test]
fn wgsl_vegas_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::vegas::Vegas;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let mut base = Vegas::read(Params::EMPTY);
    base.width = 3.0;
    base.segment_length = 12.0;
    base.threshold = 55.0;
    let op_of = |v: Vegas| {
        let p = v.packed();
        VegasOp {
            from_alpha: p.from_alpha,
            level: p.level,
            half_width: p.half_width,
            band: p.band,
            inv_segment: p.inv_segment,
            duty: p.duty,
            phase: p.phase,
            colour: p.colour,
            opacity: p.opacity,
            composite: p.composite,
            mix: p.mix,
        }
    };
    let run = |v: Vegas| {
        let out = fx.vegas(&ctx, &tex, w, h, None, &op_of(v));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };

    let mut solid = base;
    solid.length = 100.0;
    let mut alpha = base;
    alpha.source = 1;
    alpha.threshold = 70.0;
    let mut marched = base;
    marched.rotation = 140.0;
    let mut soft = base;
    soft.hardness = 0.0;
    let mut alone = base;
    alone.composite_on_original = false;
    let mut off = base;
    off.opacity = 0.0;
    let mut faded = base;
    faded.mix = 55.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, v) in [
        ("default", base),
        ("solid", solid),
        ("alpha", alpha),
        ("marched", marched),
        ("soft", soft),
        ("alone", alone),
        ("opacity-zero", off),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = v.packed();
        let mut cpu = img.clone();
        lumit_core::fx::cpu::vegas(&mut cpu, w, h, &p);
        let gpu = run(v);
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("vegas {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name == "opacity-zero" || name == "mix-zero" {
            assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
        } else if name != "alpha" {
            assert!(gpu != img, "{name}: the stroke must draw something");
        }
    }

    // **The stroke follows the picture**: a different Threshold is a different
    // contour, so the lit pixels must move.
    let mut lower = base;
    lower.threshold = 35.0;
    assert!(run(lower) != run(base), "Threshold must move the contour");

    // **Length breaks the outline into dashes.** The continuous stroke must
    // light strictly more pixels than the dashed one at the same width.
    let lit = |v: &[f32]| {
        (0..v.len() / 4)
            .filter(|i| v[i * 4 + 3] > img[i * 4 + 3] + 1e-3)
            .count()
    };
    let mut solid_alone = solid;
    solid_alone.composite_on_original = true;
    let mut dashed = base;
    dashed.length = 30.0;
    assert!(
        lit(&run(solid_alone)) > lit(&run(dashed)),
        "dashes must light less of the contour than a continuous outline"
    );
}

/// The §1.6 oracle for Add grain (docs/08 §3.77), and with it the fourth reader
/// of the shared noise core.
///
/// Four claims beyond parity: Intensity 0 is the exact identity; **the grain has
/// a size**, so neighbouring pixels are correlated at Size 4 and not at Size 1;
/// **Monochrome moves the three channels together**; and **the tonal weights
/// bite**, so grain confined to the shadows leaves the highlights alone.
#[test]
fn wgsl_add_grain_matches_the_cpu_oracle() {
    use lumit_core::fx::effects::add_grain::AddGrain;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);

    let mut base = AddGrain::read(Params::EMPTY);
    base.seed = 20_260_820;
    let op_of = |g: AddGrain, tick: i32| {
        let p = g.packed(tick);
        AddGrainOp {
            amplitude: p.amplitude,
            inv_size: p.inv_size,
            softness: p.softness,
            tonal: p.tonal,
            monochrome: p.monochrome,
            seed: p.seed,
            tick: p.tick,
            mix: p.mix,
        }
    };
    let run = |g: AddGrain, tick: i32| {
        let out = fx.add_grain(&ctx, &tex, w, h, None, &op_of(g, tick));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };

    let mut coarse = base;
    coarse.size = 4.0;
    let mut hard = base;
    hard.softness = 0.0;
    let mut mono = base;
    mono.monochrome = true;
    let mut shadows_only = base;
    shadows_only.midtones = 0.0;
    shadows_only.highlights = 0.0;
    let mut strong = base;
    strong.intensity = 200.0;
    let mut quiet = base;
    quiet.intensity = 0.0;
    let mut faded = base;
    faded.mix = 55.0;
    let mut none = base;
    none.mix = 0.0;

    for (name, g) in [
        ("default", base),
        ("coarse", coarse),
        ("hard", hard),
        ("mono", mono),
        ("shadows-only", shadows_only),
        ("strong", strong),
        ("intensity-zero", quiet),
        ("mixed", faded),
        ("mix-zero", none),
    ] {
        let p = g.packed(37);
        let mut cpu = img.clone();
        lumit_core::fx::cpu::add_grain(&mut cpu, w, h, &p);
        let gpu = run(g, 37);
        let worst = worst_diff(&cpu, &gpu);
        eprintln!("add_grain {name}: worst {worst}");
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        match name {
            "intensity-zero" | "mix-zero" => {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }
            _ => assert!(gpu != img, "{name}: the grain must land"),
        }
    }

    // **The grain has a size.** Correlate each pixel's deviation with its right
    // neighbour's: at Size 4 the cells are four pixels across and the two agree
    // far more often than at Size 1.
    let agreement = |g: AddGrain| {
        let out = run(g, 37);
        let mut same = 0usize;
        let mut total = 0usize;
        for y in 0..h {
            for x in 0..w - 1 {
                let i = ((y * w + x) * 4) as usize;
                let j = ((y * w + x + 1) * 4) as usize;
                let a = out[i] - img[i];
                let b = out[j] - img[j];
                if a.abs() > 1e-5 && b.abs() > 1e-5 {
                    total += 1;
                    if (a > 0.0) == (b > 0.0) {
                        same += 1;
                    }
                }
            }
        }
        same as f32 / total.max(1) as f32
    };
    let mut fine = base;
    fine.size = 1.0;
    fine.softness = 0.0;
    let mut chunky = base;
    chunky.size = 4.0;
    chunky.softness = 0.0;
    assert!(
        agreement(chunky) > agreement(fine) + 0.15,
        "a bigger Size must give a coarser grain: {} vs {}",
        agreement(chunky),
        agreement(fine)
    );

    // **Monochrome moves the three channels together**, which is what a lane
    // rather than an average means. On a pixel with equal-ish weights the three
    // deviations must share a sign.
    let m = run(mono, 37);
    let mut checked = 0;
    for i in (0..m.len()).step_by(4) {
        if img[i + 3] < 0.5 {
            continue;
        }
        let d: Vec<f32> = (0..3).map(|c| m[i + c] - img[i + c]).collect();
        if d.iter().all(|v| v.abs() > 1e-4) {
            assert!(
                d.iter().all(|v| *v > 0.0) || d.iter().all(|v| *v < 0.0),
                "mono grain must move all three channels the same way"
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "too few pixels tested, saw {checked}");

    // **The tonal weights bite**: grain confined to the shadows must leave the
    // brightest pixels alone.
    let s = run(shadows_only, 37);
    let bright = (0..img.len() / 4)
        .filter(|i| img[i * 4 + 3] > 0.9 && img[i * 4] / img[i * 4 + 3] > 0.9)
        .collect::<Vec<_>>();
    assert!(!bright.is_empty(), "the corpus has no highlights to test");
    for i in bright {
        assert!(
            (s[i * 4] - img[i * 4]).abs() < 1e-3,
            "a highlight must be left alone when only the shadows are grained"
        );
    }
}

/// The uniform's piece array and the builder's agree (K-408, docs/08 §3.78).
#[test]
fn the_path_piece_cap_matches_the_core() {
    assert_eq!(
        PATH_PRIMITIVES,
        lumit_core::fx::cpu::PATH_PRIMITIVES,
        "the uniform's piece array and the builder's disagree"
    );
}

/// A mask an effect can be pointed at: an ellipse, at the size the oracle
/// corpus is (K-408). Closed, so it has an inside to fill.
fn oracle_ellipse(w: u32, h: u32) -> lumit_core::mask::MaskPolyline {
    let m = lumit_core::mask::Mask::ellipse(
        f64::from(w) * 0.5,
        f64::from(h) * 0.5,
        f64::from(w) * 0.34,
        f64::from(h) * 0.34,
    );
    lumit_core::mask::mask_path_at(std::slice::from_ref(&m), None, true, 0.0)
}

/// An open bezier squiggle across the frame — the other shape the seam can
/// hand over, and the one that proves an *open* path is walked end to end
/// rather than closed behind the effect's back.
fn oracle_squiggle(w: u32, h: u32) -> lumit_core::mask::MaskPolyline {
    use lumit_core::mask::{BezierPath, Vertex};
    let (fw, fh) = (f64::from(w), f64::from(h));
    let v = |x: f64, y: f64, t: f64| Vertex {
        pos: (x, y),
        tan_in: (-t, 0.0),
        tan_out: (t, 0.0),
    };
    let path = BezierPath {
        vertices: vec![
            v(fw * 0.1, fh * 0.75, fw * 0.15),
            v(fw * 0.5, fh * 0.2, fw * 0.15),
            v(fw * 0.9, fh * 0.8, fw * 0.15),
        ],
        closed: false,
    };
    lumit_core::mask::flatten_path(&path, lumit_core::mask::MASK_PATH_TOLERANCE_PX)
}

/// The §1.6 oracle for the shared path drawing (docs/08 §3.78 Scribble, §3.79
/// Stroke, §3.76's Mask/Path source) — one kernel, so one parity test, run over
/// every shape the three consumers produce.
///
/// By absolute difference on the smooth corpus, for Beam's reason (K-399): the
/// coverage is a *threshold on a distance*, which magnifies a fused
/// multiply-add exactly as a sample position does.
///
/// Claims beyond parity, each invisible to a parity check because a kernel that
/// drew nothing would pass one:
///
/// - **Something is drawn** on a real mask, and it is inside the mask.
/// - **An empty polyline is the bit-exact identity** — the documented no-op for
///   an unset row, a deleted mask, a layer with no masks (docs/08 §1.2).
/// - **Start and End trim the drawing**, so a stroke to 50 % covers about half
///   of what the whole one covers.
/// - **Reveal original keeps the picture and drops the rest of it**, which is
///   the paint style no other effect in the catalogue has.
/// - **Scribble's waver moves with its tick**, and holds still on Static.
#[test]
fn wgsl_path_draw_matches_the_cpu_oracle() {
    use lumit_core::fx::cpu;
    use lumit_core::fx::effects::{scribble::Scribble, stroke::Stroke, vegas::Vegas};
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let tex = upload_linear_f32(&ctx, &img, w, h);
    let ellipse = oracle_ellipse(w, h);
    let squiggle = oracle_squiggle(w, h);
    let empty = lumit_core::mask::MaskPolyline::default();

    let op_of = |p: &cpu::PathDrawParams| PathDrawOp {
        segments: p.segments,
        arcs: p.arcs,
        count: p.count,
        half_width: p.half_width,
        band: p.band,
        inv_segment: p.inv_segment,
        duty: p.duty,
        phase: p.phase,
        wiggle_amp: p.wiggle_amp,
        wiggle_freq: p.wiggle_freq,
        wiggle_tick: p.wiggle_tick,
        seed: p.seed,
        colour: p.colour,
        opacity: p.opacity,
        style: p.style,
        mix: p.mix,
    };
    let run = |p: &cpu::PathDrawParams| {
        let out = fx.path_draw(&ctx, &tex, w, h, None, &op_of(p));
        readback_linear_f32(&ctx, &out, w, h).unwrap()
    };
    let covered = |a: &[f32]| -> usize {
        a.chunks_exact(4)
            .zip(img.chunks_exact(4))
            .filter(|(x, y)| (x[0] - y[0]).abs() + (x[1] - y[1]).abs() > 1e-3)
            .count()
    };

    // ---- Scribble: a hatch inside the ellipse, at three densities.
    let mut base = Scribble::read(Params::EMPTY);
    base.spacing = 5.0;
    base.stroke_width = 1.5;
    let mut fine = base;
    fine.spacing = 2.0;
    let mut steep = base;
    steep.angle = 78.0;
    let mut alone = base;
    alone.composite_on_original = false;
    let mut half = base;
    half.end = 50.0;
    let mut moved = base;
    moved.wiggle_type = 2;
    // The containment claim below is made against this one: Path overlap is
    // *meant* to run past the edge, so the shape that must stay inside is the
    // one told not to.
    let mut tight = base;
    tight.path_overlap = 0.0;

    // ---- Stroke: the continuous brush, and the dotted one.
    let mut brush = Stroke::read(Params::EMPTY);
    brush.brush_size = 5.0;
    let mut dotted = brush;
    dotted.spacing = 220.0;
    let mut soft = brush;
    soft.hardness = 0.0;
    let mut trimmed = brush;
    trimmed.start = 20.0;
    trimmed.end = 60.0;
    let mut lifted = brush;
    lifted.paint_style = 1;
    let mut revealed = brush;
    revealed.paint_style = 2;

    // ---- Vegas on a path: the dashes it could not march before K-408.
    let mut dashes = Vegas::read(Params::EMPTY);
    dashes.source = Vegas::SOURCE_MASK_PATH;
    dashes.segment_length = 14.0;
    dashes.width = 3.0;
    let mut solid = dashes;
    solid.length = 100.0;
    let mut marched = dashes;
    marched.rotation = 137.0;

    let cases: Vec<(&str, cpu::PathDrawParams)> = vec![
        ("scribble", base.packed(&ellipse, 1.0, 0.0)),
        ("scribble-fine", fine.packed(&ellipse, 1.0, 0.0)),
        ("scribble-steep", steep.packed(&ellipse, 1.0, 0.0)),
        ("scribble-alone", alone.packed(&ellipse, 1.0, 0.0)),
        ("scribble-half", half.packed(&ellipse, 1.0, 0.0)),
        ("scribble-moved", moved.packed(&ellipse, 1.0, 2.5)),
        ("scribble-tight", tight.packed(&ellipse, 1.0, 0.0)),
        ("scribble-empty", base.packed(&empty, 1.0, 0.0)),
        ("stroke", brush.packed(&ellipse, 1.0)),
        ("stroke-open", brush.packed(&squiggle, 1.0)),
        ("stroke-dotted", dotted.packed(&ellipse, 1.0)),
        ("stroke-soft", soft.packed(&ellipse, 1.0)),
        ("stroke-trimmed", trimmed.packed(&ellipse, 1.0)),
        ("stroke-transparent", lifted.packed(&ellipse, 1.0)),
        ("stroke-reveal", revealed.packed(&ellipse, 1.0)),
        ("stroke-empty", brush.packed(&empty, 1.0)),
        ("vegas-dashes", dashes.path_packed(&ellipse, 1.0)),
        ("vegas-solid", solid.path_packed(&ellipse, 1.0)),
        ("vegas-marched", marched.path_packed(&ellipse, 1.0)),
        ("vegas-open", dashes.path_packed(&squiggle, 1.0)),
        ("vegas-empty", dashes.path_packed(&empty, 1.0)),
    ];

    for (name, p) in &cases {
        let mut cpu_img = img.clone();
        cpu::path_draw(&mut cpu_img, w, h, p);
        let gpu = run(p);
        let worst = worst_diff(&cpu_img, &gpu);
        eprintln!("path draw {name}: worst {worst}, {} pieces", p.count);
        assert!(worst < 2e-3, "{name}: worst diff {worst}");
        if name.ends_with("-empty") {
            assert_eq!(
                gpu, img,
                "{name}: an absent mask must be the exact identity"
            );
            assert_eq!(p.count, 0, "{name}: an absent mask must build no geometry");
        } else {
            assert!(p.count > 0, "{name}: nothing was built to draw");
            assert!(covered(&gpu) > 20, "{name}: nothing was drawn");
        }
    }

    let pick = |name: &str| {
        cases
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, p)| *p)
            .expect("case")
    };

    // **The scribble stays inside its mask.** Every pixel it touched must be
    // within the ellipse it was told to fill — allowing the waver, half a stroke
    // width and a pixel of anti-aliasing, and with Path overlap at zero, since
    // running past the edge is exactly what that control is for.
    let filled = run(&pick("scribble-tight"));
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let (rx, ry) = (w as f32 * 0.34, h as f32 * 0.34);
    let tight_p = pick("scribble-tight");
    let slack = tight_p.wiggle_amp + tight_p.half_width + 1.0;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (filled[i] - img[i]).abs() + (filled[i + 1] - img[i + 1]).abs() < 1e-3 {
                continue;
            }
            let (dx, dy) = (
                (x as f32 + 0.5 - cx) / (rx + slack),
                (y as f32 + 0.5 - cy) / (ry + slack),
            );
            assert!(
                dx * dx + dy * dy <= 1.0 + 1e-3,
                "the scribble drew at ({x},{y}), outside the mask it was given"
            );
        }
    }

    // **Start and End trim it.** Half the drawing covers about half as much.
    let whole = covered(&run(&pick("scribble")));
    let part = covered(&run(&pick("scribble-half")));
    assert!(
        part * 2 < whole * 3 && part * 3 > whole,
        "End 50 covered {part} of the whole drawing's {whole}"
    );

    // **The dots come apart.** A stroke spaced well past its own width must
    // cover less than the continuous one it would otherwise be.
    assert!(
        covered(&run(&pick("stroke-dotted"))) < covered(&run(&pick("stroke"))),
        "a dotted stroke must cover less than a continuous one"
    );

    // **Reveal original is a hole in reverse**: what the brush did not touch is
    // gone, and what it did keeps the picture rather than taking the colour.
    let reveal = run(&pick("stroke-reveal"));
    let mut kept = 0usize;
    for i in (0..reveal.len()).step_by(4) {
        if reveal[i + 3] > img[i + 3] * 0.5 && img[i + 3] > 0.5 {
            // Where the stroke is, the colour is the picture's own — never the
            // brush colour, which is white and would have shown as a lift.
            assert!(
                reveal[i] <= img[i] + 1e-3,
                "Reveal original brightened a pixel; it must only take away"
            );
            kept += 1;
        }
    }
    assert!(kept > 20, "Reveal original kept nothing at all");

    // **The waver holds still on Static and moves on Wiggly.** Same seed, same
    // shape, two ticks: Static is bit-identical, Wiggly is not.
    let held = base.packed(&ellipse, 1.0, 3.5);
    assert_eq!(
        held.wiggle_tick, 0.0,
        "Static's tick must be pinned, whatever the clock says"
    );
    assert_eq!(
        run(&held),
        run(&pick("scribble")),
        "a Static waver must not move with time"
    );
    assert_ne!(
        run(&moved.packed(&ellipse, 1.0, 9.0)),
        run(&pick("scribble-moved")),
        "a Wiggly waver must move with time"
    );
}

// ---------------------------------------------------------------------------
// The matte scales the amount (K-426, docs/08 §2.6): the blur, sharpen and
// colour claims.
// ---------------------------------------------------------------------------

/// One effect's claim on the matte, checked four ways by [`check_matte_claim`].
struct MatteClaim<'a> {
    name: &'static str,
    w: u32,
    h: u32,
    /// Scene-linear premultiplied RGBA, fp16-quantised by the harness.
    img: &'a [f32],
    /// The §1.6 oracle with a matte; an empty one is the unmatted path.
    cpu: &'a dyn Fn(&mut [f32], &[f32]),
    /// The unmatted oracle as it was before the claim — what an empty matte
    /// must reproduce to the byte (K-258).
    plain: &'a dyn Fn(&mut [f32]),
    /// The GPU pass, the matte bound or not.
    gpu: &'a dyn Fn(&wgpu::Texture, Option<&wgpu::Texture>) -> wgpu::Texture,
    /// Absolute parity tolerance. The corpus carries an HDR spike at 6.0, so
    /// the Moderate-class epsilon of the matted blur applies across the board.
    tol: f32,
}

/// **Every claim is held to the same four facts** (K-426, K-258, §1.6).
///
/// 1. Under a left-to-right ramp matte the WGSL kernel agrees with the CPU
///    oracle op-for-op, and is bit-stable run to run.
/// 2. An empty matte IS the pre-claim function — the oracle's empty-matte path
///    reproduces the old function to the byte, and the GPU's unbound path
///    tracks it.
/// 3. A flat half matte is **not** the generic dissolve: the picture the
///    kernel makes differs from `matte_mix(full, input, ½)`, which is the one
///    thing a strength matte cannot do and the whole reason the effect claims
///    its matte instead of taking the dissolve.
/// 4. At that flat matte the two paths still agree.
fn check_matte_claim(ctx: &GpuContext, c: &MatteClaim<'_>) {
    let n = (c.w * c.h) as usize;
    let q = |v: &[f32]| -> Vec<f32> { v.iter().map(|x| f16_to_f32(f16_bits(*x))).collect() };
    let readback = |t: &wgpu::Texture| readback_linear_f32(ctx, t, c.w, c.h).unwrap();
    let img = q(c.img);
    let tex = upload_linear_f32(ctx, &img, c.w, c.h);

    // 1. Parity and stability under a ramp.
    let ramp: Vec<f32> = (0..n)
        .flat_map(|i| {
            let k = (i % c.w as usize) as f32 / (c.w - 1) as f32;
            [k, k, k, 1.0]
        })
        .collect();
    let ramp = q(&ramp);
    let mtex = upload_linear_f32(ctx, &ramp, c.w, c.h);
    let mut cpu = img.clone();
    (c.cpu)(&mut cpu, &ramp);
    let gpu = readback(&(c.gpu)(&tex, Some(&mtex)));
    let worst = worst_diff(&cpu, &gpu);
    assert!(
        worst < c.tol,
        "{}: matted kernel drifted from the oracle by {worst}",
        c.name
    );
    assert_eq!(
        gpu,
        readback(&(c.gpu)(&tex, Some(&mtex))),
        "{}: the matted kernel must be bit-stable",
        c.name
    );

    // 2. An empty matte is the old function.
    let mut empty = img.clone();
    (c.cpu)(&mut empty, &[]);
    let mut plain = img.clone();
    (c.plain)(&mut plain);
    assert_eq!(
        empty, plain,
        "{}: the oracle's empty-matte path must BE the pre-claim function",
        c.name
    );
    let unbound = readback(&(c.gpu)(&tex, None));
    let worst = worst_diff(&plain, &unbound);
    assert!(
        worst < c.tol,
        "{}: the unbound GPU path drifted from the unmatted oracle by {worst}",
        c.name
    );
    assert_ne!(
        plain, img,
        "{}: the effect must actually do something",
        c.name
    );

    // 3. A flat half matte is not the dissolve.
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5, 0.5, 0.5, 1.0]).collect();
    let mut half = img.clone();
    (c.cpu)(&mut half, &flat);
    let mut dissolved = plain.clone();
    lumit_core::fx::cpu::matte_mix(&mut dissolved, &img, &flat, false);
    let apart = worst_diff(&half, &dissolved);
    assert!(
        apart > 1e-3,
        "{}: a half matte gave the generic dissolve (worst difference {apart}) — \
         the matte is not scaling the amount inside the maths, which is the \
         whole reason this effect claims it",
        c.name
    );

    // 4. And the two paths agree there too.
    let ftex = upload_linear_f32(ctx, &flat, c.w, c.h);
    let gpu_half = readback(&(c.gpu)(&tex, Some(&ftex)));
    let worst = worst_diff(&half, &gpu_half);
    assert!(
        worst < c.tol,
        "{}: at a flat half matte the kernel drifted from the oracle by {worst}",
        c.name
    );
}

/// A corpus for the colour claims: a gradient between two saturated colours,
/// a transparent half, partial alpha and an HDR spike — and, in row 1, a run
/// of near-primary colours whose grades clip, which is where scaling an
/// amount and fading a clipped result part company.
fn claim_corpus(w: u32, h: u32) -> Vec<f32> {
    let mut img = corpus_with_partials(w, h);
    let q = |v: f32| f16_to_f32(f16_bits(v));
    for x in 0..w.min(8) {
        let i = ((w + x) * 4) as usize;
        let t = x as f32 / 8.0;
        img[i] = q(1.0);
        img[i + 1] = q(0.1 * t);
        img[i + 2] = q(0.05);
        img[i + 3] = 1.0;
    }
    img
}

#[test]
fn the_matte_scales_the_directional_blur_length() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let (length, angle, edge, mix) = (9.5f32, 33.0f32, 1u32, 0.8f32);
    let (dx, dy) = lumit_core::fx::rgb_split_offset(1.0, angle);
    let op = DirBlurOp {
        dx,
        dy,
        length_px: length,
        taps: lumit_core::fx::cpu::dir_blur_taps(length),
        edge,
        mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "directional_blur",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::blur_directional_matted(px, w, h, length, angle, edge, mix, m);
            },
            plain: &|px| lumit_core::fx::cpu::blur_directional(px, w, h, length, angle, edge, mix),
            gpu: &|t, m| fx.dir_blur(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_radial_blur_amount() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    for spin in [false, true] {
        // Raster px since K-558: four tenths across the 32x24 corpus and six
        // tenths down it.
        let (centre, amount, edge, mix) = ([12.8f32, 14.4f32], 12.0f32, 1u32, 1.0f32);
        let op = RadialBlurOp {
            centre_px: centre,
            amount_px: amount,
            taps: lumit_core::fx::cpu::radial_blur_taps(amount),
            spin,
            edge,
            mix,
        };
        check_matte_claim(
            &ctx,
            &MatteClaim {
                name: if spin {
                    "radial_blur (spin)"
                } else {
                    "radial_blur (zoom)"
                },
                w,
                h,
                img: &img,
                cpu: &|px, m| {
                    lumit_core::fx::cpu::blur_radial_matted(
                        px, w, h, centre, amount, spin, edge, mix, m,
                    );
                },
                plain: &|px| {
                    lumit_core::fx::cpu::blur_radial(px, w, h, centre, amount, spin, edge, mix);
                },
                gpu: &|t, m| fx.radial_blur(&ctx, t, w, h, m, &op),
                tol: 2e-2,
            },
        );
    }
}

#[test]
fn the_matte_scales_the_unsharp_mask_amount() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // An Amount big enough to undershoot past zero at the spike: that clip is
    // where scaling the Amount and fading a clipped result differ.
    let op = SharpenOp {
        amount: 3.0,
        radius_px: 3.0,
        threshold: 0.0,
        luma_only: false,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "sharpen",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::sharpen_matted(
                    px,
                    w,
                    h,
                    op.amount,
                    op.radius_px,
                    op.threshold,
                    op.luma_only,
                    op.mix,
                    m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::sharpen(
                    px,
                    w,
                    h,
                    op.amount,
                    op.radius_px,
                    op.threshold,
                    op.luma_only,
                    op.mix,
                );
            },
            gpu: &|t, m| fx.sharpen(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_sharpen_amount() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let op = SharpenSimpleOp {
        amount: 2.0,
        radius: 1.0,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "sharpen_simple",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::sharpen_simple_matted(
                    px, w, h, op.amount, op.radius, op.mix, m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::sharpen_simple(px, w, h, op.amount, op.radius, op.mix)
            },
            gpu: &|t, m| fx.sharpen_simple(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_all_four_channel_blur_radii() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let op = ChannelBlurOp {
        radii: [6.0, 2.0, 0.0, 4.0],
        edge: 1,
        mix: 0.9,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "channel_blur",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::channel_blur_matted(px, w, h, op.radii, op.edge, op.mix, m);
            },
            plain: &|px| lumit_core::fx::cpu::channel_blur(px, w, h, op.radii, op.edge, op.mix),
            gpu: &|t, m| fx.channel_blur(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_exposure_stops_toward_zero() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let stops = 2.0f32;
    let op = ExposureOp {
        stops,
        factor: 2f64.powf(f64::from(stops)) as f32,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "exposure",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::exposure_matted(px, op.factor, stops, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::exposure(px, op.factor, op.mix),
            gpu: &|t, m| fx.exposure(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
    // A half matte on +2 stops is +1 stop — x2, not the dissolve's x2.5.
    let mut px = vec![0.25f32, 0.25, 0.25, 1.0];
    let flat = [0.5f32, 0.5, 0.5, 1.0];
    lumit_core::fx::cpu::exposure_matted(&mut px, op.factor, stops, 1.0, &flat);
    assert!(
        (px[0] - 0.5).abs() < 1e-6,
        "half of +2 stops must be +1 stop, got {}",
        px[0]
    );
}

#[test]
fn the_matte_pulls_saturation_toward_neutral() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    // 300 %: the near-primaries in row 1 clip to zero in full, which a fade
    // keeps at a fraction of the input and a scaled Saturation does not.
    let op = SaturationOp {
        saturation: 3.0,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "saturation",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::saturate_matted(px, op.saturation, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::saturate(px, op.saturation, op.mix),
            gpu: &|t, m| fx.saturation(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_pulls_gamma_toward_one() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let op = GammaOp {
        gamma: 2.0,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "gamma",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::gamma_matted(px, op.gamma, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::gamma(px, op.gamma, op.mix),
            gpu: &|t, m| fx.gamma(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
    // The owner's worked example: a half matte on Gamma 2 is pow(x, 1/1.5),
    // not lerp(x, pow(x, 1/2), ½).
    let x = 0.25f32;
    let mut px = vec![x, x, x, 1.0];
    lumit_core::fx::cpu::gamma_matted(&mut px, 2.0, 1.0, &[0.5, 0.5, 0.5, 1.0]);
    let want = x.powf(1.0 / 1.5);
    let lerp = 0.5 * x + 0.5 * x.sqrt();
    assert!(
        (px[0] - want).abs() < 1e-6,
        "got {}, want pow(x, 1/1.5) = {want}",
        px[0]
    );
    assert!(
        (px[0] - lerp).abs() > 1e-3,
        "that is the dissolve's {lerp}, not a gentler curve"
    );
}

#[test]
fn the_matte_scales_temperature_toward_zero() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    // 150: past the blue gain's floor, where rebuilding the gains from a
    // smaller Temperature is not a lerp of the floored gains.
    let t = 1.5f32;
    let (gain_r, gain_b) = lumit_core::fx::cpu::temperature_gains(t);
    let op = TemperatureOp {
        t,
        gain_r,
        gain_b,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "temperature",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::temperature_matted(px, gain_r, gain_b, t, op.mix, m);
            },
            plain: &|px| lumit_core::fx::cpu::temperature(px, gain_r, gain_b, op.mix),
            gpu: &|tex, m| fx.temperature(&ctx, tex, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_vibrancy_amount() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let op = VibrancyOp {
        amount: 3.0,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "vibrancy",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::vibrance_matted(px, op.amount, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::vibrance(px, op.amount, op.mix),
            gpu: &|t, m| fx.vibrancy(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_hue_shift_angle() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    for preserve in [true, false] {
        let deg = 90.0f32;
        let m = if preserve {
            lumit_core::fx::hue_matrix(f64::from(deg))
        } else {
            lumit_core::fx::hue_matrix_rgb(f64::from(deg))
        };
        let op = HueShiftOp {
            angle_rad: deg.to_radians(),
            preserve,
            m,
            mix: 1.0,
        };
        check_matte_claim(
            &ctx,
            &MatteClaim {
                name: if preserve {
                    "hue_shift"
                } else {
                    "hue_shift (rgb)"
                },
                w,
                h,
                img: &img,
                cpu: &|px, mt| {
                    lumit_core::fx::cpu::hue_shift_matted(
                        px,
                        m,
                        op.angle_rad,
                        preserve,
                        op.mix,
                        mt,
                    );
                },
                plain: &|px| lumit_core::fx::cpu::hue_shift(px, m, op.mix),
                gpu: &|t, mt| fx.hue_shift(&ctx, t, w, h, mt, &op),
                tol: 5e-2,
            },
        );
    }
    // A half matte on 90° is the 45° matrix, to the precision of f32
    // trigonometry: the hue genuinely turns half way rather than fading.
    let mut px = vec![1.0f32, 0.0, 0.0, 1.0];
    lumit_core::fx::cpu::hue_shift_matted(
        &mut px,
        lumit_core::fx::hue_matrix(90.0),
        90f32.to_radians(),
        true,
        1.0,
        &[0.5, 0.5, 0.5, 1.0],
    );
    let mut want = vec![1.0f32, 0.0, 0.0, 1.0];
    lumit_core::fx::cpu::hue_shift(&mut want, lumit_core::fx::hue_matrix(45.0), 1.0);
    assert!(
        worst_diff(&px, &want) < 1e-5,
        "half of 90° must be the 45° turn: got {px:?}, want {want:?}"
    );
}

#[test]
fn the_matte_pulls_brightness_and_contrast_toward_neutral() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    // Both set: scaling the pair is `b·k` through `(1 + (c − 1)·k)`, which a
    // fade of the finished grade is not (with one of them neutral it would be).
    let op = BrightnessOp {
        b: 0.3,
        k: 1.8,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "brightness",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::brightness_matted(px, op.b, op.k, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::brightness(px, op.b, op.k, op.mix),
            gpu: &|t, m| fx.brightness(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_pulls_colour_balance_toward_neutral() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let op = ColourBalanceOp {
        lift: [0.05, 0.0, -0.05],
        gamma: [1.3, 1.0, 0.8],
        gain: [1.2, 1.0, 0.9],
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "colour_balance",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::colour_balance_matted(
                    px, op.lift, op.gamma, op.gain, op.mix, m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::colour_balance(px, op.lift, op.gamma, op.gain, op.mix);
            },
            gpu: &|t, m| fx.colour_balance(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_every_hue_and_saturation_range_toward_zero() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let mut bands = [[0.0f32; 4]; 7];
    bands[0] = [120.0, 40.0, -20.0, 0.0];
    bands[1] = [-60.0, 80.0, 10.0, 0.0];
    bands[3] = [30.0, -50.0, 0.0, 0.0];
    let op = HueSaturationOp { bands, mix: 1.0 };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "hue_saturation",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::hue_saturation_matted(px, bands, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::hue_saturation(px, bands, op.mix),
            gpu: &|t, m| fx.hue_saturation(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_photo_filter_density() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    // Preserve luminosity on: the luma put back depends on how dark the glass
    // was, which is what makes thinner glass a different picture from a fade.
    let p = lumit_core::fx::cpu::PhotoFilterParams {
        filter: [0.9, 0.5, 0.1],
        density: 0.8,
        preserve: 1.0,
        mix: 1.0,
    };
    let op = PhotoFilterOp {
        filter: p.filter,
        density: p.density,
        preserve: p.preserve,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "photo_filter",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::photo_filter_matted(px, &p, m),
            plain: &|px| lumit_core::fx::cpu::photo_filter(px, &p),
            gpu: &|t, m| fx.photo_filter(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_scales_shadow_and_highlight_amounts() {
    use lumit_core::fx::effects::shadow_highlight::ShadowHighlight;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let mut s = ShadowHighlight::read(Params::EMPTY);
    s.radius = 3.0;
    s.shadow_amount = 100.0;
    s.highlight_amount = 100.0;
    s.shadow_tonal_width = 100.0;
    let p = s.packed();
    let op = ShadowHighlightOp {
        shadow: p.shadow,
        highlight: p.highlight,
        shadow_width: p.shadow_width,
        highlight_width: p.highlight_width,
        radius_px: p.radius_px,
        contrast: p.contrast,
        colour_correction: p.colour_correction,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "shadow_highlight",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::shadow_highlight_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::shadow_highlight(px, w, h, &p),
            gpu: &|t, m| fx.shadow_highlight(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
}

#[test]
fn the_matte_pulls_posterize_levels_toward_256() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    let op = PosterizeOp { n: 3.0, mix: 1.0 };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "posterize",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::posterize_matted(px, op.n, op.mix, m),
            plain: &|px| lumit_core::fx::cpu::posterize(px, op.n, op.mix),
            gpu: &|t, m| fx.posterize(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
    // A black matte is 256 levels: the 8-bit ladder, which on this corpus is
    // a step nobody can see rather than the untouched picture to the bit.
    let mut px = img.clone();
    let black: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [0.0, 0.0, 0.0, 1.0])
        .collect();
    lumit_core::fx::cpu::posterize_matted(&mut px, op.n, op.mix, &black);
    let worst = worst_diff(&px, &img);
    assert!(
        worst < 1.0 / 64.0,
        "a black matte must be a step too fine to see, not {worst}"
    );
}

#[test]
fn the_matte_scales_the_threshold_level() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = claim_corpus(w, h);
    // Softness 10, so the crossing is a ramp rather than a step: a hard cut
    // turns a last-bit disagreement about the level into a whole white pixel,
    // which would make the parity check a coin toss on the pixels that land on
    // the line (§3.59 decision 2). The claim is in where the crossing sits, and
    // a soft crossing tests that more strictly, not less.
    let op = ThresholdOp {
        level: 0.6,
        half_width: 0.05,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "threshold",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::threshold_matted(px, op.level, op.half_width, op.mix, m)
            },
            plain: &|px| lumit_core::fx::cpu::threshold(px, op.level, op.half_width, op.mix),
            gpu: &|t, m| fx.threshold(&ctx, t, w, h, m, &op),
            tol: 5e-2,
        },
    );
    // A black matte cuts at level 0: every lit pixel is above it, so the
    // picture goes white where it has any light at all — the far end of a cut
    // that moves, and nothing a strength dissolve could produce (K-559).
    let mut px = img.clone();
    let black: Vec<f32> = (0..(w * h) as usize)
        .flat_map(|_| [0.0, 0.0, 0.0, 1.0])
        .collect();
    lumit_core::fx::cpu::threshold_matted(&mut px, op.level, op.half_width, op.mix, &black);
    let lit = px
        .chunks_exact(4)
        .zip(img.chunks_exact(4))
        .filter(|(o, i)| i[3] > 0.0 && i[0].max(i[1]).max(i[2]) > 0.01 && o[0] >= o[3] * 0.99)
        .count();
    assert!(
        lit > 100,
        "a black matte must cut at 0, whitening every lit pixel — only {lit} came back white"
    );
}

// ---------------------------------------------------------------------------
// The matte scales the displacement (K-427, docs/08 §2.6): the distortion
// claims. Every one runs through `check_matte_claim`, so each is held to the
// same four facts as the blur and colour claims — parity under a ramp matte,
// the empty matte equal to the old function to the byte, a half matte that is
// NOT the generic dissolve, and parity there too.
// ---------------------------------------------------------------------------

#[test]
fn the_matte_scales_the_rgb_split_amount() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let tints = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (amount, angle, scale, mix) = (4.0f32, 33.0f32, [1.0f32, 0.0, 1.0], 1.0f32);
    let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, angle);
    let op = RgbSplitOp {
        dx,
        dy,
        scale,
        tints,
        mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "rgb_split (classic)",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::rgb_split_matted(
                    px, w, h, amount, angle, scale, tints, mix, m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::rgb_split(px, w, h, amount, angle, scale, tints, mix);
            },
            gpu: &|t, m| fx.rgb_split(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
    // The Wavelength mode runs its own kernel and claims the same Amount.
    let samples = 16i32;
    let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
    let sop = SpectralSplitOp {
        dx,
        dy,
        amount_px: amount,
        radial: false,
        basis,
        count,
        mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "rgb_split (wavelength)",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::spectral_split_matted(
                    px, w, h, amount, angle, false, samples, tints, mix, m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::spectral_split(
                    px, w, h, amount, angle, false, samples, tints, mix,
                );
            },
            gpu: &|t, m| fx.spectral_split(&ctx, t, w, h, m, &sop),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_chromatic_aberration_amount() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let tints = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (amount, mix) = (10.0f32, 1.0f32);
    let op = ChromaticAberrationOp {
        amount_px: amount,
        tints,
        mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "chromatic_aberration (classic)",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::chromatic_aberration_matted(px, w, h, amount, tints, mix, m);
            },
            plain: &|px| lumit_core::fx::cpu::chromatic_aberration(px, w, h, amount, tints, mix),
            gpu: &|t, m| fx.chromatic_aberration(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
    // Its Wavelength mode is the radial spectral split.
    let samples = 16i32;
    let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, 0.0);
    let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
    let sop = SpectralSplitOp {
        dx,
        dy,
        amount_px: amount,
        radial: true,
        basis,
        count,
        mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "chromatic_aberration (wavelength)",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::spectral_split_matted(
                    px, w, h, amount, 0.0, true, samples, tints, mix, m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::spectral_split(
                    px, w, h, amount, 0.0, true, samples, tints, mix,
                );
            },
            gpu: &|t, m| fx.spectral_split(&ctx, t, w, h, m, &sop),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_shake_displacement() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    // A wobble with all three parts, so the displacement the matte scales is
    // a shove, a twist and a zoom together.
    let wobble = lumit_core::fx::ShakeSample {
        offset_px: [5.0, -3.0],
        rotation_deg: 6.0,
        zoom: 1.04,
    };
    let (edge, mix) = (1u32, 1.0f32);
    let (anchor, position, scale, rot) =
        lumit_core::fx::shake_affine(w, h, wobble.offset_px, wobble.rotation_deg, wobble.zoom);
    let (m, off, opacity) =
        lumit_core::fx::transform_op(anchor, position, scale, rot, lumit_core::fx::NO_SKEW, 1.0);
    let op = TransformOp {
        m,
        off,
        opacity,
        mix,
        edge,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "shake (plain)",
            w,
            h,
            img: &img,
            cpu: &|px, mt| {
                lumit_core::fx::cpu::transform_matted(
                    px,
                    w,
                    h,
                    anchor,
                    position,
                    scale,
                    rot,
                    lumit_core::fx::NO_SKEW,
                    edge,
                    1.0,
                    mix,
                    mt,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::transform(
                    px,
                    w,
                    h,
                    anchor,
                    position,
                    scale,
                    rot,
                    lumit_core::fx::NO_SKEW,
                    edge,
                    1.0,
                    mix,
                );
            },
            gpu: &|t, mt| fx.transform(&ctx, t, w, h, mt, &op),
            tol: 2e-2,
        },
    );
    // A pure shove under a flat half matte is exactly half the Amplitude: the
    // picture the matte draws is the one a shake at half the Amplitude draws,
    // not a blend of the shoved picture over the still one.
    let shove = [6.0f32, 4.0];
    let (a1, p1, s1, r1) = lumit_core::fx::shake_affine(w, h, shove, 0.0, 1.0);
    let (a2, p2, s2, r2) = lumit_core::fx::shake_affine(w, h, [3.0, 2.0], 0.0, 1.0);
    let n = (w * h) as usize;
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5f32, 0.5, 0.5, 1.0]).collect();
    let mut matted = img.clone();
    lumit_core::fx::cpu::transform_matted(
        &mut matted,
        w,
        h,
        a1,
        p1,
        s1,
        r1,
        lumit_core::fx::NO_SKEW,
        edge,
        1.0,
        1.0,
        &flat,
    );
    let mut half = img.clone();
    lumit_core::fx::cpu::transform(
        &mut half,
        w,
        h,
        a2,
        p2,
        s2,
        r2,
        lumit_core::fx::NO_SKEW,
        edge,
        1.0,
        1.0,
    );
    assert!(
        worst_diff(&matted, &half) < 1e-5,
        "a half matte on a 6,4 shove must be the 3,2 shove"
    );

    // The shake's own motion blur: every sub-frame tap scales the same way.
    let mut samples = [lumit_core::fx::ShakeSample::IDENTITY; SHAKE_MB_SAMPLES];
    for (i, s) in samples.iter_mut().enumerate() {
        let t = i as f32 / (SHAKE_MB_SAMPLES - 1) as f32 - 0.5;
        *s = lumit_core::fx::ShakeSample {
            offset_px: [5.0 + 4.0 * t, -3.0 - 2.0 * t],
            rotation_deg: 6.0 * (1.0 + t),
            zoom: 1.04 + 0.02 * t,
        };
    }
    let mut taps = [ShakeMbTap {
        m: [1.0, 0.0, 0.0, 1.0],
        off: [0.0, 0.0],
    }; SHAKE_MB_SAMPLES];
    let mut ops = [([1.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0]); SHAKE_MB_SAMPLES];
    for ((t, o), s) in taps.iter_mut().zip(ops.iter_mut()).zip(samples.iter()) {
        let (anchor, position, scale, rot) =
            lumit_core::fx::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
        let (m, off, _opacity) = lumit_core::fx::transform_op(
            anchor,
            position,
            scale,
            rot,
            lumit_core::fx::NO_SKEW,
            1.0,
        );
        *t = ShakeMbTap { m, off };
        *o = (m, off);
    }
    let mbop = ShakeMbOp {
        taps,
        count: SHAKE_MB_SAMPLES as u32,
        edge,
        mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "shake (motion blur)",
            w,
            h,
            img: &img,
            cpu: &|px, mt| {
                lumit_core::fx::cpu::transform_average_matted(px, w, h, &ops, edge, mix, mt);
            },
            plain: &|px| lumit_core::fx::cpu::transform_average(px, w, h, &ops, edge, mix),
            gpu: &|t, mt| fx.shake_mb(&ctx, t, w, h, mt, &mbop),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_block_glitch_intensity() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let op = BlockGlitchOp {
        intensity: 0.8,
        seed: 11,
        tick: 4,
        block_size_px: 6.0,
        jitter_frac: 0.3,
        amount_px: 4.0,
        chan_px: 1.5,
        slice_frac: 0.4,
        mix: 1.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "block_glitch",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::block_glitch_matted(
                    px,
                    w,
                    h,
                    op.intensity,
                    op.seed,
                    op.tick,
                    op.block_size_px,
                    op.jitter_frac,
                    op.amount_px,
                    op.chan_px,
                    op.slice_frac,
                    op.mix,
                    m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::block_glitch(
                    px,
                    w,
                    h,
                    op.intensity,
                    op.seed,
                    op.tick,
                    op.block_size_px,
                    op.jitter_frac,
                    op.amount_px,
                    op.chan_px,
                    op.slice_frac,
                    op.mix,
                );
            },
            gpu: &|t, m| fx.block_glitch(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_widens_the_scanline_period() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let op = ScanlinesOp {
        intensity: 0.8,
        period_px: 4.0,
        roll_px: 2.5,
        interlace: true,
        mix: 1.0,
    };
    let run = |px: &mut [f32], m: &[f32]| {
        lumit_core::fx::cpu::scanlines_matted(
            px,
            w,
            h,
            op.intensity,
            op.period_px,
            op.roll_px,
            op.interlace,
            op.mix,
            m,
        );
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "scanlines",
            w,
            h,
            img: &img,
            cpu: &run,
            plain: &|px| {
                lumit_core::fx::cpu::scanlines(
                    px,
                    w,
                    h,
                    op.intensity,
                    op.period_px,
                    op.roll_px,
                    op.interlace,
                    op.mix,
                );
            },
            gpu: &|t, m| fx.scanlines(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
    let n = (w * h) as usize;
    // A half matte is the lines at twice the period — not the same lines half
    // as dark, which is what scaling Intensity (the dissolve) would give.
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5f32, 0.5, 0.5, 1.0]).collect();
    let mut half = img.clone();
    run(&mut half, &flat);
    let mut doubled = img.clone();
    lumit_core::fx::cpu::scanlines(
        &mut doubled,
        w,
        h,
        op.intensity,
        op.period_px * 2.0,
        op.roll_px,
        op.interlace,
        op.mix,
    );
    assert_eq!(
        half, doubled,
        "a half matte must be the lines at twice the period"
    );
    // A black matte is no visible lines at all: the picture comes back whole.
    let black: Vec<f32> = (0..n).flat_map(|_| [0.0f32, 0.0, 0.0, 1.0]).collect();
    let mut none = img.clone();
    run(&mut none, &black);
    assert_eq!(none, img, "a black matte must leave no line on the picture");
}

#[test]
fn the_matte_scales_the_offset_shift() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    let (shift, mix) = ([7.0f32, -5.0f32], 1.0f32);
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "offset",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::offset_matted(px, w, h, shift, mix, m),
            plain: &|px| lumit_core::fx::cpu::offset(px, w, h, shift, mix),
            gpu: &|t, m| fx.offset(&ctx, t, w, h, m, shift, mix),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_lens_distortion() {
    use lumit_core::fx::effects::lens_distort::LensDistort;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut l = LensDistort::read(Params::EMPTY);
    l.centre_x = 16.0;
    l.centre_y = 12.0;
    l.fov = 120.0;
    l.edge = 1;
    let p = l.packed();
    let op = LensDistortOp {
        active: p.active,
        tan_half_fov: p.tan_half_fov,
        reverse: p.reverse,
        half_kind: p.half_kind,
        centre: p.centre,
        edge: p.edge,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "lens_distort",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::lens_distort_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::lens_distort(px, w, h, &p),
            gpu: &|t, m| fx.lens_distort(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_corner_pin_pull() {
    use lumit_core::fx::effects::corner_pin::CornerPin;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut c = CornerPin::read(Params::EMPTY);
    c.upper_left_x = 8.0;
    c.upper_left_y = 2.0;
    c.upper_right_x = 24.0;
    c.upper_right_y = 2.0;
    c.lower_left_x = 0.0;
    c.lower_left_y = 22.0;
    c.lower_right_x = 32.0;
    c.lower_right_y = 22.0;
    c.edge = 1;
    let p = c.packed();
    let op = CornerPinOp {
        inv: p.inv,
        active: p.active,
        edge: p.edge,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "corner_pin",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::corner_pin_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::corner_pin(px, w, h, &p),
            gpu: &|t, m| fx.corner_pin(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
    // The owner's words: where the matte is black the pixel stays where it
    // was — a black matte is the untouched picture, not a transparent one.
    let n = (w * h) as usize;
    let black: Vec<f32> = (0..n).flat_map(|_| [0.0f32, 0.0, 0.0, 1.0]).collect();
    let mut still = img.clone();
    lumit_core::fx::cpu::corner_pin_matted(&mut still, w, h, &p, &black);
    assert!(
        worst_diff(&still, &img) < 1e-5,
        "a black matte must leave every pixel where it was"
    );
}

#[test]
fn the_matte_scales_the_bezier_warp_bend() {
    use lumit_core::fx::effects::bezier_warp::BezierWarp;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let (fw, fh) = (w as f32, h as f32);
    let mut b = BezierWarp::read(Params::EMPTY);
    b.upper_left_x = 0.0;
    b.upper_left_y = 0.0;
    b.upper_right_x = fw;
    b.upper_right_y = 0.0;
    b.lower_right_x = fw;
    b.lower_right_y = fh;
    b.lower_left_x = 0.0;
    b.lower_left_y = fh;
    b.top_left_tangent_x = fw / 3.0;
    b.top_left_tangent_y = -6.0;
    b.top_right_tangent_x = fw * 2.0 / 3.0;
    b.top_right_tangent_y = -6.0;
    b.right_top_tangent_x = fw;
    b.right_top_tangent_y = fh / 3.0;
    b.right_bottom_tangent_x = fw;
    b.right_bottom_tangent_y = fh * 2.0 / 3.0;
    b.bottom_left_tangent_x = fw / 3.0;
    b.bottom_left_tangent_y = fh + 6.0;
    b.bottom_right_tangent_x = fw * 2.0 / 3.0;
    b.bottom_right_tangent_y = fh + 6.0;
    b.left_top_tangent_x = 0.0;
    b.left_top_tangent_y = fh / 3.0;
    b.left_bottom_tangent_x = 0.0;
    b.left_bottom_tangent_y = fh * 2.0 / 3.0;
    let p = b.packed();
    let op = BezierWarpOp {
        pts: p.pts,
        steps: p.steps,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "bezier_warp",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::bezier_warp_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::bezier_warp(px, w, h, &p),
            gpu: &|t, m| fx.bezier_warp(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_twirl_angle() {
    use lumit_core::fx::effects::twirl::Twirl;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut t = Twirl::read(Params::EMPTY);
    t.centre_x = 16.0;
    t.centre_y = 12.0;
    t.radius = 10.0;
    t.angle = 200.0;
    let p = t.packed();
    let op = TwirlOp {
        centre: p.centre,
        radius: p.radius,
        inv_radius: p.inv_radius,
        angle: p.angle,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "twirl",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::twirl_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::twirl(px, w, h, &p),
            gpu: &|tex, m| fx.twirl(&ctx, tex, w, h, m, &op),
            tol: 2e-2,
        },
    );
    // A half matte on 200° IS the twirl at 100°: the same picture, to the
    // byte, that the control at half draws — which no dissolve of the 200°
    // picture can be.
    let n = (w * h) as usize;
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5f32, 0.5, 0.5, 1.0]).collect();
    let mut matted = img.clone();
    lumit_core::fx::cpu::twirl_matted(&mut matted, w, h, &p, &flat);
    let mut half = t;
    half.angle = 100.0;
    let mut at_half = img.clone();
    lumit_core::fx::cpu::twirl(&mut at_half, w, h, &half.packed());
    assert!(
        worst_diff(&matted, &at_half) < 1e-5,
        "a half matte on 200° must be the 100° twirl"
    );
}

#[test]
fn the_matte_scales_the_spherize_bulge() {
    use lumit_core::fx::effects::spherize::Spherize;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut s = Spherize::read(Params::EMPTY);
    s.centre_x = 16.0;
    s.centre_y = 12.0;
    s.radius = 11.0;
    s.bulge = 100.0;
    let p = s.packed();
    let op = SpherizeOp {
        centre: p.centre,
        radius: p.radius,
        inv_radius: p.inv_radius,
        bulge: p.bulge,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "spherize",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::spherize_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::spherize(px, w, h, &p),
            gpu: &|t, m| fx.spherize(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_ripple_height() {
    use lumit_core::fx::effects::ripple::Ripple;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut r = Ripple::read(Params::EMPTY);
    r.centre_x = 16.0;
    r.centre_y = 12.0;
    r.radius = 12.0;
    r.wave_height = 2.5;
    r.wave_width = 5.0;
    let p = r.packed();
    let op = RippleOp {
        centre: p.centre,
        radius: p.radius,
        inv_radius: p.inv_radius,
        amount: p.amount,
        inv_width: p.inv_width,
        turns: p.turns,
        asymmetric: p.asymmetric,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "ripple",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::ripple_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::ripple(px, w, h, &p),
            gpu: &|t, m| fx.ripple(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_wave_warp_height() {
    use lumit_core::fx::effects::wave_warp::WaveWarp;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut v = WaveWarp::read(Params::EMPTY);
    v.wave_height = 3.0;
    v.wave_width = 14.0;
    let p = v.packed();
    let op = WaveWarpOp {
        dir: p.dir,
        perp: p.perp,
        height: p.height,
        inv_width: p.inv_width,
        turns: p.turns,
        shape: p.shape,
        pin: p.pin,
        inv_pin_band: p.inv_pin_band,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "wave_warp",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::wave_warp_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::wave_warp(px, w, h, &p),
            gpu: &|t, m| fx.wave_warp(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_warp_bend() {
    use lumit_core::fx::effects::warp::Warp;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    // Bulge with both perspective tapers, so all three scaled controls matter.
    let mut a = Warp::read(Params::EMPTY);
    a.style = 4;
    a.bend = 60.0;
    a.horizontal_distortion = 40.0;
    a.vertical_distortion = -30.0;
    let p = a.packed();
    let op = WarpOp {
        style: p.style,
        bend: p.bend,
        h_distort: p.h_distort,
        v_distort: p.v_distort,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "warp",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::warp_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::warp(px, w, h, &p),
            gpu: &|t, m| fx.warp(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

// ---------------------------------------------------------------------------
// The matte scales the amount (K-428, docs/08 §2.6): the Generate and Stylise
// claims. Every one runs through `check_matte_claim`, so each is held to the
// same four facts — parity under a ramp, bit-stability, an empty matte that IS
// the pre-claim function, and a half matte that is NOT the generic dissolve.
//
// The drawn effects are set to their "on transparent" composite here, which is
// where the claim and the dissolve part company: at a black matte the drawing
// is simply not drawn, and with the layer that arrived already discarded there
// is nothing for a dissolve to fade back to.
// ---------------------------------------------------------------------------

/// A flat matte of one grey, at this test's raster — the picture a dissolve is
/// held against when the claim's own answer has to be named exactly.
fn flat_matte(w: u32, h: u32, grey: f32) -> Vec<f32> {
    (0..(w * h) as usize)
        .flat_map(|_| [grey, grey, grey, 1.0])
        .collect()
}

/// The corpus as the harness sees it: fp16-quantised, so a CPU answer compared
/// against another CPU answer starts from the same bits the GPU was given.
fn quantised(img: &[f32]) -> Vec<f32> {
    img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
}

#[test]
fn the_matte_scales_the_add_grain_intensity() {
    use lumit_core::fx::effects::add_grain::AddGrain;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let mut g = AddGrain::read(Params::EMPTY);
    g.seed = 20_260_823;
    g.intensity = 200.0;
    let p = g.packed(37);
    let op = AddGrainOp {
        amplitude: p.amplitude,
        inv_size: p.inv_size,
        softness: p.softness,
        tonal: p.tonal,
        monochrome: p.monochrome,
        seed: p.seed,
        tick: p.tick,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "add_grain",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::add_grain_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::add_grain(px, w, h, &p),
            gpu: &|t, m| fx.add_grain(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_lightning_opacity() {
    use lumit_core::fx::effects::lightning::Lightning;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let mut l = Lightning::read(Params::EMPTY);
    l.origin_x = 5.0;
    l.origin_y = 27.0;
    l.direction_x = 43.0;
    l.direction_y = 5.0;
    l.core_radius = 1.0;
    l.glow_radius = 5.0;
    l.seed = 20_260_823;
    l.composite_on_original = false;
    let p = l.packed();
    let op = LightningOp {
        segments: p.segments,
        fades: p.fades,
        count: p.count,
        core_radius: p.core_radius,
        glow_radius: p.glow_radius,
        glow_opacity: p.glow_opacity,
        core_colour: p.core_colour,
        glow_colour: p.glow_colour,
        composite: p.composite,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "lightning",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::lightning_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::lightning(px, w, h, &p),
            gpu: &|t, m| fx.lightning(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_radio_waves_opacity() {
    use lumit_core::fx::effects::radio_waves::RadioWaves;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 48u32);
    let img = smooth_corpus(w, h);
    let mut r = RadioWaves::read(Params::EMPTY);
    r.centre_x = 24.0;
    r.centre_y = 24.0;
    r.expansion = 8.0;
    r.stroke_width = 1.5;
    r.time = 2.5;
    r.composite_on_original = false;
    let p = r.packed();
    let op = RadioWavesOp {
        centre: p.centre,
        vertex: p.vertex,
        normal: p.normal,
        period: p.period,
        rotation: p.rotation,
        spin: p.spin,
        newest: p.newest,
        count: p.count,
        time: p.time,
        period_s: p.period_s,
        expansion: p.expansion,
        lifespan: p.lifespan,
        half_width: p.half_width,
        fade_in: p.fade_in,
        fade_out: p.fade_out,
        colour: p.colour,
        opacity: p.opacity,
        composite: p.composite,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "radio_waves",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::radio_waves_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::radio_waves(px, w, h, &p),
            gpu: &|t, m| fx.radio_waves(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_vegas_opacity() {
    use lumit_core::fx::effects::vegas::Vegas;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let mut v = Vegas::read(Params::EMPTY);
    v.width = 3.0;
    v.segment_length = 12.0;
    v.threshold = 55.0;
    v.composite_on_original = false;
    let p = v.packed();
    let op = VegasOp {
        from_alpha: p.from_alpha,
        level: p.level,
        half_width: p.half_width,
        band: p.band,
        inv_segment: p.inv_segment,
        duty: p.duty,
        phase: p.phase,
        colour: p.colour,
        opacity: p.opacity,
        composite: p.composite,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "vegas",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::vegas_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::vegas(px, w, h, &p),
            gpu: &|t, m| fx.vegas(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );

    // **A black matte leaves the pixel empty, and no dissolve can.** With
    // Composite on original off the layer that arrived is already gone, so
    // "draw nothing here" is transparency — where a strength dissolve would
    // hand the whole picture back.
    let mut dark = quantised(&img);
    lumit_core::fx::cpu::vegas_matted(&mut dark, w, h, &p, &flat_matte(w, h, 0.0));
    assert!(
        dark.iter().all(|v| *v == 0.0),
        "a black matte on Vegas painting on transparent must leave nothing at all"
    );
}

#[test]
fn the_matte_scales_the_path_drawing_opacity() {
    use lumit_core::fx::cpu;
    use lumit_core::fx::effects::{scribble::Scribble, stroke::Stroke};
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (48u32, 32u32);
    let img = smooth_corpus(w, h);
    let ellipse = oracle_ellipse(w, h);
    let squiggle = oracle_squiggle(w, h);

    let op_of = |p: &cpu::PathDrawParams| PathDrawOp {
        segments: p.segments,
        arcs: p.arcs,
        count: p.count,
        half_width: p.half_width,
        band: p.band,
        inv_segment: p.inv_segment,
        duty: p.duty,
        phase: p.phase,
        wiggle_amp: p.wiggle_amp,
        wiggle_freq: p.wiggle_freq,
        wiggle_tick: p.wiggle_tick,
        seed: p.seed,
        colour: p.colour,
        opacity: p.opacity,
        style: p.style,
        mix: p.mix,
    };

    // Scribble's hatch, laid on transparent.
    let mut s = Scribble::read(Params::EMPTY);
    s.spacing = 5.0;
    s.stroke_width = 1.5;
    s.composite_on_original = false;
    let sp = s.packed(&ellipse, 1.0, 3.5);
    // Stroke's brush, revealing the original — the drawing as a hole.
    let mut b = Stroke::read(Params::EMPTY);
    b.brush_size = 5.0;
    b.paint_style = cpu::PAINT_REVEAL_ORIGINAL;
    let bp = b.packed(&squiggle, 1.0);

    for (name, p) in [("scribble", sp), ("stroke", bp)] {
        let op = op_of(&p);
        check_matte_claim(
            &ctx,
            &MatteClaim {
                name,
                w,
                h,
                img: &img,
                cpu: &|px, m| cpu::path_draw_matted(px, w, h, &p, m),
                plain: &|px| cpu::path_draw(px, w, h, &p),
                gpu: &|t, m| fx.path_draw(&ctx, t, w, h, m, &op),
                tol: 2e-2,
            },
        );
    }
}

#[test]
fn the_matte_scales_the_drop_shadow_opacity() {
    use lumit_core::fx::effects::drop_shadow::DropShadow;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = alpha_corpus(w, h);
    // Shadow only: the mode in which scaling the shadow's Opacity is not the
    // dissolve, because there is no layer left underneath to fade back to.
    let mut d = DropShadow::read(Params::EMPTY);
    d.distance = 6.0;
    d.softness = 3.0;
    d.shadow_only = true;
    let p = d.packed();
    let op = DropShadowOp {
        colour: p.colour,
        opacity: p.opacity,
        offset: p.offset,
        softness_px: p.softness_px,
        shadow_only: p.shadow_only,
        mix: p.mix,
        spread_scale: p.spread_scale,
        knockout: p.knockout,
        invert: p.invert,
        inner: p.inner,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "drop_shadow",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::drop_shadow_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::drop_shadow(px, w, h, &p),
            gpu: &|t, m| fx.drop_shadow(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_roughen_edges_border() {
    use lumit_core::fx::effects::roughen_edges::RoughenEdges;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = disc_corpus(w, h);
    let mut r = RoughenEdges::read(Params::EMPTY);
    r.border = 4.0;
    r.scale = 8.0;
    r.offset_x = 16.0;
    r.offset_y = 12.0;
    r.seed = 7;
    let p = r.packed();
    let op = RoughenEdgesOp {
        seed: p.field.seed,
        octaves: p.field.octaves,
        gain: p.field.gain,
        lacunarity: p.field.lacunarity,
        cycle: p.field.cycle,
        flags: u32::from(p.field.perlin) | (u32::from(p.field.turbulent) << 1),
        offset: p.offset,
        inv_scale: p.inv_scale,
        z: p.z,
        border_px: p.border_px,
        influence: p.influence,
        half_width: p.half_width,
        colour: p.colour,
        colour_on: p.colour_on,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "roughen_edges",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::roughen_edges_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::roughen_edges(px, w, h, &p),
            gpu: &|t, m| fx.roughen_edges(&ctx, t, w, h, m, &op),
            tol: 3e-2,
        },
    );
}

#[test]
fn the_matte_scales_the_median_radius() {
    use lumit_core::fx::effects::median::Median;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = speckled_corpus(w, h);
    let of = |radius: f32| {
        let mut m = Median::read(Params::EMPTY);
        m.radius = radius;
        m
    };
    let p = of(2.0).packed();
    let n = (2 * p.radius + 1) * (2 * p.radius + 1);
    let op = MedianOp {
        radius: p.radius,
        keep: (n + 1) / 2,
        alpha_on: f32::from(u8::from(p.alpha)),
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "median",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::median_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::median(px, w, h, &p),
            gpu: &|t, m| fx.median(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );

    // **A half matte on Radius 2 IS the Radius 1 median** — the picture a
    // Radius 1 window makes, not a half fade of the Radius 2 one. That is the
    // whole claim in one equality, and no dissolve can produce it.
    let mut half = quantised(&img);
    lumit_core::fx::cpu::median_matted(&mut half, w, h, &p, &flat_matte(w, h, 0.5));
    let mut narrow = quantised(&img);
    lumit_core::fx::cpu::median(&mut narrow, w, h, &of(1.0).packed());
    assert_eq!(
        half, narrow,
        "a half matte on Radius 2 must BE the Radius 1 median, to the bit"
    );
}

#[test]
fn the_matte_scales_the_emboss_relief() {
    use lumit_core::fx::effects::emboss::Emboss;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let of = |relief: f32| {
        let mut e = Emboss::read(Params::EMPTY);
        e.relief = relief;
        e
    };
    let p = of(4.0).packed();
    let op = EmbossOp {
        offset: p.offset,
        contrast: p.contrast,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "emboss",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::emboss_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::emboss(px, w, h, &p),
            gpu: &|t, m| fx.emboss(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );

    // **A black matte is the flat sheet, not the picture.** Relief 0 is mid-grey
    // with no light on it (§3.67), so the honest answer at a black matte is that
    // sheet — where a dissolve would give the untouched picture back.
    let mut dark = quantised(&img);
    lumit_core::fx::cpu::emboss_matted(&mut dark, w, h, &p, &flat_matte(w, h, 0.0));
    let mut flat = quantised(&img);
    lumit_core::fx::cpu::emboss(&mut flat, w, h, &of(0.0).packed());
    assert_eq!(
        dark, flat,
        "a black matte on Emboss must BE the Relief 0 sheet, to the bit"
    );
}

#[test]
fn the_matte_scales_the_texturize_relief() {
    use lumit_core::fx::effects::texturize::Texturize;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    // The same coarse weave the oracle test embosses.
    let mut weave = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let u = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            let g = 0.5 + 0.4 * ((u * 18.0).sin() * (v * 14.0).cos());
            weave[i] = g;
            weave[i + 1] = g;
            weave[i + 2] = g;
            weave[i + 3] = 1.0;
        }
    }
    let qweave = quantised(&weave);
    let weave_tex = upload_linear_f32(&ctx, &weave, w, h);

    let mut t = Texturize::read(Params::EMPTY);
    t.scale = 100.0;
    t.relief = 3.0;
    let p = t.packed();
    let op = TexturizeOp {
        offset: p.offset,
        contrast: p.contrast,
        inv_scale: p.inv_scale,
        placement: p.placement,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "texturize",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::texturize_matted(px, &qweave, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::texturize(px, &qweave, w, h, &p),
            gpu: &|tx, m| fx.texturize(&ctx, tx, w, h, Some(&weave_tex), m, &op),
            tol: 2e-2,
        },
    );
}

// ---------------------------------------------------------------------------
// The matte scales the amount (K-429, docs/08 §2.6): the temporal and
// transition claims — Echo's Decay, both motion blurs' Shutter angle, and
// every wipe's Completion.
// ---------------------------------------------------------------------------

#[test]
fn the_matte_scales_the_linear_wipe_completion() {
    use lumit_core::fx::effects::linear_wipe::LinearWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let l = {
        let mut l = LinearWipe::read(Params::EMPTY);
        l.centre_x = w as f32 * 0.5;
        l.centre_y = h as f32 * 0.5;
        l.feather = 6.0;
        l
    };
    let p = l.packed();
    let op = LinearWipeOp {
        centre: p.centre,
        normal: p.normal,
        completion: p.completion,
        band: p.band,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "linear_wipe",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::linear_wipe_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::linear_wipe(px, w, h, &p),
            gpu: &|t, m| fx.linear_wipe(&ctx, t, w, h, m, &op),
            tol: 2e-3,
        },
    );
}

#[test]
fn the_matte_scales_the_radial_wipe_completion() {
    use lumit_core::fx::effects::radial_wipe::RadialWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let r = {
        let mut r = RadialWipe::read(Params::EMPTY);
        r.centre_x = w as f32 * 0.5;
        r.centre_y = h as f32 * 0.5;
        r.feather = 4.0;
        r
    };
    let p = r.packed();
    let op = RadialWipeOp {
        centre: p.centre,
        start: p.start,
        dir: p.dir,
        completion: p.completion,
        feather: p.feather,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "radial_wipe",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::radial_wipe_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::radial_wipe(px, w, h, &p),
            gpu: &|t, m| fx.radial_wipe(&ctx, t, w, h, m, &op),
            tol: 2e-3,
        },
    );
}

#[test]
fn the_matte_scales_the_venetian_blinds_completion() {
    use lumit_core::fx::effects::venetian_blinds::VenetianBlinds;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let v = {
        let mut v = VenetianBlinds::read(Params::EMPTY);
        v.width = 9.0;
        v.feather = 3.0;
        v
    };
    let p = v.packed();
    let op = VenetianBlindsOp {
        normal: p.normal,
        period: p.period,
        completion: p.completion,
        band: p.band,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "venetian_blinds",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::venetian_blinds_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::venetian_blinds(px, w, h, &p),
            gpu: &|t, m| fx.venetian_blinds(&ctx, t, w, h, m, &op),
            // A shade looser than the unmatted parity check: a ramp matte puts
            // a slat's soft edge under every column, so far more pixels sit on
            // the feather where fp16 rounds hardest.
            tol: 6e-3,
        },
    );
}

/// The Iris wipe has no Completion — **the radius is the transition** (§3.71) —
/// so its matte scales that instead, which is the same sentence about the same
/// thing (K-429). Beyond the four facts every claim is held to, one equality no
/// dissolve can reach: a flat half matte on a radius-8 iris draws *exactly* the
/// radius-4 picture, a genuinely smaller hole.
#[test]
fn the_matte_scales_the_iris_wipe_radius() {
    use lumit_core::fx::effects::iris_wipe::IrisWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let base = {
        let mut i = IrisWipe::read(Params::EMPTY);
        i.centre_x = w as f32 * 0.5;
        i.centre_y = h as f32 * 0.5;
        i.outer_radius = 8.0;
        i.inner_radius = 4.0;
        i
    };
    let op_of = |i: IrisWipe| {
        let p = i.packed();
        IrisWipeOp {
            centre: p.centre,
            vertex: p.vertex,
            normal: p.normal,
            period: p.period,
            rotation: p.rotation,
            band: p.band,
            active: p.active,
            mix: p.mix,
        }
    };
    let p = base.packed();
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "iris_wipe",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::iris_wipe_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::iris_wipe(px, w, h, &p),
            gpu: &|t, m| fx.iris_wipe(&ctx, t, w, h, m, &op_of(base)),
            tol: 2e-3,
        },
    );

    // **A half matte IS the half-radius iris**, to the bit. A dissolve cannot
    // move an edge; this does.
    let q = |v: &[f32]| -> Vec<f32> { v.iter().map(|x| f16_to_f32(f16_bits(*x))).collect() };
    let img = q(&img);
    let n = (w * h) as usize;
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5, 0.5, 0.5, 1.0]).collect();
    let mut halved = img.clone();
    lumit_core::fx::cpu::iris_wipe_matted(&mut halved, w, h, &p, &flat);
    let mut smaller = base;
    smaller.outer_radius = 4.0;
    smaller.inner_radius = 2.0;
    let mut want = img.clone();
    lumit_core::fx::cpu::iris_wipe(&mut want, w, h, &smaller.packed());
    assert_eq!(
        halved, want,
        "a half matte on a radius-8 iris must BE the radius-4 iris"
    );

    // And a black matte leaves the frame exactly alone — the same exact
    // identity Outer radius 0 already is.
    let black: Vec<f32> = (0..n).flat_map(|_| [0.0, 0.0, 0.0, 1.0]).collect();
    let mut shut = img.clone();
    lumit_core::fx::cpu::iris_wipe_matted(&mut shut, w, h, &p, &black);
    assert_eq!(shut, img, "a black matte must be the bit-exact identity");
}

#[test]
fn the_matte_scales_the_card_wipe_completion() {
    use lumit_core::fx::effects::card_wipe::CardWipe;
    use lumit_core::fx::{EffectMetadata, Params};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = smooth_corpus(w, h);
    let c = {
        let mut c = CardWipe::read(Params::EMPTY);
        c.seed = 12_345;
        // Half of *this* raster, for the reason the oracle above gives.
        c.transition_width = w as f32 * 0.5;
        c
    };
    let p = c.packed(w as f32, h as f32);
    let op = CardWipeOp {
        grid: p.grid,
        completion: p.completion,
        inv_width: p.inv_width,
        one_minus_width: p.one_minus_width,
        order_axis: p.order_axis,
        order_bias: p.order_bias,
        order_scale: p.order_scale,
        axis: p.axis,
        direction: p.direction,
        randomness: p.randomness,
        seed: p.seed,
        mix: p.mix,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "card_wipe",
            w,
            h,
            img: &img,
            cpu: &|px, m| lumit_core::fx::cpu::card_wipe_matted(px, w, h, &p, m),
            plain: &|px| lumit_core::fx::cpu::card_wipe(px, w, h, &p),
            gpu: &|t, m| fx.card_wipe(&ctx, t, w, h, m, &op),
            tol: 2e-2,
        },
    );
}

/// Echo's Decay, per pixel (K-429). Beyond the four facts, one equality a
/// dissolve cannot reach: a flat half matte on decay 0.6 draws *exactly* the
/// decay-0.3 trail — genuinely shorter ghosts, not a long trail faded back.
#[test]
fn the_matte_scales_the_echo_decay() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (24u32, 16u32);
    let q = |v: &[f32]| -> Vec<f32> { v.iter().map(|x| f16_to_f32(f16_bits(*x))).collect() };
    let current = q(&corpus(w, h));
    // Two neighbours that are the current frame shifted along, so a trail is
    // visibly a trail rather than a brightening.
    let shifted = |by: usize| -> Vec<f32> {
        let n = (w * h) as usize;
        let mut out = vec![0.0f32; n * 4];
        for i in 0..n {
            let src = (i + by * 3) % n;
            out[i * 4..i * 4 + 4].copy_from_slice(&current[src * 4..src * 4 + 4]);
        }
        out
    };
    let n1 = shifted(1);
    let n2 = shifted(2);
    let n1_t = upload_linear_f32(&ctx, &n1, w, h);
    let n2_t = upload_linear_f32(&ctx, &n2, w, h);
    let gpu_neighbours: [(i32, &wgpu::Texture); 2] = [(-1, &n1_t), (-2, &n2_t)];
    let cpu_neighbours: [(i32, &[f32]); 2] = [(-1, &n1), (-2, &n2)];

    let weights_for = |decay: f32| {
        let mut ws = [0.0f32; 16];
        ws[0] = decay;
        ws[1] = decay * decay;
        ws
    };
    // Screen, the schema's default combine, at full Mix.
    let (mode, mix) = (3u32, 1.0f32);
    let weights = weights_for(0.6);
    let op = EchoOp { weights, mode, mix };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "echo",
            w,
            h,
            img: &current,
            cpu: &|px, m| {
                let out =
                    lumit_core::fx::cpu::echo_matted(px, &cpu_neighbours, weights, mode, mix, m);
                px.copy_from_slice(&out);
            },
            plain: &|px| {
                let out = lumit_core::fx::cpu::echo(px, &cpu_neighbours, weights, mode, mix);
                px.copy_from_slice(&out);
            },
            gpu: &|t, m| fx.echo(&ctx, t, &gpu_neighbours, w, h, m, &op),
            tol: 2e-2,
        },
    );

    // **A half matte IS the half-decay trail.** `(decay·k)^(i+1)` factorises as
    // `decay^(i+1) · k^(i+1)`, which is why this is exact rather than close.
    let n = (w * h) as usize;
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5, 0.5, 0.5, 1.0]).collect();
    let halved =
        lumit_core::fx::cpu::echo_matted(&current, &cpu_neighbours, weights, mode, mix, &flat);
    let want = lumit_core::fx::cpu::echo(&current, &cpu_neighbours, weights_for(0.3), mode, mix);
    assert_eq!(
        halved, want,
        "a half matte on decay 0.6 must BE the decay-0.3 trail"
    );

    // And a black matte skips every tap, leaving the current frame exactly —
    // which a zero-weight tap folded in would NOT do under Multiply.
    let black: Vec<f32> = (0..n).flat_map(|_| [0.0, 0.0, 0.0, 1.0]).collect();
    let none = lumit_core::fx::cpu::echo_matted(&current, &cpu_neighbours, weights, 4, mix, &black);
    assert_eq!(
        none, current,
        "a black matte must leave the frame alone, Multiply included"
    );
}

/// Fast motion blur's Shutter angle, per pixel (K-429).
#[test]
fn the_matte_scales_the_fast_motion_blur_shutter() {
    use lumit_core::fx::{MbQuality, MbView};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let img = corpus(w, h);
    let n = (w * h) as usize;
    let u = vec![3.0f32; n];
    let v = vec![1.5f32; n];
    let conf = vec![1.0f32; n];
    let flow_t = upload_flow_field(&ctx, &u, &v, &conf, w, h);
    let (shutter, samples, mix) = (1.0f32, 16i32, 1.0f32);
    let op = MotionBlurOp {
        shutter_frac: shutter,
        samples,
        mix,
        view: MbView::Rendered.code(),
        quality: MbQuality::Normal.code(),
        vector_scale: 0.0,
    };
    check_matte_claim(
        &ctx,
        &MatteClaim {
            name: "motion_blur",
            w,
            h,
            img: &img,
            cpu: &|px, m| {
                lumit_core::fx::cpu::motion_blur_matted(
                    px,
                    w,
                    h,
                    &u,
                    &v,
                    &conf,
                    shutter,
                    samples,
                    mix,
                    MbView::Rendered,
                    MbQuality::Normal,
                    m,
                );
            },
            plain: &|px| {
                lumit_core::fx::cpu::motion_blur(
                    px,
                    w,
                    h,
                    &u,
                    &v,
                    &conf,
                    shutter,
                    samples,
                    mix,
                    MbView::Rendered,
                    MbQuality::Normal,
                );
            },
            gpu: &|t, m| fx.motion_blur(&ctx, t, &flow_t, w, h, m, &op),
            tol: 3e-2,
        },
    );

    // **A half matte IS the half-shutter streak**, which is a genuinely shorter
    // smear rather than a long one faded back over a sharp frame.
    let q = |x: &[f32]| -> Vec<f32> { x.iter().map(|y| f16_to_f32(f16_bits(*y))).collect() };
    let img = q(&img);
    let flat: Vec<f32> = (0..n).flat_map(|_| [0.5, 0.5, 0.5, 1.0]).collect();
    let mut halved = img.clone();
    lumit_core::fx::cpu::motion_blur_matted(
        &mut halved,
        w,
        h,
        &u,
        &v,
        &conf,
        shutter,
        samples,
        mix,
        MbView::Rendered,
        MbQuality::Normal,
        &flat,
    );
    let mut want = img.clone();
    lumit_core::fx::cpu::motion_blur(
        &mut want,
        w,
        h,
        &u,
        &v,
        &conf,
        shutter * 0.5,
        samples,
        mix,
        MbView::Rendered,
        MbQuality::Normal,
    );
    assert_eq!(
        halved, want,
        "a half matte on a 360° shutter must BE the 180° streak"
    );
}

/// **A Motion vectors layer stands in for the measured flow** (K-429, 7.48).
/// The layer's red and green are the per-pixel motion, centred at ½ and scaled
/// by Vector scale; the GPU conversion is the op-for-op twin of
/// `cpu::motion_vectors_field`, and the blur that follows is the same blur.
#[test]
fn a_motion_vectors_layer_stands_in_for_the_measured_flow() {
    use lumit_core::fx::{MbQuality, MbView};

    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (32u32, 24u32);
    let q = |x: &[f32]| -> Vec<f32> { x.iter().map(|y| f16_to_f32(f16_bits(*y))).collect() };
    let img = q(&corpus(w, h));
    let src = upload_linear_f32(&ctx, &img, w, h);
    let n = (w * h) as usize;
    // A vector pass: red ramps left to right, green is a constant lift, both
    // about the standing-still mid-grey. Quantised first, so the CPU oracle
    // reads exactly what the texture holds.
    let vectors = q(&(0..n)
        .flat_map(|i| {
            let x = (i % w as usize) as f32 / (w - 1) as f32;
            [0.5 + 0.25 * x, 0.6, 0.5, 1.0]
        })
        .collect::<Vec<f32>>());
    let vec_t = upload_linear_f32(&ctx, &vectors, w, h);
    let scale = 24.0f32;
    let (u, v, conf) = lumit_core::fx::cpu::motion_vectors_field(&vectors, n, scale);

    let (samples, mix) = (16i32, 1.0f32);
    let op = MotionBlurOp {
        shutter_frac: 0.5,
        samples,
        mix,
        view: MbView::Rendered.code(),
        quality: MbQuality::Normal.code(),
        vector_scale: scale,
    };
    let field = fx.motion_vectors_field(&ctx, &vec_t, w, h, scale);
    let out = fx.motion_blur(&ctx, &src, &field, w, h, None, &op);
    let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

    let mut cpu = img.clone();
    lumit_core::fx::cpu::motion_blur(
        &mut cpu,
        w,
        h,
        &u,
        &v,
        &conf,
        0.5,
        samples,
        mix,
        MbView::Rendered,
        MbQuality::Normal,
    );
    let worst = worst_diff(&cpu, &gpu);
    eprintln!("motion vectors layer: worst {worst}");
    assert!(worst < 3e-2, "the supplied field drifted by {worst}");
    assert_ne!(gpu, img, "a supplied field must actually smear the picture");

    // Mid-grey everywhere is standing still: the field is zero and the blur is
    // the picture back.
    let still = q(&(0..n)
        .flat_map(|_| [0.5, 0.5, 0.5, 1.0])
        .collect::<Vec<f32>>());
    let still_t = upload_linear_f32(&ctx, &still, w, h);
    let field = fx.motion_vectors_field(&ctx, &still_t, w, h, scale);
    let out = fx.motion_blur(&ctx, &src, &field, w, h, None, &op);
    assert_eq!(
        readback_linear_f32(&ctx, &out, w, h).unwrap(),
        img,
        "a mid-grey vector pass is standing still, so the picture must come back"
    );
}

/// Accumulation motion blur's Shutter angle, per pixel (docs/08 §3.26, K-429).
///
/// This one has no kernel of its own — it orchestrates a re-render — so the
/// claim is checked on the combine that averages the sub-frame pictures. Three
/// facts: a white matte is the equal-weight average; a black matte is the
/// sample at the frame's own moment, so nothing is blurred; and a half matte is
/// neither, and is not the dissolve between them either.
#[test]
fn the_matte_scales_the_accumulation_shutter() {
    let Some(ctx) = crate::test_support::lease() else {
        crate::no_adapter();
        return;
    };
    let fx = ctx.fx();
    let (w, h) = (16u32, 8u32);
    let n = (w * h) as usize;
    let q = |x: &[f32]| -> Vec<f32> { x.iter().map(|y| f16_to_f32(f16_bits(*y))).collect() };
    // Five sub-frame renders, each a flat grey — the moving scene reduced to the
    // one thing the combine can see. Five and not four so the frame's own
    // moment lands inside a sample's span rather than on the seam between two,
    // and deliberately lopsided so a shorter exposure, the full average and the
    // dissolve between them are three visibly different numbers.
    let levels = [0.05f32, 0.1, 0.2, 0.8, 0.9];
    let frames: Vec<wgpu::Texture> = levels
        .iter()
        .map(|l| {
            let px = q(&(0..n).flat_map(|_| [*l, *l, *l, 1.0]).collect::<Vec<f32>>());
            upload_linear_f32(&ctx, &px, w, h)
        })
        .collect();
    let flat = |v: f32| {
        let px = q(&(0..n).flat_map(|_| [v, v, v, 1.0]).collect::<Vec<f32>>());
        upload_linear_f32(&ctx, &px, w, h)
    };
    // The default − 90° phase on a 180° shutter puts the frame's own time in
    // the middle of the open span.
    let anchor = 0.5f32;
    let read = |m: &wgpu::Texture| {
        let out = fx.accumulate_with_shutter(&ctx, &frames, m, w, h, anchor);
        readback_linear_f32(&ctx, &out, w, h).unwrap()[0]
    };

    // Fully open: the equal-weight average, which is what the effect has always
    // drawn.
    let mean = levels.iter().sum::<f32>() / levels.len() as f32;
    let white = read(&flat(1.0));
    assert!(
        (white - mean).abs() < 2e-3,
        "a white matte must be the equal-weight average: {white} vs {mean}"
    );

    // Shut: the shutter has closed to the frame's own instant, so the picture
    // is the sub-frame render at that instant and nothing is blurred at all.
    let black = read(&flat(0.0));
    assert!(
        (black - levels[2]).abs() < 2e-3,
        "a black matte must be the one sample at the frame's own moment: {black} vs {}",
        levels[2]
    );

    // Half open: the average over the middle half of the span — weights 0.3,
    // 0.4, 0.3 across samples 1, 2 and 3. **Not** the dissolve between the
    // blurred and the sharp picture, which is the one thing a strength matte on
    // this effect could ever have produced.
    let half = read(&flat(0.5));
    let want = 0.3 * levels[1] + 0.4 * levels[2] + 0.3 * levels[3];
    assert!(
        (half - want).abs() < 3e-3,
        "a half matte must average the middle half of the span: {half} vs {want}"
    );
    let dissolve = 0.5 * mean + 0.5 * black;
    assert!(
        (half - dissolve).abs() > 1e-2,
        "a half matte gave the dissolve between the blurred and the sharp frame          ({half} vs {dissolve}) — which is not a shorter exposure"
    );

    // Bit-stable, run to run (§2.4).
    assert_eq!(half, read(&flat(0.5)), "the combine must be bit-stable");
}
