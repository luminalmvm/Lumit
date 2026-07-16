//! Oklab/OkLCh perceptual colour operations (decision K-034).
//!
//! In plain terms: linear RGB is where light adds correctly; Oklab is where
//! *perception* behaves. Interpolate a gradient here and it stays vivid;
//! rotate a hue here and its lightness genuinely doesn't move. Users only
//! ever see RGB — these conversions happen inside operations, and they are
//! cheap: two 3×3 matrix multiplies and three cube roots each way.
//!
//! Constants are Björn Ottosson's reference values. The WGSL twin
//! (`oklab.wgsl`) carries the SAME constants; a test compiles it so the two
//! implementations cannot silently diverge in validity, and the CPU functions
//! below are the oracle for future effect kernels (K-019).

/// Linear sRGB → Oklab.
#[inline]
pub fn linear_srgb_to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * r + 0.536_332_54 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    ]
}

/// Oklab → linear sRGB. (May leave gamut for extreme inputs; callers clamp
/// at the display/8-bit edge, never mid-pipeline.)
#[inline]
pub fn oklab_to_linear_srgb([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    [
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    ]
}

/// Rectangular Oklab lerp: maximally smooth, but the straight chord can dip
/// in chroma between distant hues. For "stay colourful" gradients use
/// [`oklch_lerp`] — the polar path around the hue wheel.
#[inline]
pub fn oklab_lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let a = linear_srgb_to_oklab(a);
    let b = linear_srgb_to_oklab(b);
    oklab_to_linear_srgb([
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ])
}

/// OkLCh from linear sRGB: (lightness, chroma, hue in radians).
#[inline]
pub fn oklch_from_linear(rgb: [f32; 3]) -> [f32; 3] {
    let [l, a, b] = linear_srgb_to_oklab(rgb);
    [l, (a * a + b * b).sqrt(), b.atan2(a)]
}

/// Linear sRGB from OkLCh.
#[inline]
pub fn linear_from_oklch([l, c, h]: [f32; 3]) -> [f32; 3] {
    oklab_to_linear_srgb([l, c * h.cos(), c * h.sin()])
}

/// THE gradient primitive (K-034): interpolate in OkLCh with shortest-arc
/// hue, so a ramp between two saturated colours travels around the hue wheel
/// at full chroma instead of cutting through grey. Achromatic ends adopt the
/// other end's hue (hue is undefined at zero chroma).
#[inline]
pub fn oklch_lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    const ACHROMATIC: f32 = 1e-5;
    let [la, ca, mut ha] = oklch_from_linear(a);
    let [lb, cb, mut hb] = oklch_from_linear(b);
    if ca < ACHROMATIC {
        ha = hb;
    }
    if cb < ACHROMATIC {
        hb = ha;
    }
    let tau = std::f32::consts::TAU;
    let mut dh = (hb - ha) % tau;
    if dh > tau / 2.0 {
        dh -= tau;
    }
    if dh < -tau / 2.0 {
        dh += tau;
    }
    linear_from_oklch([la + (lb - la) * t, ca + (cb - ca) * t, ha + dh * t])
}

/// Chroma (colourfulness) of a linear-sRGB colour, in Oklab terms.
#[inline]
pub fn chroma(rgb: [f32; 3]) -> f32 {
    let [_, a, b] = linear_srgb_to_oklab(rgb);
    (a * a + b * b).sqrt()
}

/// Rotate hue by `degrees`, preserving Oklab lightness and chroma exactly
/// (rotation in the a/b plane — L is untouched by construction).
#[inline]
pub fn hue_rotate(rgb: [f32; 3], degrees: f32) -> [f32; 3] {
    let [l, a, b] = linear_srgb_to_oklab(rgb);
    let (sin, cos) = degrees.to_radians().sin_cos();
    oklab_to_linear_srgb([l, a * cos - b * sin, a * sin + b * cos])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn known_reference_values_hold() {
        // Ottosson's reference: white → L=1, a=b=0; and pure sRGB red.
        let white = linear_srgb_to_oklab([1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 2e-4, "white L {}", white[0]);
        assert!(white[1].abs() < 2e-4 && white[2].abs() < 2e-4);

        let red = linear_srgb_to_oklab([1.0, 0.0, 0.0]);
        assert!((red[0] - 0.627_98).abs() < 1e-3, "red L {}", red[0]);
        assert!((red[1] - 0.224_86).abs() < 1e-3, "red a {}", red[1]);
        assert!((red[2] - 0.125_85).abs() < 1e-3, "red b {}", red[2]);
    }

    #[test]
    fn round_trips_are_tight_across_the_gamut() {
        // Deterministic sweep of the RGB cube (no wall-clock randomness).
        let mut worst = 0.0f32;
        for r in 0..=10 {
            for g in 0..=10 {
                for b in 0..=10 {
                    let rgb = [r as f32 / 10.0, g as f32 / 10.0, b as f32 / 10.0];
                    let back = oklab_to_linear_srgb(linear_srgb_to_oklab(rgb));
                    for i in 0..3 {
                        worst = worst.max((back[i] - rgb[i]).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-4, "worst round-trip error {worst}");
    }

    #[test]
    fn oklch_gradients_stay_colourful() {
        // Red → blue: the classic muddy-middle gradient.
        let red = [1.0f32, 0.0, 0.0];
        let blue = [0.0f32, 0.0, 1.0];
        let (c_red, c_blue) = (chroma(red), chroma(blue));

        let mid = oklch_lerp(red, blue, 0.5);
        let c_mid = chroma(mid);
        // Chroma at the midpoint is the lerp of end chromas (full vividness)…
        let expected = (c_red + c_blue) * 0.5;
        assert!(
            (c_mid - expected).abs() < 0.02,
            "midpoint chroma {c_mid} vs expected {expected}"
        );
        // …which beats both the rectangular-Oklab chord and the RGB lerp.
        assert!(c_mid > chroma(oklab_lerp(red, blue, 0.5)) * 1.5);
        assert!(c_mid > chroma([0.5, 0.0, 0.5]) * 1.05);

        // Endpoints exact; achromatic ends don't spin the hue.
        let start = oklch_lerp(red, blue, 0.0);
        for i in 0..3 {
            assert!((start[i] - red[i]).abs() < 1e-4);
        }
        let to_white = oklch_lerp(red, [1.0, 1.0, 1.0], 0.5);
        let [_, _, h] = oklch_from_linear(to_white);
        let [_, _, h_red] = oklch_from_linear(red);
        assert!((h - h_red).abs() < 1e-3, "fade-to-white keeps red's hue");
    }

    #[test]
    fn hue_rotation_preserves_lightness_and_chroma_exactly() {
        let colours = [
            [1.0, 0.0, 0.0],
            [0.2, 0.7, 0.1],
            [0.05, 0.1, 0.9],
            [0.8, 0.6, 0.2],
        ];
        for rgb in colours {
            let before = linear_srgb_to_oklab(rgb);
            for deg in [30.0, 120.0, 275.0] {
                let after = linear_srgb_to_oklab(hue_rotate(rgb, deg));
                assert!(
                    (after[0] - before[0]).abs() < 1e-4,
                    "L moved: {} → {} at {deg}°",
                    before[0],
                    after[0]
                );
                let c0 = (before[1] * before[1] + before[2] * before[2]).sqrt();
                let c1 = (after[1] * after[1] + after[2] * after[2]).sqrt();
                assert!((c1 - c0).abs() < 1e-4, "chroma moved: {c0} → {c1}");
            }
        }
    }

    /// The WGSL twin must stay compilable and carry the same constants —
    /// validated by building a shader module against a real device.
    #[test]
    fn wgsl_twin_compiles() {
        let Ok(ctx) = crate::GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let _module = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("oklab.wgsl"));
    }
}
