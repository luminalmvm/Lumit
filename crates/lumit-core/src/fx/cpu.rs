use super::{MbView, Resolved};

/// Apply one resolved effect to an RGBA f32 image (premultiplied,
/// linear light), in place.
pub fn apply(rgba: &mut [f32], w: u32, h: u32, fx: &Resolved) {
    match fx {
        Resolved::Blur {
            radius_px,
            edge,
            mix,
        } => blur_gaussian(rgba, w, h, *radius_px, *edge, *mix),
        Resolved::DirBlur {
            length_px,
            angle_deg,
            edge,
            mix,
        } => blur_directional(rgba, w, h, *length_px, *angle_deg, *edge, *mix),
        Resolved::RadialBlur {
            centre_frac,
            amount_px,
            spin,
            edge,
            mix,
        } => blur_radial(rgba, w, h, *centre_frac, *amount_px, *spin, *edge, *mix),
        Resolved::Sharpen {
            amount,
            radius_px,
            threshold,
            luma_only,
            mix,
        } => sharpen(
            rgba, w, h, *amount, *radius_px, *threshold, *luma_only, *mix,
        ),
        Resolved::SharpenSimple { amount, mix } => sharpen_simple(rgba, w, h, *amount, *mix),
        Resolved::RgbSplit {
            amount_px,
            angle_deg,
            radial,
            scale,
            mix,
        } => rgb_split(rgba, w, h, *amount_px, *angle_deg, *radial, *scale, *mix),
        Resolved::SpectralSplit {
            amount_px,
            angle_deg,
            radial,
            samples,
            mix,
        } => spectral_split(rgba, w, h, *amount_px, *angle_deg, *radial, *samples, *mix),
        Resolved::ChromaticAberration {
            amount_px,
            tints,
            mix,
        } => chromatic_aberration(rgba, w, h, *amount_px, *tints, *mix),
        Resolved::Flash {
            strength,
            colour,
            mix,
        } => flash(rgba, *strength, *colour, *mix),
        Resolved::ColourBalance {
            lift,
            gamma,
            gain,
            mix,
        } => colour_balance(rgba, *lift, *gamma, *gain, *mix),
        Resolved::Saturation { saturation, mix } => saturate(rgba, *saturation, *mix),
        Resolved::Vibrancy { amount, mix } => vibrance(rgba, *amount, *mix),
        Resolved::MatteKey {
            key,
            tol,
            soft,
            spill,
            mix,
        } => matte_key(rgba, *key, *tol, *soft, *spill, *mix),
        Resolved::Vignette {
            amount,
            radius,
            softness,
            roundness,
            mix,
        } => vignette(rgba, w, h, *amount, *radius, *softness, *roundness, *mix),
        Resolved::Exposure { factor, mix } => exposure(rgba, *factor, *mix),
        Resolved::HueShift { m, mix } => hue_shift(rgba, *m, *mix),
        Resolved::Contrast { k, mix } => contrast(rgba, *k, *mix),
        Resolved::Gamma { gamma: g, mix } => gamma(rgba, *g, *mix),
        Resolved::Temperature {
            gain_r,
            gain_b,
            mix,
        } => temperature(rgba, *gain_r, *gain_b, *mix),
        Resolved::Invert { mix } => invert(rgba, *mix),
        Resolved::Tint { black, white, mix } => tint(rgba, *black, *white, *mix),
        Resolved::Transform {
            anchor,
            position,
            scale,
            rotation_deg,
            opacity,
            mix,
        } => transform(
            rgba,
            w,
            h,
            *anchor,
            *position,
            *scale,
            *rotation_deg,
            // The Transform effect has no Edges control: transparent border,
            // its long-standing behaviour.
            0,
            *opacity,
            *mix,
        ),
        Resolved::Glow {
            radius_px,
            threshold,
            knee,
            intensity,
            tint,
            mix,
        } => glow(
            rgba, w, h, *radius_px, *threshold, *knee, *intensity, *tint, *mix,
        ),
        // Shake is a transform-domain effect (docs/08 §3.4): the
        // resolved wobble maps to the Transform reference through the
        // same shared affine the GPU dispatch uses, so both paths
        // consume bit-identical numbers. A neutral shake (zero wobble)
        // maps to the identity affine — the bit-exact passthrough the
        // Transform reference pins. `edge` is Shake's own Edges control.
        Resolved::Shake {
            offset_px,
            rotation_deg,
            zoom,
            edge,
            mix,
        } => {
            let (anchor, position, scale, rot) =
                super::shake_affine(w, h, *offset_px, *rotation_deg, *zoom);
            transform(rgba, w, h, anchor, position, scale, rot, *edge, 1.0, *mix);
        }
        Resolved::BlockGlitch {
            intensity,
            seed,
            tick,
            block_size_px,
            jitter_frac,
            amount_px,
            chan_px,
            slice_frac,
            mix,
        } => block_glitch(
            rgba,
            w,
            h,
            *intensity,
            *seed,
            *tick,
            *block_size_px,
            *jitter_frac,
            *amount_px,
            *chan_px,
            *slice_frac,
            *mix,
        ),
        Resolved::Scanlines {
            intensity,
            period_px,
            darkness,
            roll_px,
            interlace,
            mix,
        } => scanlines(
            rgba, w, h, *intensity, *period_px, *darkness, *roll_px, *interlace, *mix,
        ),
        // Echo is temporal: it needs the layer's neighbour frames, which
        // this single-buffer in-place dispatcher does not carry. The real
        // path is [`echo`] (with neighbours) on the GPU; here it is a
        // pass-through (the CPU-fallback render can't echo).
        Resolved::Echo { .. } => {}
        // Motion blur needs the layer's flow field, which this
        // single-buffer dispatcher does not carry either. The real path is
        // [`motion_blur`] (with the flow field) on the GPU; here it is a
        // pass-through, exactly like Echo.
        Resolved::MotionBlur { .. } => {}
        // Datamosh needs the layer's -1 neighbour and its flow field,
        // which this single-buffer dispatcher does not carry either. The
        // real path is `FxEngine::datamosh` (with neighbour + flow) on
        // the GPU; here it is a pass-through, exactly like Echo and
        // Motion blur.
        Resolved::Datamosh { .. } => {}
        // A LUT is a GPU colour map: the parsed cube never reaches this
        // Resolved-based CPU dispatcher (the file path is threaded
        // separately), so the CPU-degradation rung renders it as identity.
        // The §1.6 oracle reference is `lut::Lut3d::sample`, exercised
        // directly in the lumit-gpu test, not through cpu::apply.
        Resolved::Lut { .. } => {}
        // Depth of field reads a depth texture (the referenced layer
        // rendered alone) that never reaches this single-buffer dispatcher,
        // so — like Echo, Motion blur and LUT — the CPU-degradation rung
        // renders it as identity. The §1.6 oracle reference is
        // `dof_reference` in the lumit-gpu test (the depth is a texture, not
        // a number), not through cpu::apply.
        Resolved::Dof { .. } => {}
    }
}

/// Glow (docs/08 §3.3, v1 core): bright-pass every premultiplied channel
/// through [`super::glow_bright`] — alpha included, so the halo carries
/// coverage and glow spreads over transparency like light — blur the
/// leftover light with the shared gaussian (Repeat edges, fixed: the
/// halo holds its strength along frame borders instead of dimming), then
/// recombine additively in linear: `out = input + intensity · tint ·
/// halo`, output alpha saturating at 1 (full coverage). Highlights are
/// never clipped (§2.1). Intensity 0 is the effect's neutral point and
/// short-circuits to the bit-exact identity (the WGSL twin matches).
#[allow(clippy::too_many_arguments)]
pub fn glow(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    radius_px: f32,
    threshold: f32,
    knee: f32,
    intensity: f32,
    tint: [f32; 4],
    mix: f32,
) {
    if intensity == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let mut halo = vec![0.0f32; rgba.len()];
    for (dst, src) in halo.iter_mut().zip(original.iter()) {
        *dst = super::glow_bright(*src, threshold, knee);
    }
    blur_gaussian(&mut halo, w, h, radius_px, 1, 1.0);
    for i in (0..rgba.len()).step_by(4) {
        let o = &original[i..i + 4];
        let hl = &halo[i..i + 4];
        for c in 0..3 {
            let glowed = o[c] + intensity * (hl[c] * tint[c]);
            rgba[i + c] = o[c] * (1.0 - mix) + glowed * mix;
        }
        let a = (o[3] + intensity * hl[3]).min(1.0);
        rgba[i + 3] = o[3] * (1.0 - mix) + a * mix;
    }
}

/// Transform (docs/08 §3.5, K-090): resample the input through the
/// inverse of `position + R·S·(p − anchor)` — one bilinear tap per
/// output pixel, the revealed border handled by `edge` (0 Transparent,
/// 1 Repeat, 2 Mirror — the same shared policy the blur family uses,
/// [`EdgesMode`](super::EdgesMode)), premultiplied throughout, with
/// opacity multiplied into all four channels. The Transform effect passes
/// `edge = 0`; Shake threads its own Edges control (FX-11/K-146).
/// Identity parameters reproduce the input bit-exactly: the inverse
/// affine is exactly `q = p`, a bilinear tap at a pixel centre is
/// exactly that pixel, and opacity/mix 1 multiply by exact 1.0 — the
/// WGSL twin follows the identical arithmetic. A degenerate scale
/// (|s| < 1e-6) renders fully transparent, never a division blow-up.
#[allow(clippy::too_many_arguments)]
pub fn transform(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    anchor: [f32; 2],
    position: [f32; 2],
    scale: [f32; 2],
    rotation_deg: f32,
    edge: u32,
    opacity: f32,
    mix: f32,
) {
    let original = rgba.to_vec();
    // A collapsed (zero-scale) image is invisible: opacity 0, and the
    // sample point no longer matters (super::transform_op's rule).
    let (m, o, opacity) = super::transform_op(anchor, position, scale, rotation_deg, opacity);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let qx = m[0] * px + m[1] * py + o[0];
            let qy = m[2] * px + m[3] * py + o[1];
            // `edge` picks how the revealed border is sampled (0 Transparent,
            // 1 Repeat, 2 Mirror): the Transform effect passes 0 (its
            // long-standing behaviour); Shake passes its Edges control.
            let s = bilinear_edge(&original, w, h, qx, qy, edge);
            for c in 0..4 {
                let v = s[c] * opacity;
                rgba[i + c] = original[i + c] * (1.0 - mix) + v * mix;
            }
        }
    }
}

/// Colour balance (docs/08 §3.10 as amended by K-090): per-channel
/// gain → lift → gamma in linear light on unpremultiplied colour (§2.2),
/// re-premultiplied on the way out. Fully neutral parameters
/// short-circuit the whole effect, so a Colour balance at defaults is
/// the bit-exact identity rather than a round trip through `powf` and
/// the unpremultiply divide. Negative light clamps at zero (that is
/// what a crushing lift means); highlights are never clipped (§2.1).
pub fn colour_balance(rgba: &mut [f32], lift: [f32; 3], gamma: [f32; 3], gain: [f32; 3], mix: f32) {
    if lift == [0.0; 3] && gamma == [1.0; 3] && gain == [1.0; 3] {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let mut v = [0.0f32; 3];
        for c in 0..3 {
            let mut x = (u[c] * gain[c] + lift[c]).max(0.0);
            if gamma[c] != 1.0 {
                x = x.powf(1.0 / gamma[c]);
            }
            v[c] = x;
        }
        for c in 0..3 {
            let graded = v[c] * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// Saturation (docs/08 §3.10 as amended by K-090): scale colourfulness
/// about Rec. 709 luma, in linear light on unpremultiplied colour
/// (§2.2), re-premultiplied on the way out. Saturation 1 short-circuits
/// the whole effect (bit-exact identity); 0 collapses to true greyscale.
/// Named `saturate` so the parameter can keep the plain name.
pub fn saturate(rgba: &mut [f32], saturation: f32, mix: f32) {
    if saturation == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let luma = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
        for c in 0..3 {
            let v = (luma + (u[c] - luma) * saturation).max(0.0);
            let s = v * a;
            px[c] = px[c] * (1.0 - mix) + s * mix;
        }
    }
}

/// Vibrancy (docs/08 §3.10, K-152): a saturation boost weighted by each
/// pixel's current colourfulness — the per-pixel factor is `1 + amount·(1 −
/// sat)`, so low-saturation pixels lift more and already-vivid ones little
/// (protecting skin tones, avoiding clipping), unlike Saturation's uniform
/// scale. In linear light on unpremultiplied colour (§2.2), re-premultiplied.
/// `sat` is the scale-invariant HSV saturation `(max − min)/max`, clamped to
/// 0..1. Amount 0 short-circuits the whole effect (bit-exact identity); the
/// colour then scales about Rec. 709 luma exactly as Saturation does, so the
/// two share their premultiply handling and the WGSL twin matches op-for-op.
pub fn vibrance(rgba: &mut [f32], amount: f32, mix: f32) {
    if amount == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let luma = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
        // HSV-style saturation in 0..1, scale-invariant so HDR values above 1
        // read the same "how colourful" as ones below.
        let mx = u[0].max(u[1]).max(u[2]);
        let mn = u[0].min(u[1]).min(u[2]);
        let sat = if mx > 0.0 {
            ((mx - mn) / mx).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // More boost where sat is low; none where already saturated.
        let factor = 1.0 + amount * (1.0 - sat);
        for c in 0..3 {
            let v = (luma + (u[c] - luma) * factor).max(0.0);
            let s = v * a;
            px[c] = px[c] * (1.0 - mix) + s * mix;
        }
    }
}

/// Matte key (docs/08 §3.21): a soft chroma key (greenscreen removal), on
/// straight (unpremultiplied) colour (§2.2) — unpremultiply → key + despill
/// → re-premultiply, exactly Saturation's premultiply handling. The metric
/// is Euclidean distance in the chroma plane: each colour's chroma is
/// `rgb − luma` (Rec. 709 luma, [`LUMA`]), a pure-chroma vector, so greens
/// of any brightness sit at the same point and key alike. A **smoothstep**
/// keep-factor is 0 (fully keyed, alpha ·= 0) at chroma distance ≤ `tol`,
/// 1 (fully kept) at ≥ `tol + soft`, and smooth between — no hard step, so
/// the effect is continuous everywhere and safe under the §1.6 fp16 ULP
/// oracle (the WGSL twin matches op-for-op). Spill suppression pulls a
/// `spill` fraction of the pixel's key-hue projection out of its chroma
/// (desaturating toward luma along the key hue), fading green fringes on
/// kept pixels. The key's chroma/hue direction are derived here once and
/// per-invocation in the kernel from the identical `key`, so both paths use
/// the same numbers; a grey key (no hue) makes spill a no-op. Mix 0 is the
/// bit-exact identity (the `× (1 − mix) + · × mix` blend collapses to the
/// input). `soft`'s transition width floors at a small epsilon so `soft` 0
/// reads as a steep edge rather than a division by zero.
pub fn matte_key(rgba: &mut [f32], key: [f32; 4], tol: f32, soft: f32, spill: f32, mix: f32) {
    // Key chroma (a pure-chroma vector: its own luma is zero) and unit hue
    // direction; a grey key has no hue, so its direction is zero and spill
    // does nothing.
    let kl = key[0] * LUMA[0] + key[1] * LUMA[1] + key[2] * LUMA[2];
    let kc = [key[0] - kl, key[1] - kl, key[2] - kl];
    let klen = (kc[0] * kc[0] + kc[1] * kc[1] + kc[2] * kc[2]).sqrt();
    let kdir = if klen > 1e-6 {
        [kc[0] / klen, kc[1] / klen, kc[2] / klen]
    } else {
        [0.0; 3]
    };
    let e1 = tol + soft.max(1e-6);
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let pl = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
        let pc = [u[0] - pl, u[1] - pl, u[2] - pl];
        // Distance from the key's chroma → smoothstep keep-factor.
        let dc = [pc[0] - kc[0], pc[1] - kc[1], pc[2] - kc[2]];
        let d = (dc[0] * dc[0] + dc[1] * dc[1] + dc[2] * dc[2]).sqrt();
        let t = ((d - tol) / (e1 - tol)).clamp(0.0, 1.0);
        let keep = t * t * (3.0 - 2.0 * t);
        // Spill: remove the key-hue projection from the kept colour.
        let proj = (pc[0] * kdir[0] + pc[1] * kdir[1] + pc[2] * kdir[2]).max(0.0) * spill;
        let despilled = [
            u[0] - proj * kdir[0],
            u[1] - proj * kdir[1],
            u[2] - proj * kdir[2],
        ];
        let out_a = a * keep;
        for c in 0..3 {
            let proc = despilled[c] * out_a;
            px[c] = px[c] * (1.0 - mix) + proc * mix;
        }
        px[3] = a * (1.0 - mix) + out_a * mix;
    }
}

/// Exposure (docs/08 §3.16): a scene-linear gain on RGB. Premultiplied
/// colour scales consistently under a scalar, so there is no unpremultiply
/// round trip and alpha is untouched. `factor` (= 2^stops) 1.0 is the
/// bit-exact neutral point (the WGSL twin matches its early return); Mix 0
/// is likewise the identity.
pub fn exposure(rgba: &mut [f32], factor: f32, mix: f32) {
    if factor == 1.0 {
        return;
    }
    for px in rgba.chunks_exact_mut(4) {
        for ch in &mut px[..3] {
            let scaled = *ch * factor;
            *ch = *ch * (1.0 - mix) + scaled * mix;
        }
    }
}

/// Hue shift (docs/08 §3.17): a row-major linear 3×3 colour matrix `m`
/// (from [`super::hue_matrix`]) applied to RGB, alpha untouched. Works on
/// premultiplied colour directly — a linear matrix scales through alpha —
/// so no unpremultiply round trip. The identity matrix is the bit-exact
/// neutral point (the WGSL twin matches); Mix 0 is likewise the identity.
pub fn hue_shift(rgba: &mut [f32], m: [f32; 9], mix: f32) {
    if m == [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
        return;
    }
    for px in rgba.chunks_exact_mut(4) {
        let (r, g, b) = (px[0], px[1], px[2]);
        let nr = m[0] * r + m[1] * g + m[2] * b;
        let ng = m[3] * r + m[4] * g + m[5] * b;
        let nb = m[6] * r + m[7] * g + m[8] * b;
        px[0] = r * (1.0 - mix) + nr * mix;
        px[1] = g * (1.0 - mix) + ng * mix;
        px[2] = b * (1.0 - mix) + nb * mix;
    }
}

/// The mid-grey pivot contrast expands or compresses about (docs/08 §3.18).
pub const CONTRAST_PIVOT: f32 = 0.5;

/// Contrast (docs/08 §3.18): the affine grade `(u − pivot) × k + pivot` per
/// RGB channel about the fixed mid-grey pivot (0.5), in linear light on
/// unpremultiplied colour (§2.2), re-premultiplied on the way out —
/// exactly Saturation's premultiply handling. The `− pivot` offset is why
/// this cannot run through premultiplied alpha: it is an affine grade, not
/// a pure scale, so it does not commute with the alpha multiply. `k` 1.0
/// (Contrast 100 %) short-circuits the whole effect (bit-exact identity;
/// the WGSL twin matches). Purely continuous — no round/clamp/quantize — so
/// it is safe under the §1.6 fp16 ULP oracle. Highlights are never clipped
/// (§2.1) and values may go negative between grade and re-premultiply; that
/// is the honest affine result, matched op-for-op by the kernel.
pub fn contrast(rgba: &mut [f32], k: f32, mix: f32) {
    if k == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        for c in 0..3 {
            let v = (u[c] - CONTRAST_PIVOT) * k + CONTRAST_PIVOT;
            let graded = v * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// Gamma (docs/08 §3.19): a per-channel power curve
/// `out = pow(max(u, 0), 1/gamma)` in the compositor's scene-linear working
/// space, on unpremultiplied colour (§2.2), re-premultiplied on the way out
/// — exactly Contrast's and Saturation's premultiply handling. pow is
/// non-linear, so it does not commute with the alpha multiply: the pixel is
/// unpremultiplied, curved, then re-premultiplied. The input is clamped to
/// ≥ 0 before the pow (scene-linear colour can dip slightly negative, and
/// pow of a negative base is undefined); the clamp is byte-identical in the
/// WGSL twin so the §1.6 oracle holds. `gamma` 1.0 short-circuits the whole
/// effect (bit-exact identity — a short-circuit, not a reliance on
/// `pow(x, 1)` being exactly `x`; the WGSL twin matches). Continuous for
/// input ≥ 0, so it is safe under the §1.6 fp16 ULP oracle. `gamma` is
/// clamped ≥ 0.01 at resolve so `1/gamma` stays finite; alpha is untouched.
pub fn gamma(rgba: &mut [f32], gamma: f32, mix: f32) {
    if gamma == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let inv = 1.0 / gamma;
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        for c in 0..3 {
            let curved = u[c].max(0.0).powf(inv);
            let graded = curved * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// Temperature (docs/08 §3.20): a warm/cool white-balance shift as a
/// per-channel gain in scene-linear light — red by `gain_r`, blue by
/// `gain_b`, green and alpha untouched. Like Exposure, a per-channel scalar
/// scales premultiplied colour consistently (straight × gain, then × the
/// unchanged alpha), so there is no unpremultiply round trip — unlike the
/// affine Contrast/Saturation grades, whose − pivot offset breaks that
/// commutation. The gains are computed host-side (in the resolve step) so
/// the CPU reference and the WGSL kernel multiply by the identical numbers
/// (§1.6). Gains `(1.0, 1.0)` (Temperature 0) short-circuit the whole
/// effect — the bit-exact neutral point (the WGSL twin matches); Mix 0 is
/// likewise the identity. Purely continuous (a linear per-channel scale),
/// so it is safe under the §1.6 fp16 ULP oracle; highlights are never
/// clipped (§2.1).
pub fn temperature(rgba: &mut [f32], gain_r: f32, gain_b: f32, mix: f32) {
    if gain_r == 1.0 && gain_b == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let sr = px[0] * gain_r;
        let sb = px[2] * gain_b;
        px[0] = px[0] * (1.0 - mix) + sr * mix;
        px[2] = px[2] * (1.0 - mix) + sb * mix;
    }
}

/// Invert (docs/08 §3.23): the colour inverse `out.rgb = 1 − u` per RGB
/// channel in the compositor's scene-linear working space, on
/// unpremultiplied colour (§2.2), re-premultiplied on the way out — exactly
/// Contrast's and Gamma's premultiply handling. `1 − c` is affine, so it does
/// not commute with premultiplied alpha: the pixel is unpremultiplied,
/// inverted, then re-premultiplied, so matte edges do not fringe. The inverse
/// is a plain `1 − c` in scene-linear light — the owner's "simple inverse" —
/// so HDR values above 1 invert to honest negatives, never clipped (§2.1).
/// There is no neutral value (invert always inverts); Mix 0 is the bit-exact
/// identity (the `× (1 − mix) + · × mix` blend collapses to the input), and
/// the WGSL twin matches. Purely continuous, so it is safe under the §1.6
/// fp16 ULP oracle. Alpha is untouched.
pub fn invert(rgba: &mut [f32], mix: f32) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        for c in 0..3 {
            let inverted = (1.0 - u[c]) * a;
            px[c] = px[c] * (1.0 - mix) + inverted * mix;
        }
    }
}

/// Tint (docs/08 §3.24): a luminance duotone / gradient map
/// `out.rgb = black + (white − black)·luma(u)` per RGB channel, with Rec.709
/// `luma` on the unpremultiplied colour `u` (§2.2), re-premultiplied on the
/// way out — exactly Contrast's and Gamma's premultiply handling. A
/// luma-driven colour remap does not commute with premultiplied alpha, so the
/// pixel is unpremultiplied, mapped, then re-premultiplied, and matte edges do
/// not fringe. The lerp is written `black + (white − black)·luma` (not the
/// `black·(1 − luma) + white·luma` form) so the CPU reference and the WGSL
/// kernel reduce in the same order and the §1.6 oracle holds. The default
/// black→black / white→white maps every pixel to its own luma (a greyscale) —
/// a visible tasteful default, not a no-op; Mix 0 is the bit-exact identity
/// (the WGSL twin matches). Purely continuous, so it is safe under the §1.6
/// fp16 ULP oracle. Alpha is untouched.
pub fn tint(rgba: &mut [f32], black: [f32; 3], white: [f32; 3], mix: f32) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let luma = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
        for c in 0..3 {
            let mapped = black[c] + (white[c] - black[c]) * luma;
            let graded = mapped * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// Vignette (docs/08 §3.14): darkens toward black away from the frame
/// centre, on premultiplied colour — a coverage-like darkening, not a
/// colour grade, so no unpremultiply round trip (alpha is untouched).
/// Roundness blends the distance metric between a true circle (1: both
/// axes normalised by the shorter side, so equal pixel distances read
/// as equal) and an ellipse that exactly reaches the frame's own edges
/// (0: each axis normalised by its own half-extent) — the schema's own
/// description of the knob. Radius is the clear centre's reach in that
/// normalised metric (1.0 = the metric's own reference edge) and
/// Softness the feather beyond it; the feather width floors at a small
/// epsilon so Softness 0 reads as a hard edge rather than a division by
/// zero. Amount 0 is the neutral point (bit-exact passthrough, pinned
/// by test — the WGSL twin matches).
#[allow(clippy::too_many_arguments)]
pub fn vignette(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount: f32,
    radius: f32,
    softness: f32,
    roundness: f32,
    mix: f32,
) {
    if amount == 0.0 {
        return;
    }
    let (fw, fh) = (w as f32, h as f32);
    if fw <= 0.0 || fh <= 0.0 {
        return;
    }
    let half = fw.min(fh) * 0.5;
    let rx = (fw * 0.5) * (1.0 - roundness) + half * roundness;
    let ry = (fh * 0.5) * (1.0 - roundness) + half * roundness;
    let (cx, cy) = (fw * 0.5, fh * 0.5);
    let edge0 = radius;
    let edge1 = radius + softness.max(1e-6);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let nx = (x as f32 + 0.5 - cx) / rx;
            let ny = (y as f32 + 0.5 - cy) / ry;
            let dist = (nx * nx + ny * ny).sqrt();
            let t = ((dist - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            let s = t * t * (3.0 - 2.0 * t);
            let vig = (s * amount).clamp(0.0, 1.0);
            let keep = 1.0 - vig;
            for c in 0..3 {
                let darkened = rgba[i + c] * keep;
                rgba[i + c] = rgba[i + c] * (1.0 - mix) + darkened * mix;
            }
        }
    }
}

/// Flash (docs/08 §3.7, manual form): blend each pixel toward the flash
/// colour by the evaluated strength. The colour is scaled by the pixel's
/// own alpha so the flash respects the layer's footprint (a transparent
/// region never lights up); alpha itself is untouched.
pub fn flash(rgba: &mut [f32], strength: f32, colour: [f32; 4], mix: f32) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        for c in 0..3 {
            let lit = px[c] * (1.0 - strength) + colour[c] * a * strength;
            px[c] = px[c] * (1.0 - mix) + lit * mix;
        }
    }
}

/// The §1.6 oracle for Echo (docs/08 §3.13): the CPU twin of `fx_echo.wgsl`,
/// op-for-op. `current` is the leading (this-frame) linear premultiplied
/// RGBA; `neighbours` are the layer's decoded source frames keyed by their
/// frame offset (all the same length as `current`). `weights[i]` is the
/// tap intensity for the echo at offset `-(i+1)`; a zero weight or a
/// missing neighbour is skipped. `mode` is 0 = Add, 1 = Behind (the
/// accumulator over the echo), 2 = Max. Finally the trail is blended
/// toward `current` by `mix`. Working colour is premultiplied, so a tap
/// scales all four channels together — the correct premultiplied fade.
pub fn echo(
    current: &[f32],
    neighbours: &[(i32, &[f32])],
    weights: [f32; 8],
    mode: u32,
    mix: f32,
) -> Vec<f32> {
    let mut out = current.to_vec();
    for (px_idx, o) in out.chunks_exact_mut(4).enumerate() {
        let mut acc = [
            current[px_idx * 4],
            current[px_idx * 4 + 1],
            current[px_idx * 4 + 2],
            current[px_idx * 4 + 3],
        ];
        for (i, &weight) in weights.iter().enumerate() {
            if weight <= 0.0 {
                continue;
            }
            let offset = -(i as i32 + 1);
            let Some((_, buf)) = neighbours.iter().find(|(oo, _)| *oo == offset) else {
                continue;
            };
            let base = px_idx * 4;
            let n = [
                buf[base] * weight,
                buf[base + 1] * weight,
                buf[base + 2] * weight,
                buf[base + 3] * weight,
            ];
            acc = match mode {
                0 => [acc[0] + n[0], acc[1] + n[1], acc[2] + n[2], acc[3] + n[3]],
                1 => {
                    let k = 1.0 - acc[3];
                    [
                        acc[0] + n[0] * k,
                        acc[1] + n[1] * k,
                        acc[2] + n[2] * k,
                        acc[3] + n[3] * k,
                    ]
                }
                _ => [
                    acc[0].max(n[0]),
                    acc[1].max(n[1]),
                    acc[2].max(n[2]),
                    acc[3].max(n[3]),
                ],
            };
        }
        for c in 0..4 {
            o[c] = current[px_idx * 4 + c] * (1.0 - mix) + acc[c] * mix;
        }
    }
    out
}

/// The §1.6 oracle for Fast motion blur (docs/08 §3.2): the CPU twin of
/// `fx_motionblur.wgsl`, op-for-op. `rgba` is linear premultiplied RGBA,
/// mutated in place; `u`/`v` are the per-pixel forward flow (pixels of
/// this raster, one entry per pixel) the decode worker measured between
/// the current source frame and the next, and `conf` is the matching
/// per-pixel confidence in 0..1 ([`lumit_flow::confidence`]). Each pixel's
/// streak vector is its own motion scaled by `shutter_frac` (shutter ÷ 360)
/// **and by its confidence** (FX-19): a suspect pixel shortens its streak
/// smoothly toward no blur, so occlusions and motion boundaries fade out
/// instead of leaving a hard cut. The streak is a centred box integral of
/// `samples` evenly spaced bilinear taps — the same line-integral shape as
/// [`blur_directional`], but per-pixel directed by the flow rather than one
/// global angle. Fixed tap order and count for determinism (§2.4). Edges
/// clamp (the shared [`bilinear`] rule), so a full-frame smear never darkens
/// the border. A zero streak — `shutter_frac == 0.0`, a still pixel, or zero
/// confidence — collapses every tap onto the pixel itself, so with
/// `mix == 1.0` the result is the bit-exact input. `view` selects the output:
/// the blurred picture, the colour-coded flow, or the confidence as greyscale
/// (the diagnostic views ignore `mix` — they show the field itself).
#[allow(clippy::too_many_arguments)]
pub fn motion_blur(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    u: &[f32],
    v: &[f32],
    conf: &[f32],
    shutter_frac: f32,
    samples: i32,
    mix: f32,
    view: MbView,
) {
    let original = rgba.to_vec();
    let n = samples.max(1);
    let nf = n as f32;
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let i = idx * 4;
            let out: [f32; 4] = match view {
                MbView::Rendered => {
                    let pos = (x as f32 + 0.5, y as f32 + 0.5);
                    // The full streak vector: this pixel's inter-frame motion,
                    // shortened by the shutter fraction and its confidence.
                    let c = conf[idx];
                    let sv = (u[idx] * shutter_frac * c, v[idx] * shutter_frac * c);
                    let mut acc = [0.0f32; 4];
                    for k in 0..n {
                        let t = (k as f32 + 0.5) / nf - 0.5;
                        let s = bilinear(&original, w, h, pos.0 + t * sv.0, pos.1 + t * sv.1);
                        for cc in 0..4 {
                            acc[cc] += s[cc];
                        }
                    }
                    let mut o = [0.0f32; 4];
                    for cc in 0..4 {
                        let vv = acc[cc] / nf;
                        o[cc] = original[i + cc] * (1.0 - mix) + vv * mix;
                    }
                    o
                }
                MbView::MotionVectors => {
                    // Colour-code the raw flow: red = +x, green = +y, mid-grey
                    // = still. Opaque (premultiplied, alpha 1). k maps ±16 px to
                    // the full 0..1 range.
                    let k = 1.0 / 32.0;
                    [
                        (0.5 + u[idx] * k).clamp(0.0, 1.0),
                        (0.5 + v[idx] * k).clamp(0.0, 1.0),
                        0.5,
                        1.0,
                    ]
                }
                MbView::Confidence => {
                    let c = conf[idx].clamp(0.0, 1.0);
                    [c, c, c, 1.0]
                }
            };
            rgba[i..i + 4].copy_from_slice(&out);
        }
    }
}

/// The §1.6 oracle for Datamosh (docs/08 §3.12, K-104): the CPU twin of
/// `fx_datamosh.wgsl`, op-for-op. `current` is the already block/scanline'd
/// frame (linear premultiplied RGBA) this section blends against; `prev`
/// is the raw -1 source neighbour; `u`/`v` are the dense flow the decode
/// worker measured from the current frame to it (this raster's pixel
/// grid, one entry per pixel — the same current→neighbour convention
/// [`motion_blur`] uses for its own +1 neighbour, just pointed at -1). A
/// single bilinear tap per pixel, not a streak integral: this looks up
/// one displaced source pixel (motion-compensated prediction), not a
/// line integral of motion. `intensity == 0.0` collapses every warped tap
/// weight to zero, so the result is the bit-exact `current` input.
pub fn datamosh(
    current: &[f32],
    prev: &[f32],
    w: u32,
    h: u32,
    u: &[f32],
    v: &[f32],
    intensity: f32,
) -> Vec<f32> {
    let mut out = current.to_vec();
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let i = idx * 4;
            let pos = (x as f32 + 0.5 + u[idx], y as f32 + 0.5 + v[idx]);
            let warped = bilinear(prev, w, h, pos.0, pos.1);
            for c in 0..4 {
                out[i + c] = current[i + c] * (1.0 - intensity) + warped[c] * intensity;
            }
        }
    }
    out
}

/// Rec. 709 luma weights, applied in linear light.
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The unpremultiplied colour of one premultiplied RGBA pixel. A fully
/// transparent pixel's colour is undefined, so it reads as black — the
/// WGSL kernels use the identical rule.
fn unpremult(px: &[f32]) -> [f32; 3] {
    if px[3] > 0.0 {
        [px[0] / px[3], px[1] / px[3], px[2] / px[3]]
    } else {
        [0.0; 3]
    }
}

/// Soft threshold: detail within ±t collapses to zero, detail beyond it
/// is shrunk by t — no hard step, so no contouring at the gate (§3.9's
/// noise suppression). Written as explicit branches so the WGSL twin
/// matches bit-for-bit.
fn soft_gate(d: f32, t: f32) -> f32 {
    if d > t {
        d - t
    } else if d < -t {
        d + t
    } else {
        0.0
    }
}

/// Clamp-addressed bilinear sample at continuous pixel-centre
/// coordinates (the texel at index x covers [x, x+1), centre x+0.5).
/// Written with the exact arithmetic order the WGSL kernels use.
fn bilinear(rgba: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> [f32; 4] {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let (wi, hi) = (w as i64, h as i64);
    let at = |x: i64, y: i64| {
        let s = ((y.clamp(0, hi - 1) * wi + x.clamp(0, wi - 1)) * 4) as usize;
        [rgba[s], rgba[s + 1], rgba[s + 2], rgba[s + 3]]
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let c00 = at(x0, y0);
    let c10 = at(x0 + 1, y0);
    let c01 = at(x0, y0 + 1);
    let c11 = at(x0 + 1, y0 + 1);
    let mut out = [0.0f32; 4];
    for c in 0..4 {
        let top = c00[c] * (1.0 - tx) + c10[c] * tx;
        let bottom = c01[c] * (1.0 - tx) + c11[c] * tx;
        out[c] = top * (1.0 - ty) + bottom * ty;
    }
    out
}

/// Chromatic aberration (docs/08 §3.6): R samples behind the offset, B
/// ahead of it, G and alpha stay put (alpha follows the green channel so
/// mattes never fringe). Linear mode shifts every pixel by the same
/// vector; radial mode scales the pixel's own offset from the frame
/// centre so aberration grows toward the corners (`amount_px` is reached
/// at the corner distance). `scale` is the per-channel displacement scale
/// `[r, g, b]` (FX-9): R and G sample along −offset·scale, B along
/// +offset·scale, so `[1, 0, 1]` is the classic split (R one way, B the
/// other, G on its own pixel). Sampling G with `bilinear` at scale 0 lands
/// exactly on its own pixel, bit-identical to reading it directly, so the
/// default reproduces the historical output. Premultiplied throughout;
/// samples outside the frame clamp to the edge.
#[allow(clippy::too_many_arguments)]
pub fn rgb_split(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    angle_deg: f32,
    radial: bool,
    scale: [f32; 3],
    mix: f32,
) {
    let original = rgba.to_vec();
    let (dx, dy) = super::rgb_split_offset(amount_px, angle_deg);
    let (fw, fh) = (w as f32, h as f32);
    let diag = (fw * fw + fh * fh).sqrt();
    let k = amount_px / (0.5 * diag);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let (ox, oy) = if radial {
                ((pos.0 - fw * 0.5) * k, (pos.1 - fh * 0.5) * k)
            } else {
                (dx, dy)
            };
            let r = bilinear(
                &original,
                w,
                h,
                pos.0 - ox * scale[0],
                pos.1 - oy * scale[0],
            )[0];
            let g = bilinear(
                &original,
                w,
                h,
                pos.0 - ox * scale[1],
                pos.1 - oy * scale[1],
            )[1];
            let b = bilinear(
                &original,
                w,
                h,
                pos.0 + ox * scale[2],
                pos.1 + oy * scale[2],
            )[2];
            let split = [r, g, b, original[i + 3]];
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + split[c] * mix;
            }
        }
    }
}

/// The RGB split's Wavelength mode (docs/08 §3.6, K-090; chromatic
/// aberration's own Wavelength mode, K-144): instead of three channels at
/// three offsets, `samples` spectral taps spread across `±offset`, each
/// weighted by its wavelength's linear-RGB basis colour and summed — real
/// dispersion's rainbow fringe rather than the classic hard R/G/B rim. The
/// taps (each carrying its weight and its offset fraction in the `w` lane)
/// come from [`super::spectral_taps`], shared with the GPU path, and their
/// colour columns are normalised so a uniform image passes through
/// unchanged. More taps fill the same span more densely, so a large offset
/// disperses smoothly rather than showing a few discrete copies. Offsets
/// (linear or radial) and edge handling match the classic mode exactly;
/// alpha stays put, so mattes never fringe.
#[allow(clippy::too_many_arguments)]
pub fn spectral_split(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    angle_deg: f32,
    radial: bool,
    samples: i32,
    mix: f32,
) {
    let original = rgba.to_vec();
    let taps = super::spectral_taps(samples);
    let (dx, dy) = super::rgb_split_offset(amount_px, angle_deg);
    let (fw, fh) = (w as f32, h as f32);
    let diag = (fw * fw + fh * fh).sqrt();
    let k = amount_px / (0.5 * diag);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let (ox, oy) = if radial {
                ((pos.0 - fw * 0.5) * k, (pos.1 - fh * 0.5) * k)
            } else {
                (dx, dy)
            };
            let mut acc = [0.0f32; 3];
            for tap in &taps {
                let t = tap[3];
                let s = bilinear(&original, w, h, pos.0 + t * ox, pos.1 + t * oy);
                for c in 0..3 {
                    acc[c] += tap[c] * s[c];
                }
            }
            let split = [acc[0], acc[1], acc[2], original[i + 3]];
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + split[c] * mix;
            }
        }
    }
}

/// Chromatic aberration (docs/08 §3.15): a dedicated, always-radial
/// sibling of [`rgb_split`]'s own Radial mode — three tinted radial taps,
/// always centred on the frame, no angle or linear mode of its own. The
/// three taps sit at fractions −1 / 0 / +1 (toward centre / on the pixel /
/// away), each sampled and multiplied component-wise by its `tints[i]`
/// colour, then summed. Default tints red / green / blue keep only their
/// own channel — tap −1 → R (reads outward), tap 0 → G (its own pixel),
/// tap +1 → B (reads inward) — reproducing the classic split; G and alpha
/// stay put. Premultiplied throughout; samples outside the frame clamp to
/// the edge. Amount 0 is the bit-exact passthrough through the general
/// formula (`k` is an exact `0.0`, so every tap lands on its own pixel and
/// the tinted sum returns the input for the primary defaults) — no separate
/// short-circuit, mirroring `rgb_split`'s own un-guarded style.
pub fn chromatic_aberration(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    tints: [[f32; 3]; 3],
    mix: f32,
) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let diag = (fw * fw + fh * fh).sqrt();
    let k = amount_px / (0.5 * diag);
    let (cx, cy) = (fw * 0.5, fh * 0.5);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let (ox, oy) = ((pos.0 - cx) * k, (pos.1 - cy) * k);
            let mut acc = [0.0f32; 3];
            for (tap, tint) in [-1.0f32, 0.0, 1.0].iter().zip(tints.iter()) {
                let s = bilinear(&original, w, h, pos.0 + tap * ox, pos.1 + tap * oy);
                for c in 0..3 {
                    acc[c] += tint[c] * s[c];
                }
            }
            let split = [acc[0], acc[1], acc[2], original[i + 3]];
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + split[c] * mix;
            }
        }
    }
}

/// Unsharp mask (docs/08 §3.9) in linear light on unpremultiplied colour
/// (§2.2): detail = input − gaussian(input, radius), gated by the soft
/// threshold, scaled by amount and added back. The internal gaussian
/// always uses Repeat edges (blurring unpremultiplied colour against
/// transparent borders would invent dark detail). Undershoot clamps at
/// zero — negative light is not a thing — and alpha passes through.
#[allow(clippy::too_many_arguments)]
pub fn sharpen(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount: f32,
    radius_px: f32,
    threshold: f32,
    luma_only: bool,
    mix: f32,
) {
    let original = rgba.to_vec();
    // Unpremultiplied colour buffer, alpha carried along for the ride.
    let mut blurred = vec![0.0f32; rgba.len()];
    for (dst, src) in blurred.chunks_exact_mut(4).zip(original.chunks_exact(4)) {
        dst[..3].copy_from_slice(&unpremult(src));
        dst[3] = src[3];
    }
    blur_gaussian(&mut blurred, w, h, radius_px, 1, 1.0);
    for i in (0..rgba.len()).step_by(4) {
        let o = &original[i..i + 4];
        let u = unpremult(o);
        let b = &blurred[i..i + 3];
        let mut v = [0.0f32; 3];
        if luma_only {
            let d = soft_gate(
                (u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2])
                    - (b[0] * LUMA[0] + b[1] * LUMA[1] + b[2] * LUMA[2]),
                threshold,
            );
            for c in 0..3 {
                v[c] = u[c] + amount * d;
            }
        } else {
            for c in 0..3 {
                v[c] = u[c] + amount * soft_gate(u[c] - b[c], threshold);
            }
        }
        for c in 0..3 {
            let s = v[c].max(0.0) * o[3];
            rgba[i + c] = o[c] * (1.0 - mix) + s * mix;
        }
        rgba[i + 3] = o[3];
    }
}

/// Sharpen (docs/08 §3.9, K-138): the plain, radius-free sibling of the
/// [`sharpen`] Unsharp mask — a fixed 3×3 high-pass convolution scaled by
/// `amount`, in linear light on unpremultiplied colour (§2.2). For each pixel
/// `out.rgb = u + amount · (4·u − up − down − left − right)`, where `u` and
/// its four axis neighbours are the unpremultiplied colours; the neighbours
/// clamp to the edge pixel, so a border never invents dark detail. Undershoot
/// clamps at zero (no negative light), the result is re-premultiplied by the
/// centre alpha, and alpha passes through. `amount == 0.0` short-circuits to
/// the bit-exact input (the `× (1 − mix) + · × mix` blend, and the
/// unpremultiply → re-premultiply round trip, cannot both be relied on to be
/// bit-exact, so the neutral case returns early — the WGSL twin matches with
/// its own early store). Mix 0 is likewise the identity.
pub fn sharpen_simple(rgba: &mut [f32], w: u32, h: u32, amount: f32, mix: f32) {
    if amount == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let (wi, hi) = (w as i64, h as i64);
    // Unpremultiplied colour at a clamp-addressed integer pixel.
    let at = |x: i64, y: i64| -> [f32; 3] {
        let s = ((y.clamp(0, hi - 1) * wi + x.clamp(0, wi - 1)) * 4) as usize;
        unpremult(&original[s..s + 4])
    };
    for y in 0..hi {
        for x in 0..wi {
            let i = ((y * wi + x) * 4) as usize;
            let a = original[i + 3];
            let c = at(x, y);
            let up = at(x, y - 1);
            let down = at(x, y + 1);
            let left = at(x - 1, y);
            let right = at(x + 1, y);
            for ch in 0..3 {
                let hp = 4.0 * c[ch] - up[ch] - down[ch] - left[ch] - right[ch];
                let sharpened = (c[ch] + amount * hp).max(0.0) * a;
                rgba[i + ch] = original[i + ch] * (1.0 - mix) + sharpened * mix;
            }
            rgba[i + 3] = original[i + 3];
        }
    }
}

/// Gaussian tap weights for a half-width `r` (σ = r/2, the visible
/// extent reading), normalised. r = 0 → identity single tap.
pub fn gaussian_weights(radius_px: f32) -> Vec<f32> {
    let r = radius_px.ceil().max(0.0) as i32;
    if r == 0 {
        return vec![1.0];
    }
    let sigma = (radius_px * 0.5).max(1e-3);
    let mut w: Vec<f32> = (-r..=r)
        .map(|i| (-0.5 * (i as f32 / sigma).powi(2)).exp())
        .collect();
    let sum: f32 = w.iter().sum();
    for v in &mut w {
        *v /= sum;
    }
    w
}

/// Resolve a sample index under an edge policy; None = transparent.
fn edge_index(i: i64, len: i64, edge: u32) -> Option<i64> {
    if (0..len).contains(&i) {
        return Some(i);
    }
    match edge {
        1 => Some(i.clamp(0, len - 1)), // repeat edge pixel
        2 => {
            // mirror: reflect without repeating the edge sample
            let m = if i < 0 { -i } else { 2 * (len - 1) - i };
            Some(m.clamp(0, len - 1))
        }
        _ => None, // transparent
    }
}

/// The directional blur's tap count for a streak length in pixels —
/// shared with the GPU op construction so both paths dispatch the same
/// kernel size (§1.6).
pub fn dir_blur_taps(length_px: f32) -> i32 {
    (length_px.ceil() as i32).clamp(1, 511)
}

/// The radial blur's tap count for a peak per-pixel spread in pixels
/// (docs/08 §3.8): the same rule as [`dir_blur_taps`], sized from the
/// worst case — the spread reached at the frame's farthest corner —
/// so CPU and GPU dispatch the same kernel size everywhere in the
/// image (nearer Centre simply over-samples a shorter true spread,
/// which costs taps but is never wrong).
pub fn radial_blur_taps(amount_px: f32) -> i32 {
    dir_blur_taps(amount_px)
}

/// Bilinear sample under a blur edge policy: out-of-frame taps repeat or
/// mirror per axis, or read as transparent (contributing nothing while
/// keeping full weight, exactly like the gaussian's normalisation).
fn bilinear_edge(rgba: &[f32], w: u32, h: u32, sx: f32, sy: f32, edge: u32) -> [f32; 4] {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let (wi, hi) = (w as i64, h as i64);
    let at = |x: i64, y: i64| match (edge_index(x, wi, edge), edge_index(y, hi, edge)) {
        (Some(x), Some(y)) => {
            let s = ((y * wi + x) * 4) as usize;
            [rgba[s], rgba[s + 1], rgba[s + 2], rgba[s + 3]]
        }
        _ => [0.0; 4],
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let c00 = at(x0, y0);
    let c10 = at(x0 + 1, y0);
    let c01 = at(x0, y0 + 1);
    let c11 = at(x0 + 1, y0 + 1);
    let mut out = [0.0f32; 4];
    for c in 0..4 {
        let top = c00[c] * (1.0 - tx) + c10[c] * tx;
        let bottom = c01[c] * (1.0 - tx) + c11[c] * tx;
        out[c] = top * (1.0 - ty) + bottom * ty;
    }
    out
}

/// Directional blur (docs/08 §3.8): a line integral along the angle —
/// evenly spaced bilinear taps across a segment `length_px` long centred
/// on the pixel, box weighted, normalised over the full kernel whatever
/// the edge policy (matching the gaussian's rule). Fixed tap order for
/// determinism (§2.4).
pub fn blur_directional(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    length_px: f32,
    angle_deg: f32,
    edge: u32,
    mix: f32,
) {
    let original = rgba.to_vec();
    let (dx, dy) = super::rgb_split_offset(1.0, angle_deg); // unit vector
    let n = dir_blur_taps(length_px);
    let nf = n as f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let mut acc = [0.0f32; 4];
            for k in 0..n {
                let t = ((k as f32 + 0.5) / nf - 0.5) * length_px;
                let s = bilinear_edge(&original, w, h, pos.0 + t * dx, pos.1 + t * dy, edge);
                for c in 0..4 {
                    acc[c] += s[c];
                }
            }
            for c in 0..4 {
                let v = acc[c] / nf;
                rgba[i + c] = original[i + c] * (1.0 - mix) + v * mix;
            }
        }
    }
}

/// Radial blur (docs/08 §3.8, schema status note): Spin samples along
/// an arc about Centre, Zoom along a ray through it — box-weighted,
/// evenly spaced taps across `[-0.5, 0.5]` exactly like
/// [`blur_directional`]'s line integral, fixed tap order for
/// determinism (§2.4). Both reduce to one linear scale of `d = pos −
/// centre`: Zoom's ray is `pos + t·k·d` (an exact sample along the ray,
/// since scaling `d` moves along the straight line through Centre and
/// `pos`); Spin's arc is `pos + t·k·rot90(d)` (the first-order/tangent
/// approximation to true rotation about Centre — accurate for the
/// small sweep angles `k` reaches across the shipped Amount range).
/// `k = amount_px / (half the raster diagonal)` is the same radial
/// scale [`rgb_split`]'s radial mode uses. Neither branch divides by
/// `|d|`, so every tap collapses to exactly `pos` at Centre — no
/// epsilon guard, no NaN risk. `amount_px == 0.0` gives `k == 0.0`,
/// [`radial_blur_taps`] floors at one tap (mirroring
/// [`dir_blur_taps`]'s floor), and that single tap sits at exactly
/// `pos`: with `mix == 1.0` the result is the bit-exact input (pinned
/// by test, matching the directional blur's own zero-length case).
#[allow(clippy::too_many_arguments)]
pub fn blur_radial(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    centre_frac: [f32; 2],
    amount_px: f32,
    spin: bool,
    edge: u32,
    mix: f32,
) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let centre = (centre_frac[0] * fw, centre_frac[1] * fh);
    let diag = (fw * fw + fh * fh).sqrt();
    let k = if diag > 0.0 {
        amount_px / (0.5 * diag)
    } else {
        0.0
    };
    let n = radial_blur_taps(amount_px);
    let nf = n as f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let d = (pos.0 - centre.0, pos.1 - centre.1);
            // Zoom steps along d itself (a ray through Centre); Spin
            // steps along its perpendicular (the tangent to the arc).
            let step = if spin { (-d.1, d.0) } else { d };
            let mut acc = [0.0f32; 4];
            for t in 0..n {
                let tt = (t as f32 + 0.5) / nf - 0.5;
                let s = bilinear_edge(
                    &original,
                    w,
                    h,
                    pos.0 + tt * k * step.0,
                    pos.1 + tt * k * step.1,
                    edge,
                );
                for c in 0..4 {
                    acc[c] += s[c];
                }
            }
            for c in 0..4 {
                let v = acc[c] / nf;
                rgba[i + c] = original[i + c] * (1.0 - mix) + v * mix;
            }
        }
    }
}

/// Separable two-pass gaussian on premultiplied RGBA (docs/08 §3.8),
/// fixed tap order for determinism (§2.4).
pub fn blur_gaussian(rgba: &mut [f32], w: u32, h: u32, radius_px: f32, edge: u32, mix: f32) {
    let (w, h) = (w as i64, h as i64);
    let weights = gaussian_weights(radius_px);
    let r = (weights.len() / 2) as i64;
    if r == 0 && (mix - 1.0).abs() < f32::EPSILON {
        return;
    }
    let original = rgba.to_vec();
    let mut pass = vec![0.0f32; rgba.len()];
    // Horizontal.
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (k, wt) in weights.iter().enumerate() {
                if let Some(sx) = edge_index(x + k as i64 - r, w, edge) {
                    let s = ((y * w + sx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += rgba[s + c] * wt;
                    }
                }
            }
            let d = ((y * w + x) * 4) as usize;
            pass[d..d + 4].copy_from_slice(&acc);
        }
    }
    // Vertical, blending the host Mix against the untouched input.
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (k, wt) in weights.iter().enumerate() {
                if let Some(sy) = edge_index(y + k as i64 - r, h, edge) {
                    let s = ((sy * w + x) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += pass[s + c] * wt;
                    }
                }
            }
            let d = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                rgba[d + c] = original[d + c] * (1.0 - mix) + acc[c] * mix;
            }
        }
    }
}

/// Block glitch (docs/08 §3.12, split out by K-107): standalone block
/// displacement, the block section of the old combined Glitch effect.
///
/// Partitions the raster into a `block_size_px` grid; each *nominal*
/// block hashes a small jitter offset (`jitter_frac` of `block_size_px`,
/// scaled by Intensity) that decides which block's content a pixel
/// actually reads from — a cheap stand-in for moving grid lines
/// themselves. That block then hashes its own displacement (±
/// `amount_px` per axis), R/B channel split (± `chan_px`, alpha follows
/// green exactly like [`rgb_split`]), and slice-repeat odds
/// (`slice_frac` × Intensity: folds the block's own local Y to a short
/// hashed repeat height instead of a plain read). Every hashed quantity
/// is scaled by Intensity, so Intensity 0 collapses every read back to
/// the pixel's own position — pinned as the bit-exact passthrough by
/// the early return below (matching [`glow`]'s neutral short-circuit,
/// not the tap-sum coincidence the blur family relies on, because
/// Mix should not be able to perturb a fully neutral instance either).
///
/// Clamp-addressed bilinear sampling throughout (like [`rgb_split`]);
/// fixed evaluation order for determinism (§2.4).
#[allow(clippy::too_many_arguments)]
pub fn block_glitch(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    intensity: f32,
    seed: u32,
    tick: i32,
    block_size_px: f32,
    jitter_frac: f32,
    amount_px: f32,
    chan_px: f32,
    slice_frac: f32,
    mix: f32,
) {
    if intensity == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let bw = block_size_px.max(1.0);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);

            let bx0 = (pos.0 / bw).floor();
            let by0 = (pos.1 / bw).floor();
            let h01 = |ch: u32, bxx: f32, byy: f32| {
                super::block_hash01(seed, ch, bxx as i32, byy as i32, tick)
            };
            // Grid jitter (status note): a hashed offset of the
            // *nominal* block, scaled by Intensity, decides which
            // block a pixel actually reads from.
            let jx = (h01(0, bx0, by0) - 0.5) * 2.0 * jitter_frac * bw * intensity;
            let jy = (h01(1, bx0, by0) - 0.5) * 2.0 * jitter_frac * bw * intensity;
            let jpos = (pos.0 + jx, pos.1 + jy);
            let bx = (jpos.0 / bw).floor();
            let by = (jpos.1 / bw).floor();

            let dx = (h01(2, bx, by) - 0.5) * 2.0 * amount_px * intensity;
            let dy = (h01(3, bx, by) - 0.5) * 2.0 * amount_px * intensity;
            let chan = (h01(4, bx, by) - 0.5) * 2.0 * chan_px * intensity;
            let slice_u = h01(5, bx, by);
            let slice_h_u = h01(6, bx, by);

            // Slice repeat: fold the block's own local Y to a short
            // hashed repeat height instead of a plain read.
            let mut eff_y = jpos.1;
            if slice_u < slice_frac * intensity {
                let local_y = jpos.1 - by * bw;
                let repeat_h = (slice_h_u * bw * 0.25).max(1.0);
                let folded = local_y - (local_y / repeat_h).floor() * repeat_h;
                eff_y = by * bw + folded;
            }
            let (sx, sy) = (jpos.0 + dx, eff_y + dy);

            // R/B split from the block hash (alpha follows green, like
            // rgb_split).
            let r = bilinear(&original, w, h, sx - chan, sy)[0];
            let g = bilinear(&original, w, h, sx, sy);
            let b = bilinear(&original, w, h, sx + chan, sy)[2];
            let c = [r, g[1], b, g[3]];

            for ch in 0..4 {
                rgba[i + ch] = original[i + ch] * (1.0 - mix) + c[ch] * mix;
            }
        }
    }
}

/// Scanlines (docs/08 §3.12, split out by K-107): standalone periodic
/// darken, the scanline section of the old combined Glitch effect. No
/// hash, no block resample — reads the input pixel directly (pointwise,
/// [`Roi::Exact`](super::Roi::Exact)), darkens by a periodic band in
/// raster Y (plus the precomputed roll offset), alternating which half
/// of the period darkens on odd periods when Interlace is on. Intensity
/// 0 is the bit-exact passthrough, pinned by the early return below —
/// the same neutral shape [`block_glitch`] uses.
#[allow(clippy::too_many_arguments)]
pub fn scanlines(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    intensity: f32,
    period_px: f32,
    darkness: f32,
    roll_px: f32,
    interlace: bool,
    mix: f32,
) {
    if intensity == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let period = period_px.max(1.0);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos_y = y as f32 + 0.5;
            let mut c = [
                original[i],
                original[i + 1],
                original[i + 2],
                original[i + 3],
            ];

            let yp = pos_y + roll_px;
            let cell = yp / period;
            let cell_floor = cell.floor();
            let t = cell - cell_floor;
            let odd = (cell_floor as i64).rem_euclid(2) != 0;
            let bright = (t < 0.5) != (interlace && odd);
            let band = if bright { 1.0 } else { 1.0 - darkness };
            let eff_mult = 1.0 - intensity * (1.0 - band);
            c[0] *= eff_mult;
            c[1] *= eff_mult;
            c[2] *= eff_mult;

            for ch in 0..4 {
                rgba[i + ch] = original[i + ch] * (1.0 - mix) + c[ch] * mix;
            }
        }
    }
}
