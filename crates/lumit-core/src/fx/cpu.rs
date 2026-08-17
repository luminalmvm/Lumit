use super::{MatteKeyParams, MbView, Resolved, MAX_BLADES};

/// Apply one resolved effect to an RGBA f32 image (premultiplied,
/// linear light), in place.
pub fn apply(rgba: &mut [f32], w: u32, h: u32, fx: &Resolved) {
    match fx {
        // Light wrap needs a second picture — the Background layer — which
        // this single-image entry point has no way to receive, exactly as
        // Depth of field's depth pass does not. Both are driven through their
        // own functions by the callers that hold the extra texture; here they
        // are the passthrough rather than a silent half-effect.
        Resolved::LightWrap { .. } => {}
        Resolved::MatteKey(p) => matte_key(rgba, p),
        // Shake is a transform-domain effect (docs/08 §3.4): the
        // resolved wobble maps to the Transform reference through the
        // same shared affine the GPU dispatch uses, so both paths
        // consume bit-identical numbers. A neutral shake (zero wobble)
        // maps to the identity affine — the bit-exact passthrough the
        // Transform reference pins. `edge` is Shake's own Edges control.
        // With motion blur (T18) the wobble is resampled at each sub-frame
        // placement and the results averaged; without it, one resample.
        Resolved::Shake {
            offset_px,
            rotation_deg,
            zoom,
            edge,
            mix,
            mb,
        } => match mb {
            Some(samples) => {
                let mut ops = [([1.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0]); super::SHAKE_MB_SAMPLES];
                for (op, s) in ops.iter_mut().zip(samples.iter()) {
                    let (anchor, position, scale, rot) =
                        super::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                    let (m, o, _opacity) = super::transform_op(anchor, position, scale, rot, 1.0);
                    *op = (m, o);
                }
                transform_average(rgba, w, h, &ops, *edge, *mix);
            }
            None => {
                let (anchor, position, scale, rot) =
                    super::shake_affine(w, h, *offset_px, *rotation_deg, *zoom);
                transform(rgba, w, h, anchor, position, scale, rot, *edge, 1.0, *mix);
            }
        },
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
        // Depth of field. The depth is a texture (the referenced layer
        // rendered alone) that never reaches this single-buffer dispatcher, so
        // the effect is identity here — like Echo, Motion blur and LUT — and
        // its §1.6 oracle runs through `dof` directly from the lumit-gpu test,
        // which can upload one. An UNSET depth reference is the effect's
        // labelled no-op on every path, so there is no second case to serve.
        Resolved::Dof { .. } => {}
        // Lens flare is GPU-only (K-256, the K-114 LUT precedent): its render
        // pass and baked textures never reach this single-buffer dispatcher,
        // so the CPU-degradation rung renders it as identity. The §1.6 oracle
        // is staged in the lumit-gpu tests (trace at ULP, frame at the
        // perceptual bound) against `lens_flare::cpu_flare`/`cpu_combine`.
        Resolved::LensFlare(..) => {}
        // A migrated effect's parameters live in the stack's arena, which this
        // single-op entry point has no way to receive — exactly as Light wrap's
        // background and Depth of field's depth pass do not reach it. The
        // dispatch that *does* have the arena is [`apply_stack`]; here the op is
        // the passthrough, never a silent half-effect.
        Resolved::Registry { .. } => {}
    }
}

/// Apply a whole resolved stack to an RGBA f32 image (premultiplied, linear
/// light), in place — the CPU-degradation rung (K-019) and the parity oracle's
/// entry point.
///
/// This is [`apply`] plus the one thing a single op cannot carry: the arena a
/// migrated effect's parameters live in (docs/impl/effect-registry.md §3, step
/// 4). An effect that has moved to the registry is dispatched through its own
/// [`EffectDef::apply_cpu`](super::EffectDef::apply_cpu) with its bag; an effect
/// that still carries a variant goes through [`apply`], which is the same
/// picture it always rendered.
pub fn apply_stack(rgba: &mut [f32], w: u32, h: u32, ops: &super::ResolvedOps) {
    for op in &ops.ops {
        match op {
            Resolved::Registry { op } => {
                // A dangling index cannot happen — resolve pushes the variant
                // and the bag together — but engine crates do not panic on a
                // caller's mistake (14-ENGINEERING-RULES §4), so an absent bag
                // is the passthrough.
                if let Some(fx) = ops.bags.get(*op as usize) {
                    fx.def.apply_cpu(rgba, w, h, fx.params);
                }
            }
            other => apply(rgba, w, h, other),
        }
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

/// The average of several transform resamples of one image — the reference for
/// the shake's own motion blur (T18, K-165). Each `(m, off)` is a host-computed
/// inverse affine (`lumit_core::fx::transform_op`): every output pixel centre is
/// resampled through all of them (opacity 1, the same `edge` policy and
/// `bilinear_edge` the single resample uses), the premultiplied results summed
/// in order and divided by the count, then blended against the untouched input
/// by `mix`. The accumulation order and the divide-by-count match the WGSL
/// kernel op-for-op, so both paths agree within the cheap-class fp16 bound
/// (§1.6). An empty `ops` is a defensive no-op (the caller never passes one).
pub fn transform_average(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    ops: &[([f32; 4], [f32; 2])],
    edge: u32,
    mix: f32,
) {
    if ops.is_empty() {
        return;
    }
    let original = rgba.to_vec();
    let n = ops.len() as f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let mut acc = [0.0f32; 4];
            for (m, o) in ops {
                let qx = m[0] * px + m[1] * py + o[0];
                let qy = m[2] * px + m[3] * py + o[1];
                let s = bilinear_edge(&original, w, h, qx, qy, edge);
                for c in 0..4 {
                    acc[c] += s[c];
                }
            }
            for c in 0..4 {
                let v = acc[c] / n;
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

/// The screen's primary channel index (0 R, 1 G, 2 B) and its two secondary
/// indices, chosen from the screen colour's largest component. Ties resolve
/// green > red > blue; the WGSL kernel runs the identical comparisons on the same
/// `key`, so both pick the same axis for a given screen colour.
fn matte_key_axis(key: [f32; 3]) -> (usize, usize, usize) {
    if key[1] >= key[0] && key[1] >= key[2] {
        (1, 0, 2) // green primary
    } else if key[0] >= key[1] && key[0] >= key[2] {
        (0, 1, 2) // red primary
    } else {
        (2, 0, 1) // blue primary
    }
}

/// The balance-weighted secondary reference of a colour (docs/08 §3.21): the two
/// non-screen channels blended by `balance` (0 = their min, 1 = their max, 0.5 =
/// their average). The primary is measured against this to tell screen from
/// foreground. Continuous (min/max/lerp), so the §1.6 oracle holds.
fn matte_key_secref(c: [f32; 3], si: usize, sj: usize, balance: f32) -> f32 {
    let lo = c[si].min(c[sj]);
    let hi = c[si].max(c[sj]);
    balance * hi + (1.0 - balance) * lo
}

/// Matte key (docs/08 §3.21, K-121/K-154): a Keylight-style colour-difference
/// keyer, on straight (unpremultiplied) colour (§2.2) — unpremultiply → key +
/// despill → re-premultiply, exactly Saturation's premultiply handling. It is the
/// §1.6 oracle the WGSL kernel (`fx_matte_key.wgsl`) matches op-for-op, so preview
/// and export agree pixel-for-pixel (K-031).
///
/// **Screen matte.** The screen colour's largest channel is the *primary* axis
/// (green for a green screen); the other two are *secondaries*, blended by
/// [`matte_key_secref`] into a reference. A pixel's *screen difference* is
/// `primary − reference`: large on the screen, small or negative on the
/// foreground. Normalising by the screen colour's own difference gives 1 on the
/// exact screen and 0 on a neutral, so `matte = clamp(1 − gain·raw, 0, 1)` keys
/// the screen to 0 and keeps the foreground at 1. **Alpha bias** shifts what
/// counts as neutral (a grey bias is a no-op). **Clip black/white** then remap the
/// matte's ends and **clip rollback** eases those clips back toward the un-clipped
/// matte to recover fine detail. Everything is `clamp`/`min`/`max`/`lerp` —
/// continuous — so there is no hard step and the fp16 ULP oracle holds.
///
/// **Despill.** The primary channel is pulled down toward the (despill-bias
/// shifted) secondary reference by the `spill` fraction, draining screen tint from
/// kept pixels. **Replace method** then recolours where spill was removed: Source
/// keeps the original colour, Hard/Soft blend in the replace colour (Soft scaled
/// by the pixel's brightness), None leaves the despilled colour.
///
/// **View** selects the output — Final (the keyed picture), Screen matte (the
/// matte as greyscale), or Status (a continuous heat of the matte). Mix 0 is the
/// bit-exact identity (the blend collapses to the input) on every view.
pub fn matte_key(rgba: &mut [f32], p: &MatteKeyParams) {
    let key = [p.key[0], p.key[1], p.key[2]];
    let (pi, si, sj) = matte_key_axis(key);
    let bal = p.balance;

    // Alpha-bias neutral: subtracted from every screen difference, so a grey bias
    // (its primary equals its secondary reference) contributes zero and the matte
    // reduces to the plain colour difference.
    let ab = [p.alpha_bias[0], p.alpha_bias[1], p.alpha_bias[2]];
    let ab_off = ab[pi] - matte_key_secref(ab, si, sj, bal);
    // The screen colour's own biased difference, floored so the divide is safe.
    let sd = ((key[pi] - matte_key_secref(key, si, sj, bal)) - ab_off).max(1e-6);
    // Despill-bias neutral: raises the target the primary is clamped down to.
    let db = [p.despill_bias[0], p.despill_bias[1], p.despill_bias[2]];
    let db_off = db[pi] - matte_key_secref(db, si, sj, bal);

    let repl = [
        p.replace_colour[0],
        p.replace_colour[1],
        p.replace_colour[2],
    ];
    let den = (p.clip_white - p.clip_black).max(1e-6);

    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);

        // Screen matte, before clips: 1 on the neutral, 0 on the screen colour.
        let pd = (u[pi] - matte_key_secref(u, si, sj, bal)) - ab_off;
        let raw = pd / sd;
        let m0 = (1.0 - p.gain * raw).clamp(0.0, 1.0);
        // Clip black/white, then rollback recovers detail toward the pre-clip matte.
        let mc = ((m0 - p.clip_black) / den).clamp(0.0, 1.0);
        let m = mc + p.clip_rollback * (m0 - mc);

        // Unspill: pull the primary down toward the (bias-shifted) reference.
        let target = matte_key_secref(u, si, sj, bal) + db_off;
        let removed = (u[pi] - target).max(0.0);
        let despill = p.spill * removed;
        let mut despilled = u;
        despilled[pi] = u[pi] - despill;

        // Replace method: recolour where spill was removed (`t_repl` = how much).
        let t_repl = (despill / sd).clamp(0.0, 1.0);
        let dl = despilled[0] * LUMA[0] + despilled[1] * LUMA[1] + despilled[2] * LUMA[2];
        // The blends use the `a·(1−t) + b·t` form so they match WGSL `mix`
        // op-for-op (§1.6).
        let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] * (1.0 - t) + b[0] * t,
                a[1] * (1.0 - t) + b[1] * t,
                a[2] * (1.0 - t) + b[2] * t,
            ]
        };
        let rgb = match p.replace_method {
            0 => u,                              // Source: the original straight colour
            1 => lerp3(despilled, repl, t_repl), // Hard colour
            2 => lerp3(
                despilled,
                [repl[0] * dl, repl[1] * dl, repl[2] * dl],
                t_repl,
            ), // Soft colour
            _ => despilled,                      // None
        };

        // View select (all continuous in `m`, so the oracle holds).
        let (proc_rgb, proc_a) = match p.view {
            1 => ([m, m, m], 1.0), // Screen matte
            2 => {
                // Status: greyscale matte tinted where the matte is uncertain
                // (peaks at m = 0.5, zero at the fully-keyed/kept ends).
                let warn = 4.0 * m * (1.0 - m) * 0.5;
                (
                    [
                        m + warn * (1.0 - m),
                        m + warn * (0.3 - m),
                        m + warn * (0.3 - m),
                    ],
                    1.0,
                )
            }
            _ => {
                // Final: re-premultiply the keyed colour by the new alpha.
                let out_a = a * m;
                ([rgb[0] * out_a, rgb[1] * out_a, rgb[2] * out_a], out_a)
            }
        };

        // Mix against the untouched premultiplied input; Mix 0 is the identity.
        for c in 0..3 {
            px[c] = px[c] * (1.0 - p.mix) + proc_rgb[c] * p.mix;
        }
        px[3] = a * (1.0 - p.mix) + proc_a * p.mix;
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
    ramp: f32,
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
            // Gamma on the smoothstep falloff (T16): 1 leaves it unchanged.
            let s = (t * t * (3.0 - 2.0 * t)).powf(ramp);
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

/// The §1.6 oracle for Echo (docs/08 §3.13; blend modes + 16-echo cap since
/// FX-17/K-149): the CPU twin of `fx_echo.wgsl`, op-for-op. `current` is the
/// leading (this-frame) linear premultiplied RGBA; `neighbours` are the
/// layer's decoded source frames keyed by their frame offset (all the same
/// length as `current`). `weights[i]` is the tap intensity for the echo at
/// offset `-(i+1)`; a zero weight or a missing neighbour is skipped. `mode`
/// is the combine blend applied per tap (see [`echo_blend`]): 0 = Add,
/// 1 = Behind (the accumulator over the echo), 2 = Max, 3 = Screen,
/// 4 = Normal, 5 = Multiply, 6 = Overlay, 7 = Soft light, 8 = Hard light,
/// 9 = Darken. Finally the trail is blended toward `current` by `mix`.
/// Working colour is premultiplied linear, and every mode runs per channel on
/// all four (the correct premultiplied fade for a light trail — Echo does not
/// re-encode to the compositor's perceptual domain).
pub fn echo(
    current: &[f32],
    neighbours: &[(i32, &[f32])],
    weights: [f32; 16],
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
            acc = echo_blend(mode, acc, n);
        }
        for c in 0..4 {
            o[c] = current[px_idx * 4 + c] * (1.0 - mix) + acc[c] * mix;
        }
    }
    out
}

/// One Echo combine mode (docs/08 §3.13, FX-17/K-149, T21): fold the weighted
/// neighbour tap `n` into the running accumulator `a`, both premultiplied
/// linear RGBA. Written per channel with the exact arithmetic order the WGSL
/// `echo_accumulate` twin uses, so the two agree bit-for-bit (§1.6). Indices
/// 0/1 are the effect-only compositing orders (Behind / In front); 2..=13 are
/// the order-independent light-combine blend modes, each applied to all four
/// channels in the working linear space (not the compositor's perceptual sRGB
/// domain — Echo composites light trails, so it stays linear and premultiplied,
/// and this keeps CPU/GPU parity exact). The HSL / burn / dodge modes a layer
/// offers are deliberately absent (ill-defined on a premultiplied trail).
fn echo_blend(mode: u32, a: [f32; 4], n: [f32; 4]) -> [f32; 4] {
    let mut o = [0.0f32; 4];
    match mode {
        0 => {
            // Behind: the accumulator composited over the echo (ghosting).
            let k = 1.0 - a[3];
            for c in 0..4 {
                o[c] = a[c] + n[c] * k;
            }
        }
        1 => {
            // In front: the echo composited over the accumulator.
            let k = 1.0 - n[3];
            for c in 0..4 {
                o[c] = n[c] + a[c] * k;
            }
        }
        // 2..=13: the shared light-combine table, accumulator as backdrop.
        m => o = light_blend(m - 2, a, n),
    }
    o
}

/// The order-independent light-combine table Echo and the Lens flare share
/// (K-149, K-289, T21): `mode` 0..=9 is Add, Screen, Multiply, Overlay,
/// Soft light, Hard light, Lighten, Darken, Difference, Exclusion; 10 is
/// Subtract and anything higher is Divide, the catch-all both menus end on.
/// `d` is the backdrop (Echo's accumulator, the flare's layer), `s` the
/// source, both premultiplied linear RGBA, every mode per channel on all
/// four — light combined with light, never a perceptual re-encode. Written
/// in the exact arithmetic order both WGSL twins use, so CPU and GPU agree
/// bit-for-bit (§1.6).
pub(crate) fn light_blend(mode: u32, d: [f32; 4], s: [f32; 4]) -> [f32; 4] {
    let mut o = [0.0f32; 4];
    match mode {
        // Add: light sums.
        0 => {
            for c in 0..4 {
                o[c] = d[c] + s[c];
            }
        }
        // Screen.
        1 => {
            for c in 0..4 {
                o[c] = d[c] + s[c] - d[c] * s[c];
            }
        }
        // Multiply.
        2 => {
            for c in 0..4 {
                o[c] = d[c] * s[c];
            }
        }
        // Overlay = hard light with the backdrop as the switch.
        3 => {
            for c in 0..4 {
                o[c] = if d[c] <= 0.5 {
                    2.0 * d[c] * s[c]
                } else {
                    1.0 - 2.0 * (1.0 - d[c]) * (1.0 - s[c])
                };
            }
        }
        // Soft light (W3C), source = s, backdrop = d.
        4 => {
            for c in 0..4 {
                let dd = if d[c] <= 0.25 {
                    ((16.0 * d[c] - 12.0) * d[c] + 4.0) * d[c]
                } else {
                    d[c].sqrt()
                };
                o[c] = if s[c] <= 0.5 {
                    d[c] - (1.0 - 2.0 * s[c]) * d[c] * (1.0 - d[c])
                } else {
                    d[c] + (2.0 * s[c] - 1.0) * (dd - d[c])
                };
            }
        }
        // Hard light: the source is the switch.
        5 => {
            for c in 0..4 {
                o[c] = if s[c] <= 0.5 {
                    2.0 * d[c] * s[c]
                } else {
                    1.0 - 2.0 * (1.0 - d[c]) * (1.0 - s[c])
                };
            }
        }
        // Lighten (per-channel max).
        6 => {
            for c in 0..4 {
                o[c] = d[c].max(s[c]);
            }
        }
        // Darken (per-channel min).
        7 => {
            for c in 0..4 {
                o[c] = d[c].min(s[c]);
            }
        }
        // Difference.
        8 => {
            for c in 0..4 {
                o[c] = (d[c] - s[c]).abs();
            }
        }
        // Exclusion.
        9 => {
            for c in 0..4 {
                o[c] = d[c] + s[c] - 2.0 * d[c] * s[c];
            }
        }
        // Subtract, floored at black.
        10 => {
            for c in 0..4 {
                o[c] = (d[c] - s[c]).max(0.0);
            }
        }
        // Divide, floored at black (linear, unclamped above), and the
        // catch-all for an index no menu can produce.
        _ => {
            for c in 0..4 {
                o[c] = (d[c] / s[c].max(1e-6)).max(0.0);
            }
        }
    }
    o
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

/// Clamp-addressed bilinear sample of a two-channel flow field (`u`/`v` as
/// separate per-pixel arrays), the exact arithmetic order [`bilinear`] uses so
/// the WGSL `bilinear_flow` matches op-for-op. Used to re-sample the flow at
/// each streamline step of [`datamosh`], so the melt follows curved motion.
fn bilinear_uv(u: &[f32], v: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> (f32, f32) {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let (wi, hi) = (w as i64, h as i64);
    let at = |x: i64, y: i64| {
        let s = (y.clamp(0, hi - 1) * wi + x.clamp(0, wi - 1)) as usize;
        (u[s], v[s])
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (u00, v00) = at(x0, y0);
    let (u10, v10) = at(x0 + 1, y0);
    let (u01, v01) = at(x0, y0 + 1);
    let (u11, v11) = at(x0 + 1, y0 + 1);
    let lerp = |a: f32, b: f32, c: f32, d: f32| {
        let top = a * (1.0 - tx) + b * tx;
        let bottom = c * (1.0 - tx) + d * tx;
        top * (1.0 - ty) + bottom * ty
    };
    (lerp(u00, u10, u01, u11), lerp(v00, v10, v01, v11))
}

/// The §1.6 oracle for Datamosh (docs/08 §3.12, K-104; reworked to a
/// flow-driven melt by K-164/T19): the CPU twin of `fx_datamosh.wgsl`,
/// op-for-op. `current` is the already-effected frame (linear premultiplied
/// RGBA) the melt blends over; `prev` is the raw -1 source neighbour; `u`/`v`
/// are the dense current→previous flow the decode worker measured (this
/// raster's pixel grid, one entry per pixel — the same current→neighbour
/// convention [`motion_blur`] uses for its own +1 neighbour, just pointed at
/// -1).
///
/// Per pixel, a **streamline walk** of `steps` bilinear taps follows the flow
/// out of `prev`: starting at the pixel centre, each step re-samples the flow
/// at the current position and advances by `displacement / steps` of it (≈ one
/// frame of motion per step), then samples `prev` there. The samples accumulate
/// with a geometric weight `bloom^k` from the near end, so `bloom == 0` keeps
/// only the nearest step (a short, quickly-resetting trail) and `bloom == 1`
/// averages the whole walk evenly (a long melting bloom). The weighted mean is
/// the moshed prediction, blended over `current` by `intensity`. `intensity ==
/// 0.0` collapses the blend to zero, so the result is the bit-exact `current`
/// input regardless of the other parameters. Fixed tap order and count for
/// determinism (§2.4); edges clamp (the shared [`bilinear`] rule), so a walk
/// off-frame reads the border rather than darkening.
#[allow(clippy::too_many_arguments)]
pub fn datamosh(
    current: &[f32],
    prev: &[f32],
    w: u32,
    h: u32,
    u: &[f32],
    v: &[f32],
    intensity: f32,
    displacement: f32,
    bloom: f32,
    steps: i32,
) -> Vec<f32> {
    let mut out = current.to_vec();
    let n = steps.max(1);
    let step = displacement / n as f32;
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let i = idx * 4;
            let mut px = x as f32 + 0.5;
            let mut py = y as f32 + 0.5;
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0f32;
            let mut wt = 1.0f32;
            for _ in 0..n {
                let (fu, fv) = bilinear_uv(u, v, w, h, px, py);
                px += fu * step;
                py += fv * step;
                let s = bilinear(prev, w, h, px, py);
                for c in 0..4 {
                    acc[c] += s[c] * wt;
                }
                wsum += wt;
                wt *= bloom;
            }
            let inv = 1.0 / wsum;
            for c in 0..4 {
                let warped = acc[c] * inv;
                out[i + c] = current[i + c] * (1.0 - intensity) + warped * intensity;
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
    scale: [f32; 3],
    tints: [[f32; 3]; 3],
    mix: f32,
) {
    let original = rgba.to_vec();
    let (dx, dy) = super::rgb_split_offset(amount_px, angle_deg);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            // Three tinted taps (T17): taps 0/1 along −offset, tap 2 along
            // +offset, each sampled in full colour then multiplied by its
            // tint and summed. Defaults 100/0/100 % with red/green/blue tints
            // reproduce the classic channel-separated split bit-for-bit.
            let s0 = bilinear(
                &original,
                w,
                h,
                pos.0 - dx * scale[0],
                pos.1 - dy * scale[0],
            );
            let s1 = bilinear(
                &original,
                w,
                h,
                pos.0 - dx * scale[1],
                pos.1 - dy * scale[1],
            );
            let s2 = bilinear(
                &original,
                w,
                h,
                pos.0 + dx * scale[2],
                pos.1 + dy * scale[2],
            );
            let mut split = [0.0f32; 4];
            for c in 0..3 {
                split[c] = tints[0][c] * s0[c] + tints[1][c] * s1[c] + tints[2][c] * s2[c];
            }
            split[3] = original[i + 3];
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + split[c] * mix;
            }
        }
    }
}

/// The RGB split's Wavelength mode (docs/08 §3.6, K-090; chromatic
/// aberration's own Wavelength mode, K-144; picker-driven since A1/K-163):
/// instead of three hard-tinted taps, `samples` spectral taps spread across
/// `±offset`, each tinted by the three-colour picker sampled as a gradient
/// (`tints[0]` → `tints[1]` → `tints[2]` across the span) and summed — a smooth
/// coloured fringe rather than the classic hard three-tap rim. The taps (each
/// carrying its tint weight and its offset fraction in the `w` lane) come from
/// [`super::spectral_taps`], shared with the GPU path, and their colour columns
/// are normalised so a uniform image passes through unchanged (the dispersion
/// tints the fringe, never the exposure). More taps fill the same span more
/// densely, so a large offset disperses smoothly rather than showing a few
/// discrete copies. Offsets (linear or radial) and edge handling match the
/// classic mode exactly; alpha stays put, so mattes never fringe.
#[allow(clippy::too_many_arguments)]
pub fn spectral_split(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    angle_deg: f32,
    radial: bool,
    samples: i32,
    tints: [[f32; 3]; 3],
    mix: f32,
) {
    let original = rgba.to_vec();
    let taps = super::spectral_taps(samples, tints);
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
pub fn sharpen_simple(rgba: &mut [f32], w: u32, h: u32, amount: f32, radius: f32, mix: f32) {
    if amount == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    // Neighbour distance in pixels (T15): 1 = a 3×3 kernel, larger reads a
    // coarser neighbourhood. Host-rounded so CPU and GPU sample the same taps.
    let r = radius.round().max(1.0) as i64;
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
            let up = at(x, y - r);
            let down = at(x, y + r);
            let left = at(x - r, y);
            let right = at(x + r, y);
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

/// Depth of field's resolved scalars, gathered into one struct.
///
/// Two dozen arguments is not a signature anyone can call correctly, and the
/// WGSL kernel's uniform has the same fields in the same order — keeping them
/// together is what lets the §1.6 oracle set up both paths from one value and
/// makes a field added to one side an obvious omission on the other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DofParams {
    /// The in-focus depth, 0..1, when `use_focus_point` is false.
    pub focus: f32,
    /// Half-width of the sharp band around focus, 0..1.
    pub range: f32,
    /// Per-side maximum circle-of-confusion radius in raster pixels. With no
    /// depth bound the far side is the one uniform radius.
    pub near_aperture: f32,
    pub far_aperture: f32,
    pub blade_normals: [[f32; 2]; MAX_BLADES],
    pub blade_count: u32,
    pub apothem2: f32,
    /// 1 is the circle and takes the plain `r² ≤ coc²` test.
    pub roundness: f32,
    pub rim: f32,
    pub aspect_scale: [f32; 2],
    pub threshold: f32,
    /// `2^(Exposure/12)`; **1 is the plain arithmetic mean** and skips the
    /// tonal split entirely.
    pub bokeh_power: f32,
    pub repeat_edge: bool,
    pub depth_channel: u32,
    pub depth_invert: bool,
    pub use_focus_point: bool,
    pub focus_point: [f32; 2],
    /// The Profile control, resolved: a multiplier on the depth distance before
    /// the ramp. 1 is the plain full-range falloff; above 1 the transition
    /// bites sooner, below 1 it stretches past the range.
    pub gamma: f32,
    pub remove_edge_leak: f32,
    pub detect_edge_threshold: f32,
    /// 0 Rendered, 1 Depth map, 2 Focus map.
    pub display: u32,
    pub mix: f32,
}

/// One channel of an auxiliary picture, by the shared
/// [`CHANNEL_OPTIONS`](super::CHANNEL_OPTIONS) index — how a depth pass is
/// reduced to the single number an effect reads.
///
/// Arithmetic only, deliberately: no `atan2` for hue, no `pow`, nothing whose
/// exact form differs between Rust and WGSL (§1.6). Hue is the standard
/// piecewise-linear sixth-of-a-turn form rather than the polar one, and every
/// divide is guarded — a grey pixel has no hue and zero chroma must not become
/// a NaN in the middle of a depth read.
pub fn channel_of(rgba: &[f32], channel: u32) -> f32 {
    let (r, g, b, a) = (rgba[0], rgba[1], rgba[2], rgba[3]);
    match channel {
        1 => a,
        2 => r,
        3 => g,
        4 => b,
        // 0 and anything unknown: Rec.709 luminance, the same weights every
        // other effect in the suite uses. Right for a grey map whatever
        // combination of channels it was written to.
        _ => 0.2126 * r + 0.7152 * g + 0.0722 * b,
    }
}

/// Depth of field (docs/08 §3.22) — the CPU reference: the §1.6 oracle the WGSL
/// kernel must agree with, and the degradation ladder's fallback rung (K-019).
///
/// **In plain terms.** For every pixel it works out how far out of focus that
/// pixel is, opens an aperture of that size around it, and averages what it can
/// see through it. The aperture is a polygon rather than a disc when you ask for
/// one, and the average is a *power* mean rather than a flat one when you ask
/// for that — which is what keeps a small bright thing from being averaged into
/// nothing and lets it bloom into a ball instead.
///
/// `depth` is the referenced layer's picture at this raster, **RGBA** rather
/// than a single channel, because which channel carries depth is one of this
/// effect's controls. `None` is the unbound case: the whole frame defocuses
/// uniformly at `far_aperture`, with every part of the depth model already
/// neutralised by resolve. That case needs no texture, so this rung serves it
/// properly rather than dropping the effect (K-019 as written); the bound case
/// needs the depth texture and reaches the oracle through `lumit_gpu::fx::dof`.
///
/// **Every added control contributes nothing at its neutral value, and the
/// branches below are why** (K-313). Roundness 1 takes the plain circle test,
/// Concentration 0 and Remove edge leak 0 take the *unweighted* accumulation,
/// and Exposure 0 (power 1) takes the *unsplit* one — rather than multiplying
/// every tap by one and splitting it at a threshold it never crosses. A
/// weighted gather computes `Σ(c·w)/Σw`, which is not an identity in IEEE 754
/// even when every `w` is 1; nor is `min(c,t) + max(c−t,0)` reliably `c`. At
/// their defaults these three branches leave exactly the box-weighted disc
/// average this effect has always computed, to the bit — which is what let the
/// aperture land inside the shipped effect instead of beside it.
pub fn dof(rgba: &mut [f32], depth: Option<&[f32]>, w: u32, h: u32, p: &DofParams) {
    let wi = w as i32;
    let hi = h as i32;
    let original = rgba.to_vec();
    let bound = depth.is_some();

    // The depth at one pixel, post-channel-pick and post-invert.
    let depth_at = |dm: &[f32], i: usize| {
        let d = channel_of(&dm[i * 4..i * 4 + 4], p.depth_channel);
        if p.depth_invert {
            1.0 - d
        } else {
            d
        }
    };

    // Focus is either the number or whatever depth sits under the point — the
    // reason Focus distance greys out in the panel. The point is in this
    // raster's pixels already (resolve scaled it), and is clamped rather than
    // wrapped: a point dragged off the frame focuses on the nearest edge.
    let focus = match (depth, p.use_focus_point) {
        (Some(dm), true) => {
            let fx = (p.focus_point[0].floor() as i32).clamp(0, wi - 1);
            let fy = (p.focus_point[1].floor() as i32).clamp(0, hi - 1);
            depth_at(dm, (fy * wi + fx) as usize)
        }
        _ => p.focus,
    };

    let falloff = |d: f32| dof_falloff(d, focus, p.range, p.gamma);

    // The gather is unweighted unless something actually asks for weights, and
    // then it is weighted for every tap. Two paths, not one path with a factor.
    let weighted = p.rim != 0.0 || (p.remove_edge_leak > 0.0 && bound);
    // Likewise the tonal split: at power 1 the mean is the plain arithmetic one
    // and the split is skipped rather than performed and undone.
    let tonal = p.bokeh_power != 1.0;

    for y in 0..hi {
        for x in 0..wi {
            let pi = (y * wi + x) as usize;
            let oi = pi * 4;
            let d_centre = depth.map_or(0.0, |dm| depth_at(dm, pi));
            // The diagnostic views (mirror the kernel): write the view straight
            // out, ignoring the gather, the composite and Mix alike. Resolve
            // forces Rendered when no depth is bound, so these only ever run
            // with a real depth pass behind them.
            if p.display == 1 {
                // Depth map: what the effect is actually reading, after the
                // channel pick and the invert — the view that says whether the
                // pass is aligned, upside down, or crushed to its two ends.
                rgba[oi] = d_centre;
                rgba[oi + 1] = d_centre;
                rgba[oi + 2] = d_centre;
                rgba[oi + 3] = 1.0;
                continue;
            }
            if p.display == 2 {
                // Focus map: white where sharp, darkening out of focus — where
                // the effect thinks focus landed and how fast it falls away.
                let m = 1.0 - falloff(d_centre);
                rgba[oi] = m;
                rgba[oi + 1] = m;
                rgba[oi + 2] = m;
                rgba[oi + 3] = 1.0;
                continue;
            }
            // With no depth bound the frame defocuses uniformly; with one, the
            // per-side aperture scales the ramp. The near/far select flips only
            // at `d == focus`, where the falloff is 0, so the radius stays
            // continuous and the §1.6 oracle holds across it.
            let coc = match depth {
                None => p.far_aperture,
                Some(_) => {
                    let ap = if d_centre < focus {
                        p.near_aperture
                    } else {
                        p.far_aperture
                    };
                    ap * falloff(d_centre)
                }
            };
            // In focus: the aperture is a point, so the pixel keeps itself,
            // untouched by the gather, the composite and Mix alike — which is
            // also the only way a sharp pixel stays bit-exact under a weighted
            // gather (a single weighted tap computes `(c·w)/w`).
            if coc <= 0.0 {
                continue;
            }
            let coc2 = coc * coc;
            let ri = coc.ceil() as i32;

            // **Pass one: the brightest excess in the aperture, per channel** —
            // and only when the tonal split is on at all.
            //
            // The power mean cannot be computed as `(Σ c^p / n)^(1/p)` in f32.
            // At the top of the Exposure slider (`p ≈ 5.7`, and far worse under
            // an earlier fit that put it at 32) a channel at scene-linear 0.08
            // raises to 8e-36 and one at 0.05 to 2e-42, below the smallest
            // normal. Averaging those and rooting them back yields zero, so
            // **every channel below roughly 0.116 linear collapses to black**,
            // per channel independently, which reads as black holes and
            // saturated speckle rather than as a blur. A floor on the *mean*
            // cannot save it: the underflow has already happened in the taps.
            //
            // Factoring the largest excess `M` out first is the standard fix and
            // an exact identity:
            //
            //     (Σ w·c^p / Σw)^(1/p)  =  M · (Σ w·(c/M)^p / Σw)^(1/p)
            //
            // Every `c/M` is then in `[0, 1]`, the brightest tap contributes
            // exactly 1, and the mean is bounded below by that tap's share of
            // the weight — so nothing underflows and no floor is needed at all.
            // It costs a second walk of the aperture, which is why this is two
            // loops rather than one — and why an untonal gather skips it.
            let mut peak = [0.0f32; 4];
            if tonal {
                for dy in -ri..=ri {
                    for dx in -ri..=ri {
                        if in_aperture_shape(dx as f32, dy as f32, coc2, p).is_none() {
                            continue;
                        }
                        let (sx, sy) = if p.repeat_edge {
                            ((x + dx).clamp(0, wi - 1), (y + dy).clamp(0, hi - 1))
                        } else {
                            let (ox, oy) = (x + dx, y + dy);
                            if ox < 0 || oy < 0 || ox >= wi || oy >= hi {
                                continue;
                            }
                            (ox, oy)
                        };
                        let si = ((sy * wi + sx) * 4) as usize;
                        for c in 0..4 {
                            peak[c] = peak[c].max((original[si + c] - p.threshold).max(0.0));
                        }
                    }
                }
            }

            // Pass two: the gather proper.
            let mut acc_lo = [0.0f32; 4];
            let mut acc_hi = [0.0f32; 4];
            let mut n = 0.0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let Some(r2) = in_aperture_shape(dx as f32, dy as f32, coc2, p) else {
                        continue;
                    };
                    // Edge policy. Transparent contributes nothing *and keeps
                    // its weight*, so a gather running off the frame darkens
                    // toward the edge instead of brightening — the same reading
                    // `edge_index` gives the blur family.
                    let (sx, sy) = if p.repeat_edge {
                        ((x + dx).clamp(0, wi - 1), (y + dy).clamp(0, hi - 1))
                    } else {
                        let ox = x + dx;
                        let oy = y + dy;
                        if ox < 0 || oy < 0 || ox >= wi || oy >= hi {
                            n += if weighted {
                                tap_weight(r2, coc2, p)
                            } else {
                                1.0
                            };
                            continue;
                        }
                        (ox, oy)
                    };
                    let si = ((sy * wi + sx) * 4) as usize;

                    let mut wgt = if weighted {
                        tap_weight(r2, coc2, p)
                    } else {
                        1.0
                    };
                    // Edge leak: a tap sitting across a depth discontinuity, in
                    // *front* of this pixel, is sharp foreground colour bleeding
                    // into a defocused background — the standard artefact of
                    // gathering across an edge. Pull it back rather than drop
                    // it, so the suppression is continuous in the slider.
                    if weighted && p.remove_edge_leak > 0.0 {
                        if let Some(dm) = depth {
                            let dt = depth_at(dm, (sy * wi + sx) as usize);
                            if (dt - d_centre).abs() > p.detect_edge_threshold && dt < d_centre {
                                wgt *= 1.0 - p.remove_edge_leak;
                            }
                        }
                    }

                    for c in 0..4 {
                        let v = original[si + c];
                        if tonal {
                            acc_lo[c] += v.min(p.threshold) * wgt;
                            // Normalised by the brightest excess, so the ratio
                            // is in [0, 1] and its power cannot underflow (see
                            // pass one). A peak of zero means nothing in the
                            // aperture is above the threshold at all; the excess
                            // term is then zero and the plain average is the
                            // whole answer.
                            if peak[c] > 0.0 {
                                let e = (v - p.threshold).max(0.0) / peak[c];
                                acc_hi[c] += e.powf(p.bokeh_power) * wgt;
                            }
                        } else {
                            // The historical accumulation, unchanged: one sum,
                            // no split, and with `wgt` a literal 1 on the
                            // unweighted path the multiply is exact.
                            acc_lo[c] += v * wgt;
                        }
                    }
                    n += wgt;
                }
            }
            if n <= 0.0 {
                continue;
            }
            for c in 0..4 {
                // `M · (mean of the normalised powers)^(1/p)` — the identity
                // pass one factored out, put back together. No floor: the
                // brightest tap contributes exactly 1 to the sum, so the mean is
                // at least its share of the weight and nothing underflows.
                let rooted = if tonal && peak[c] > 0.0 {
                    peak[c] * (acc_hi[c] / n).powf(1.0 / p.bokeh_power)
                } else {
                    0.0
                };
                let v = acc_lo[c] / n + rooted;
                // The defocused result replaces the original, blended by Mix.
                // There is no composite menu: an effect that wants its balls
                // added over a sharp plate is an adjustment layer with a blend
                // mode, which is the mechanism that already exists for it.
                let o = original[oi + c];
                rgba[oi + c] = o * (1.0 - p.mix) + v * p.mix;
            }
        }
    }
}

/// The defocus falloff `s` in 0..1: 0 inside the sharp band
/// `|depth − focus| ≤ range`, ramping smoothstep to 1 as the depth distance
/// reaches the far extreme. Shared by the circle-of-confusion radius and the
/// Focus-map view, and mirrors `fx_dof.wgsl`'s operation for operation.
///
/// **`falloff` is what the Profile control sets, and it is the difference
/// between usable and all-or-nothing.** Without it the ramp reaches full blur
/// only at a depth *distance of the whole range*, which sounds gentle and is the
/// opposite. A real depth pass puts nearly all of its content in a narrow band
/// with one near object well outside it, so focusing anywhere leaves the scene
/// almost sharp and that one object almost fully blurred, with nothing in
/// between. Scaling the distance first is what puts the transition where the
/// content actually is: above 1 the ramp bites sooner (a hard, shallow depth of
/// field), below 1 it stretches out past the range so even the far extreme is
/// only softened. The host computes it, so the kernel sees a plain multiplier
/// and no `exp2` — and its neutral is exactly 1, a multiply that is exact in
/// IEEE 754, which is why this one control needs no branch around it.
///
/// The smoothstep is longhand rather than the built-in: its exact form is not
/// guaranteed to match across the two languages, and §1.6 measures exactly that.
pub fn dof_falloff(d: f32, focus: f32, range: f32, falloff: f32) -> f32 {
    let dist = (d - focus).abs();
    let denom = (1.0 - range).max(1e-4);
    let e = (((dist - range) / denom) * falloff).clamp(0.0, 1.0);
    e * e * (3.0 - 2.0 * e)
}

/// Whether a tap at offset `(dx, dy)` falls inside the aperture — the bool half
/// of [`in_aperture_shape`], public so the aperture's geometry can be tested
/// directly. The scan box's correctness rests on every accepted tap lying inside
/// the circle of radius `√coc2`, and that is a property of the shape, not of any
/// picture it is run over.
pub fn dof_tap_inside(dx: f32, dy: f32, coc2: f32, p: &DofParams) -> bool {
    in_aperture_shape(dx, dy, coc2, p).is_some()
}

/// One tap's radial weight (Concentration). Written multiplicatively in `coc2`
/// so there is no division and no guard at `coc = 0`; the weights are only ever
/// used as a ratio, so the common `coc2` factor cancels in the mean.
///
/// 0 is the flat disc — but the caller branches around this entirely at 0
/// rather than trusting `w = coc2` to cancel exactly, because it does not: a
/// constant factor through a sum is not an IEEE identity.
fn tap_weight(r2: f32, coc2: f32, p: &DofParams) -> f32 {
    (coc2 + p.rim * (2.0 * r2 - coc2)).max(0.0)
}

/// The tap's deformed `r²` when it is inside the aperture, else `None`.
///
/// **Roundness 1 with no Deform is the plain circle, and takes the plain test.**
/// That is not an optimisation, it is the back-compatibility guarantee: the
/// polygon form multiplies both sides of the comparison by `apothem2`, and
/// scaling both sides of a floating-point comparison by the same positive
/// constant can change its answer on a boundary tap. The circle path is the
/// literal `dx² + dy² ≤ coc²` this effect has always used, so the default
/// aperture gathers exactly the taps it always gathered.
///
/// Below that, two things shape the region. **Roundness reaches below zero**:
/// the same test with a negative coefficient rewards distance from the centre,
/// so the edge midpoints pull in while the vertices stay exactly on the circle —
/// a star, with no new maths and no branch. **Deform** multiplies the tap offset
/// before the test, and its multipliers are always ≥ 1, so the aperture only
/// ever shrinks on one axis. Both matter for the same reason: the region stays
/// inscribed in the circle of confusion at every setting, so the `ceil(coc)`
/// scan box — and the effect's declared ROI — remain correct bounds.
fn in_aperture_shape(dx: f32, dy: f32, coc2: f32, p: &DofParams) -> Option<f32> {
    if p.roundness >= 1.0 && p.aspect_scale[0] == 1.0 && p.aspect_scale[1] == 1.0 {
        let r2 = dx * dx + dy * dy;
        return (r2 <= coc2).then_some(r2);
    }
    let ax = dx * p.aspect_scale[0];
    let ay = dy * p.aspect_scale[1];
    let r2 = ax * ax + ay * ay;
    let mut m = 0.0f32;
    for n in p.blade_normals.iter().take(p.blade_count as usize) {
        m = m.max(ax * n[0] + ay * n[1]);
    }
    let inside = (1.0 - p.roundness) * m * m + p.roundness * p.apothem2 * r2 <= p.apothem2 * coc2;
    inside.then_some(r2)
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

/// One resolved sprite flare (docs/08 §3.29, K-359) — the art-directed
/// sibling of the physically simulated §3.27.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteFlareParams {
    /// Where the light is, in raster pixels (px@comp, K-260).
    pub light: [f32; 2],
    /// Master gain on everything the effect draws; 0 is the neutral point.
    pub intensity: f32,
    /// Scene-linear RGB every element is multiplied by.
    pub tint: [f32; 3],
    /// The central glow's radius in raster pixels, and its gain.
    pub glow_size: f32,
    pub glow_intensity: f32,
    /// How many iris ghosts march along the axis, their spacing as a fraction
    /// of the light→centre distance, their base radius and their gain.
    pub ghosts: u32,
    pub ghost_spacing: f32,
    pub ghost_size: f32,
    pub ghost_intensity: f32,
    /// The anamorphic streak's half-length in raster pixels, its gain, and its
    /// angle in degrees (0 = horizontal).
    pub streak_length: f32,
    pub streak_intensity: f32,
    pub streak_angle_deg: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Most ghosts a sprite flare will draw — the loop bound both twins share.
pub const SPRITE_FLARE_MAX_GHOSTS: u32 = 16;

/// The light a sprite flare adds at one pixel, before tint and Intensity
/// (docs/08 §3.29, K-359). Shared by the CPU reference and mirrored op-for-op
/// in WGSL, so the two cannot drift.
///
/// Everything is placed from the **light's position**, never from the picture's
/// brightness: that is the whole difference from §3.27's Matte mode and the
/// reason this one does not flicker on footage. The elements march along the
/// line from the light through the frame's centre, which is what a real lens
/// does — the ghosts are reflections about the optical axis, so they land on
/// the far side of the middle as the light moves.
#[must_use]
pub fn sprite_flare_at(px: f32, py: f32, w: u32, h: u32, p: &SpriteFlareParams) -> f32 {
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let (lx, ly) = (p.light[0], p.light[1]);
    let mut acc = 0.0f32;

    // The central glow: a soft falloff on the light itself.
    if p.glow_intensity > 0.0 && p.glow_size > 0.0 {
        let d = ((px - lx).powi(2) + (py - ly).powi(2)).sqrt() / p.glow_size;
        acc += p.glow_intensity * (-d * d).exp();
    }

    // The ghosts, mirrored through the centre and shrinking with distance —
    // the far ones are the small tight discs, the near ones broad and faint.
    if p.ghost_intensity > 0.0 && p.ghost_size > 0.0 {
        let (ax, ay) = (lx - cx, ly - cy);
        let n = p.ghosts.min(SPRITE_FLARE_MAX_GHOSTS);
        for i in 1..=n {
            let t = -(i as f32) * p.ghost_spacing;
            let (gx, gy) = (cx + ax * t, cy + ay * t);
            // A ghost further from the axis' centre is a larger, softer disc.
            let radius = p.ghost_size * (0.35 + 0.65 * t.abs());
            if radius <= 0.0 {
                continue;
            }
            let d = ((px - gx).powi(2) + (py - gy).powi(2)).sqrt() / radius;
            // A soft-edged disc: flat in the middle, falling to nothing at the
            // rim, which is what an out-of-focus iris looks like.
            let disc = (1.0 - d * d).max(0.0);
            // Alternate ghosts fall off harder, so the train reads as a train
            // rather than as one smear.
            let shaped = if i % 2 == 0 { disc * disc } else { disc };
            acc += p.ghost_intensity * shaped / (i as f32);
        }
    }

    // The anamorphic streak: a long thin glow through the light.
    if p.streak_intensity > 0.0 && p.streak_length > 0.0 {
        let a = p.streak_angle_deg.to_radians();
        let (s, c) = (a.sin(), a.cos());
        let (dx, dy) = (px - lx, py - ly);
        // Into the streak's own frame, then squashed: long along it, tight
        // across it.
        let along = (dx * c + dy * s) / p.streak_length;
        let across = (-dx * s + dy * c) / (p.streak_length * 0.03).max(1e-3);
        let d2 = along * along + across * across;
        acc += p.streak_intensity * (-d2).exp();
    }

    acc.max(0.0)
}

/// **Sprite flare** (docs/08 §3.29, K-359): the art-directed flare, drawn from
/// a light POSITION rather than from the picture's bright pixels.
pub fn sprite_flare(rgba: &mut [f32], w: u32, h: u32, p: &SpriteFlareParams) {
    if p.intensity <= 0.0 || p.mix <= 0.0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let e = sprite_flare_at(x as f32 + 0.5, y as f32 + 0.5, w, h, p) * p.intensity;
            if e <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let base = rgba[i + c];
                // Additive, like every other light in the engine: a flare is
                // light arriving at the sensor, not a grade.
                let lit = base + e * p.tint[c];
                rgba[i + c] = base * (1.0 - p.mix) + lit * p.mix;
            }
        }
    }
}

/// **Light wrap** (docs/08 §3.28, K-358): spill the background's light around
/// the edge of a foreground so a cut-out sits *in* the plate instead of on it.
///
/// # In plain terms
///
/// A keyed subject looks pasted on because in a real camera the light behind
/// it would spill round its edges — off the hair, along the shoulders. This
/// takes the background, blurs it, and adds that blur back only where the
/// foreground's own edge is: strongest right at the outline, fading inwards
/// over `width` pixels, and nothing at all out where the foreground is
/// transparent or deep inside where it is solid.
///
/// The edge is found from the foreground's own alpha, which is why the effect
/// needs no mask of its own: blur the alpha, and `blurred × (1 − alpha)` is
/// large exactly in the band just inside the outline. That band is the wrap.
///
/// `bg` is the background layer, premultiplied and the same size as `rgba`.
/// The wrap is **screened** on rather than added, so a bright background
/// cannot blow the edge past white; `intensity` scales it and `mix` fades the
/// whole effect. Both zero are the bit-exact identity, and so is a zero
/// `width` — there is no band to fill.
pub fn light_wrap(
    rgba: &mut [f32],
    bg: &[f32],
    w: u32,
    h: u32,
    width_px: f32,
    intensity: f32,
    mix: f32,
) {
    if width_px <= 0.0 || intensity <= 0.0 || mix <= 0.0 {
        return;
    }
    let n = (w as usize) * (h as usize) * 4;
    if rgba.len() < n || bg.len() < n {
        return;
    }
    // The background, softened over the wrap's width: this is the light that
    // spills. Edge policy 1 (repeat the edge pixel), so a subject touching the
    // frame border wraps with the plate rather than with black.
    let mut spill = bg[..n].to_vec();
    blur_gaussian(&mut spill, w, h, width_px, 1, 1.0);
    // The foreground softened by the same gaussian. Only its ALPHA is wanted —
    // blurring the matte and taking what lies under the original alpha is the
    // edge band — and blurring the whole texture gets it for free, which is
    // also what lets the GPU twin reuse the ordinary blur kernel twice
    // instead of needing one of its own.
    let mut soft = rgba[..n].to_vec();
    blur_gaussian(&mut soft, w, h, width_px, 1, 1.0);

    for i in (0..n).step_by(4) {
        let a = rgba[i + 3];
        // **The band is where the matte has been softened AWAY from solid.**
        // Blurring a solid subject's alpha leaves 1 deep inside, about a half
        // right at the outline, and less beyond it — so `1 − soft.a` is zero
        // in the middle and rises toward the edge, which is the wrap. The
        // doubling brings it to full strength at the outline rather than a
        // half. `a` gates it, so the wrap never paints on transparent pixels,
        // which would grow a halo *outside* the matte — the classic way to
        // get this effect wrong.
        let band = ((1.0 - soft[i + 3]) * 2.0).clamp(0.0, 1.0) * a;
        let k = band * intensity;
        if k <= 0.0 {
            continue;
        }
        for c in 0..3 {
            let base = rgba[i + c];
            // Screen: 1 − (1 − base)(1 − spill·k), which cannot exceed the
            // brighter of the two and so cannot blow the edge out.
            let s = (spill[i + c] * k).max(0.0);
            let screened = 1.0 - (1.0 - base) * (1.0 - s);
            rgba[i + c] = base * (1.0 - mix) + screened * mix;
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

/// Scanlines (docs/08 §3.12, split out by K-107; single Intensity since
/// FX-13/K-147): standalone periodic darken, the scanline section of the old
/// combined Glitch effect. No hash, no block resample — reads the input pixel
/// directly (pointwise, [`Roi::Exact`](super::Roi::Exact)), darkens the dark
/// lines by a periodic band in raster Y (plus the precomputed roll offset),
/// alternating which half of the period darkens on odd periods when Interlace
/// is on. `intensity` is the single dial (0..1 = how dark the dark lines get,
/// 1 = black); the bright half is untouched. Intensity 0 is the bit-exact
/// passthrough, pinned by the early return below — the same neutral shape
/// [`block_glitch`] uses.
#[allow(clippy::too_many_arguments)]
pub fn scanlines(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    intensity: f32,
    period_px: f32,
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
            // The dark half's base is black (band 0), so eff_mult is
            // 1 − intensity there and 1 on the bright half.
            let band = if bright { 1.0 } else { 0.0 };
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
