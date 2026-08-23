use super::noise::{fractal, hash01, value3, FractalField};
use super::{MatteKeyParams, MbQuality, MbView, MAX_BLADES};

/// Apply a whole resolved stack to an RGBA f32 image (premultiplied, linear
/// light), in place — the CPU-degradation rung (K-019) and the parity oracle's
/// entry point.
///
/// Every effect is dispatched through its own
/// [`EffectDef::apply_cpu`](super::EffectDef::apply_cpu) with the bag its
/// parameters live in (docs/impl/effect-registry.md §3, step 4) — the whole
/// stack, and not one op, is the unit here because an op's numbers are a
/// borrowed run of the stack's own arena.
///
/// Several of the effects are passthroughs even here, and deliberately: Light
/// wrap's background, Depth of field's depth pass, Echo's neighbour frames,
/// Motion blur's and Datamosh's flow field and the LUT's cube are whole pictures
/// that arrive beside the op as aux slots (K-387), and no single-buffer
/// dispatcher carries one. The Lens flare is the same shape for a different
/// reason (K-256, the K-114 LUT precedent): it owns a render pass over baked
/// tables, and neither reaches a single `&mut [f32]`. Each keeps
/// `EffectDef::apply_cpu`'s identity default, and its §1.6 oracle runs against
/// its `cpu::` reference directly from the lumit-gpu test, which can upload the
/// second picture.
pub fn apply_stack(rgba: &mut [f32], w: u32, h: u32, ops: &super::ResolvedStack) {
    for fx in ops.iter() {
        match blend_seam(fx.params) {
            // The Blend row (K-425), the CPU half of the seam: the kernel runs
            // at Mix 100 into a copy, and the blend and the Mix are applied
            // here, once, exactly as `run_ops` does on the GPU.
            Some((mode, mix, entries)) => {
                let input = rgba.to_vec();
                fx.def.apply_cpu(rgba, w, h, super::Params::new(&entries));
                blend_mix(rgba, &input, mode, mix);
            }
            None => fx.def.apply_cpu(rgba, w, h, fx.params),
        }
    }
}

/// What [`blend_seam`] hands back when an op blends: the mode, the op's own
/// Mix as 0..1, and its parameters with Mix forced to 100.
pub type BlendSeam = (u32, f32, Vec<(super::ParamId, super::Value)>);

/// What the seam does about an op's Blend row (K-425), read once for both
/// render paths: `None` when the row is Normal (or absent), in which case the
/// kernel runs exactly as it always has; otherwise the mode, the op's own Mix
/// as a 0..1 fraction, and the op's parameters with Mix forced to 100, which
/// is what the kernel is run with so that its output is the *unmixed* effect
/// the blend wants.
///
/// The Mix lives inside every kernel (docs/08 §1.5), and a blend of the input
/// with an already-mixed output would apply the Mix twice — once as the
/// kernel's lerp and again as the seam's. Forcing it to 100 for the kernel and
/// lerping once here, after the blend, is the one order in which "Blend, then
/// Mix" means what it says. Normal takes no pass at all, so an effect whose
/// Blend row is unset renders byte for byte what it did (K-258).
#[must_use]
pub fn blend_seam(p: super::Params<'_>) -> Option<BlendSeam> {
    let mode = p.choice(super::BLEND_ID, 0);
    if mode == 0 {
        return None;
    }
    let mix = (p.float(super::MIX_ID, 100.0) / 100.0).clamp(0.0, 1.0);
    let entries = p
        .iter()
        .map(|(id, v)| {
            if id == super::MIX_ID {
                (id, super::Value::Float(100.0))
            } else {
                (id, v)
            }
        })
        .collect();
    Some((mode, mix, entries))
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
///
/// # The Matte gates the seed (K-395, docs/08 §2.6)
///
/// Glow is one of the effects that claim the matte inside their own maths, and
/// this is what it does with it: the source is multiplied by the matte's luma
/// **before** the bright pass, so only what the matte lights is allowed to
/// bloom. The halo then spreads from those pixels normally — out across dark
/// matte, past the matte's edge, over the parts of the picture the matte
/// excluded.
///
/// That last sentence is the whole difference from the generic dissolve. Fading
/// a finished glow by a matte clips the halo to the matte's shape, so a glow
/// "on the sign only" stops dead at the sign's outline, which is not how light
/// behaves. Gating the seed lets the sign light the wall beside it.
///
/// An empty `matte` multiplies nothing and reads no pixels, leaving the
/// arithmetic exactly as it was before K-395 (K-258). The matte arrives
/// already prepared — channel picked and Invert applied once, by
/// [`matte_prepare`] at the seam (K-425) — so this reads its luma and nothing
/// else.
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
    matte: &[f32],
) {
    if intensity == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let mut halo = vec![0.0f32; rgba.len()];
    if matte.is_empty() {
        for (dst, src) in halo.iter_mut().zip(original.iter()) {
            *dst = super::glow_bright(*src, threshold, knee);
        }
    } else {
        for i in (0..original.len()).step_by(4) {
            // One k for all four channels, from the matte pixel under this one.
            // A matte shorter than the picture seeds those pixels in full —
            // degrade, never fault (14-ENGINEERING-RULES §4).
            let k = matte_strength(matte, i);
            for c in 0..4 {
                halo[i + c] = super::glow_bright(original[i + c] * k, threshold, knee);
            }
        }
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
    transform_matted(
        rgba,
        w,
        h,
        anchor,
        position,
        scale,
        rotation_deg,
        edge,
        opacity,
        mix,
        &[],
    );
}

/// [`transform`] driven by a matte (K-395, docs/08 §2.6) — the Shake's claim
/// (K-427): each pixel's matte strength `k` scales **the displacement the
/// wobble gives that pixel** toward none, `q = p·(1 − k) + q·k` in
/// [`matte_toward`]'s form, read at the destination pixel. For the offset that
/// is exactly Amplitude·k; the rotation and the zoom scale by the offset they
/// make at that pixel, so a grey matte turns a frame-wide shove into a warp
/// and a black one leaves the pixel where it was. The Transform effect never
/// passes a matte (it keeps the strength dissolve). An empty matte is the
/// unmatted path to the byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn transform_matted(
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
    matte: &[f32],
) {
    let original = rgba.to_vec();
    // A collapsed (zero-scale) image is invisible: opacity 0, and the
    // sample point no longer matters (super::transform_op's rule).
    let (m, o, opacity) = super::transform_op(anchor, position, scale, rotation_deg, opacity);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = matte_strength(matte, i);
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let qx = matte_toward(m[0] * px + m[1] * py + o[0], px, k);
            let qy = matte_toward(m[2] * px + m[3] * py + o[1], py, k);
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
    transform_average_matted(rgba, w, h, ops, edge, mix, &[]);
}

/// [`transform_average`] driven by a matte (K-395, K-427): every sub-frame
/// tap's displacement is scaled toward none by the destination pixel's matte
/// strength, exactly as [`transform_matted`] scales the single tap. An empty
/// matte is the unmatted path to the byte (K-258).
pub fn transform_average_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    ops: &[([f32; 4], [f32; 2])],
    edge: u32,
    mix: f32,
    matte: &[f32],
) {
    if ops.is_empty() {
        return;
    }
    let original = rgba.to_vec();
    let n = ops.len() as f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = matte_strength(matte, i);
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let mut acc = [0.0f32; 4];
            for (m, o) in ops {
                let qx = matte_toward(m[0] * px + m[1] * py + o[0], px, k);
                let qy = matte_toward(m[2] * px + m[3] * py + o[1], py, k);
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
    colour_balance_matted(rgba, lift, gamma, gain, mix, &[]);
}

/// [`colour_balance`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength pulls its **Lift toward 0, Gamma toward 1 and Gain toward 1**
/// ([`matte_toward`]) before the grade runs, so a grey matte is a gentler grade
/// and not a full grade faded back. An empty matte is the unmatted path to the
/// byte (K-258).
pub fn colour_balance_matted(
    rgba: &mut [f32],
    lift: [f32; 3],
    gamma: [f32; 3],
    gain: [f32; 3],
    mix: f32,
    matte: &[f32],
) {
    if lift == [0.0; 3] && gamma == [1.0; 3] && gain == [1.0; 3] {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let k = matte_strength(matte, i * 4);
        let a = px[3];
        let u = unpremult(px);
        let mut v = [0.0f32; 3];
        for c in 0..3 {
            let (lf, gm, gn) = (
                matte_toward(lift[c], 0.0, k),
                matte_toward(gamma[c], 1.0, k),
                matte_toward(gain[c], 1.0, k),
            );
            let mut x = (u[c] * gn + lf).max(0.0);
            if gm != 1.0 {
                x = x.powf(1.0 / gm);
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
    saturate_matted(rgba, saturation, mix, &[]);
}

/// [`saturate`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength pulls its **Saturation toward 1** (the 100 % neutral,
/// [`matte_toward`]) before the scale runs. An empty matte is the unmatted
/// path to the byte (K-258).
pub fn saturate_matted(rgba: &mut [f32], saturation: f32, mix: f32, matte: &[f32]) {
    if saturation == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let sat = matte_toward(saturation, 1.0, matte_strength(matte, i * 4));
        let a = px[3];
        let u = unpremult(px);
        let luma = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
        for c in 0..3 {
            let v = (luma + (u[c] - luma) * sat).max(0.0);
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
    vibrance_matted(rgba, amount, mix, &[]);
}

/// [`vibrance`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength scales its **Amount** before the boost is worked out. An empty
/// matte is the unmatted path to the byte (K-258).
pub fn vibrance_matted(rgba: &mut [f32], amount: f32, mix: f32, matte: &[f32]) {
    if amount == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let amount = amount * matte_strength(matte, i * 4);
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
    exposure_matted(rgba, factor, 0.0, mix, &[]);
}

/// [`exposure`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength `k` scales its **Stops toward 0**, so the gain there is
/// `2^(stops·k)` — a half-grey matte on +2 stops is +1 stop, not a fade
/// between +2 and none. `factor` stays the host's `2^stops` and is what an
/// unmatted pixel multiplies by; `stops` is only read under a matte, which is
/// what keeps the empty-matte path byte-identical (K-258; `exp2` would not
/// promise to reproduce the host's `f64` power).
pub fn exposure_matted(rgba: &mut [f32], factor: f32, stops: f32, mix: f32, matte: &[f32]) {
    if factor == 1.0 {
        return;
    }
    let matted = !matte.is_empty();
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let f = if matted {
            (stops * matte_strength(matte, i * 4)).exp2()
        } else {
            factor
        };
        for ch in &mut px[..3] {
            let scaled = *ch * f;
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
    hue_shift_matted(rgba, m, 0.0, true, mix, &[]);
}

/// The hue-rotation matrix for `rad` radians in `f32`, the per-pixel twin of
/// [`super::hue_matrix`] / [`super::hue_matrix_rgb`] (same coefficients, same
/// order of operations) that the matted [`hue_shift_matted`] builds under a
/// matte and `fx_hue.wgsl` builds op-for-op. The host keeps computing the
/// unmatted matrix in `f64`; this only ever runs where a matte has changed the
/// angle, so the unmatted path never depends on it.
fn hue_matrix_px(rad: f32, preserve: bool) -> [f32; 9] {
    let (s, c) = rad.sin_cos();
    if preserve {
        let (lr, lg, lb) = (LUMA[0], LUMA[1], LUMA[2]);
        [
            lr + c * (1.0 - lr) - s * lr,
            lg - c * lg - s * lg,
            lb - c * lb + s * (1.0 - lb),
            lr - c * lr + s * 0.143,
            lg + c * (1.0 - lg) + s * 0.140,
            lb - c * lb - s * 0.283,
            lr - c * lr - s * (1.0 - lr),
            lg - c * lg + s * lg,
            lb + c * (1.0 - lb) + s * lb,
        ]
    } else {
        let a = (1.0 - c) / 3.0;
        let b = s / 3.0f32.sqrt();
        [
            c + a,
            a - b,
            a + b,
            a + b,
            c + a,
            a - b,
            a - b,
            a + b,
            c + a,
        ]
    }
}

/// [`hue_shift`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength `k` scales its **Angle toward 0**, and the rotation matrix for
/// `angle·k` is built per pixel ([`hue_matrix_px`]) — a half-grey matte on a
/// 90° shift turns the hue 45°, where a fade would mix the 90°-turned colour
/// with the original and desaturate it. `m` stays the host's matrix and is what
/// an unmatted pixel multiplies by; `angle_rad` and `preserve` are only read
/// under a matte, which keeps the empty-matte path byte-identical (K-258).
pub fn hue_shift_matted(
    rgba: &mut [f32],
    m: [f32; 9],
    angle_rad: f32,
    preserve: bool,
    mix: f32,
    matte: &[f32],
) {
    if m == [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
        return;
    }
    let matted = !matte.is_empty();
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let m = if matted {
            hue_matrix_px(angle_rad * matte_strength(matte, i * 4), preserve)
        } else {
            m
        };
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
    gamma_matted(rgba, gamma, mix, &[]);
}

/// [`gamma`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength `k` pulls its **Gamma toward 1** ([`matte_toward`]) before the
/// curve runs, so a half-grey matte on Gamma 2 gives `pow(x, 1/1.5)` — a
/// genuinely gentler curve — and not `lerp(x, pow(x, 1/2), ½)`. An empty matte
/// is the unmatted path to the byte (K-258).
pub fn gamma_matted(rgba: &mut [f32], gamma: f32, mix: f32, matte: &[f32]) {
    if gamma == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let inv = 1.0 / matte_toward(gamma, 1.0, matte_strength(matte, i * 4));
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
    temperature_matted(rgba, gain_r, gain_b, 0.0, mix, &[]);
}

/// The two channel gains for a Temperature of `t` (the slider ÷ 100, clamped
/// to ±2): `(1 ± 0.75·t)` floored at 0 — `Temperature::gains` in `f32`, which
/// is what [`temperature_matted`] and `fx_temperature.wgsl` evaluate per pixel
/// under a matte.
#[must_use]
pub fn temperature_gains(t: f32) -> (f32, f32) {
    ((1.0 + 0.75 * t).max(0.0), (1.0 - 0.75 * t).max(0.0))
}

/// [`temperature`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength `k` scales its **Temperature toward 0**, and the two gains
/// are rebuilt from `t·k` ([`temperature_gains`]) — not lerped, because the
/// blue gain floors at 0 past ±133 and a lerp of a floored gain is not the
/// gain of a smaller temperature. `gain_r`/`gain_b` stay the host's and are
/// what an unmatted pixel multiplies by; `t` is only read under a matte, which
/// keeps the empty-matte path byte-identical (K-258).
pub fn temperature_matted(
    rgba: &mut [f32],
    gain_r: f32,
    gain_b: f32,
    t: f32,
    mix: f32,
    matte: &[f32],
) {
    if gain_r == 1.0 && gain_b == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let matted = !matte.is_empty();
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let (gain_r, gain_b) = if matted {
            temperature_gains(t * matte_strength(matte, i * 4))
        } else {
            (gain_r, gain_b)
        };
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

/// Entries in one channel's baked tone curve (K-412): 257 samples of the
/// spline at `i / 256`, so the last entry is input 1 exactly and the step is
/// a power of two — which is what keeps the identity curve bit-exact through
/// the lookup.
pub const CURVE_TABLE: usize = 257;

/// Curves' five baked channel tables and its mix — everything both render
/// paths read (docs/08 §3.30, K-412).
///
/// **The spline is fitted once, host-side**
/// ([`crate::fx::effects::curves::Curves::packed`]), never per pixel and
/// never twice: the CPU reference and the WGSL kernel are handed the
/// identical numbers, so §1.6 only has to check the *lookup*. That is
/// Lightning's discipline (§3.74) applied to a shape that is the same for
/// every pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveTables {
    /// `[channel][entry]`: channel 0 Master, 1..3 R/G/B, 4 Alpha.
    pub t: [[f32; CURVE_TABLE]; 5],
    /// Every channel is the identity diagonal, so the effect is the bit-exact
    /// passthrough. Decided host-side because the kernel cannot afford to
    /// compare 1285 numbers a pixel.
    pub neutral: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The identity table: `t[i] == i / 256`, what a fresh channel bakes to.
#[must_use]
pub fn curve_identity_table() -> [f32; CURVE_TABLE] {
    let mut t = [0.0f32; CURVE_TABLE];
    for (i, slot) in t.iter_mut().enumerate() {
        *slot = i as f32 / (CURVE_TABLE - 1) as f32;
    }
    t
}

/// Bake one channel's control points into its 257-entry table (K-412).
///
/// # In plain terms
///
/// The user drags a few points; this draws the smooth line through them and
/// writes down where that line is at 257 evenly spaced inputs. Everything
/// downstream reads the written-down numbers, so nothing has to re-draw the
/// line for every pixel of every frame.
///
/// **The spline is a clamped cubic** — the family Photoshop's curve comes
/// from, and so the one every editor's hand already knows. Between two points
/// it is a cubic; at every interior point the second derivative matches on
/// both sides (that is what makes it *the* cubic spline rather than a
/// piecewise guess); and at the two ends the slope is **clamped to the end
/// secant**, the straight line to the neighbouring point. That end condition
/// is what makes a two-point curve exactly its own straight line, which the
/// identity diagonal depends on.
///
/// **The clamping rule.** Points live in the unit square, and so does the
/// baked line: every sample is clamped into `0..=1`. A cubic through monotone
/// points can bulge past its own end points, and a tone curve that climbed
/// above the white the user placed would ring a bright halo into a roll-off.
/// Clipping *inputs* is a different matter and does not happen — [`curve_at`]
/// extrapolates along the table's own end segments, so a scene-linear value
/// above 1 is carried on rather than flattened (§2.1).
///
/// Evaluated in `f64` in one fixed order, so the same points always bake to
/// the same bytes on every machine (14-ENGINEERING-RULES §5).
#[must_use]
pub fn curve_table(points: &crate::fx::CurvePoints) -> [f32; CURVE_TABLE] {
    const MAX: usize = crate::fx::CURVE_MAX_POINTS;
    let p = points.points();
    let n = p.len();
    let mut x = [0.0f64; MAX];
    let mut y = [0.0f64; MAX];
    for i in 0..n {
        x[i] = f64::from(p[i][0]);
        y[i] = f64::from(p[i][1]);
    }

    // Interval widths and secant slopes.
    let mut h = [0.0f64; MAX];
    let mut d = [0.0f64; MAX];
    for i in 0..n - 1 {
        h[i] = x[i + 1] - x[i];
        // `sanitised` guarantees a strictly increasing x, so `h` is never 0;
        // the guard is here because a divide by zero would be a NaN table
        // rather than a crash, and a NaN table is a black frame nobody can
        // explain.
        d[i] = if h[i] > 0.0 {
            (y[i + 1] - y[i]) / h[i]
        } else {
            0.0
        };
    }

    // Slopes at the points. The two ends are clamped to their secants; the
    // interior ones come from the C2 condition, a tridiagonal system solved
    // by the Thomas algorithm (n is at most 16, so this is a handful of
    // operations).
    let mut m = [0.0f64; MAX];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    if n > 2 {
        let rows = n - 2;
        let mut lower = [0.0f64; MAX];
        let mut diag = [0.0f64; MAX];
        let mut upper = [0.0f64; MAX];
        let mut rhs = [0.0f64; MAX];
        for r in 0..rows {
            let i = r + 1;
            lower[r] = h[i];
            diag[r] = 2.0 * (h[i - 1] + h[i]);
            upper[r] = h[i - 1];
            rhs[r] = 3.0 * (h[i] * d[i - 1] + h[i - 1] * d[i]);
        }
        // The known end slopes move to the right-hand side.
        rhs[0] -= lower[0] * m[0];
        lower[0] = 0.0;
        rhs[rows - 1] -= upper[rows - 1] * m[n - 1];
        upper[rows - 1] = 0.0;

        for r in 1..rows {
            let w = if diag[r - 1] != 0.0 {
                lower[r] / diag[r - 1]
            } else {
                0.0
            };
            diag[r] -= w * upper[r - 1];
            rhs[r] -= w * rhs[r - 1];
        }
        for r in (0..rows).rev() {
            let above = if r + 1 < rows { m[r + 2] } else { 0.0 };
            let num = rhs[r] - upper[r] * above;
            m[r + 1] = if diag[r] != 0.0 { num / diag[r] } else { 0.0 };
        }
    }

    // Sample the Hermite pieces at i / 256, walking the intervals forward so
    // the search is not repeated per sample.
    let mut table = [0.0f32; CURVE_TABLE];
    let mut seg = 0usize;
    for (i, slot) in table.iter_mut().enumerate() {
        let xi = i as f64 / (CURVE_TABLE - 1) as f64;
        while seg + 2 < n && xi >= x[seg + 1] {
            seg += 1;
        }
        let v = if xi <= x[0] {
            // Before the first point and after the last, the clamped end
            // slope carries on straight — the same line the first and last
            // cubic pieces leave along, so there is no kink at the join.
            y[0] + m[0] * (xi - x[0])
        } else if xi >= x[n - 1] {
            y[n - 1] + m[n - 1] * (xi - x[n - 1])
        } else {
            let hs = h[seg];
            let t = if hs > 0.0 { (xi - x[seg]) / hs } else { 0.0 };
            let t2 = t * t;
            let t3 = t2 * t;
            y[seg] * (2.0 * t3 - 3.0 * t2 + 1.0)
                + m[seg] * hs * (t3 - 2.0 * t2 + t)
                + y[seg + 1] * (-2.0 * t3 + 3.0 * t2)
                + m[seg + 1] * hs * (t3 - t2)
        };
        #[allow(clippy::cast_possible_truncation)]
        {
            *slot = v.clamp(0.0, 1.0) as f32;
        }
    }
    table
}

/// One channel of a baked tone curve at `x` (K-412): a lookup into the
/// 257-entry table with linear interpolation between entries.
///
/// The index is clamped but the fraction is not, so an input **outside 0..1
/// extrapolates along the table's first or last segment** rather than
/// clipping — which is how a scene-linear value above 1 keeps being curved
/// honestly (§2.1) and a slightly negative one stays continuous. One
/// expression, no branches, and the WGSL twin is the same four lines.
///
/// Bit-exact on the identity table: `t[i] == i / 256`, every step is a power
/// of two, and the arithmetic returns `x` unchanged.
#[must_use]
pub fn curve_at(x: f32, t: &[f32; CURVE_TABLE]) -> f32 {
    let last = (CURVE_TABLE - 1) as f32;
    let s = x * last;
    let fi = s.clamp(0.0, last - 1.0).floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = (fi as usize).min(CURVE_TABLE - 2);
    let f = s - fi;
    let a = t[i];
    a + (t[i + 1] - a) * f
}

/// Curves (docs/08 §3.30, K-412): a tone curve per channel, baked to a table
/// host-side and read here as a lookup. On unpremultiplied colour (§2.2),
/// re-premultiplied on the way out — exactly Contrast's and Gamma's
/// premultiply handling, a tone curve being non-linear.
///
/// The **per-channel curves run first, then Master** (Photoshop's and AE's
/// order, so an imported curve set lands the same way round). **Alpha is its
/// own channel** and Master does not touch it, as in After Effects; the
/// graded colour is re-premultiplied by the *graded* alpha, so a curve that
/// moves coverage moves the picture with it rather than leaving a doubled
/// matte. Identity curves on all five channels short-circuit the whole effect
/// (bit-exact identity — a short-circuit, not a reliance on the table
/// reproducing `y = x`; the WGSL twin matches). Continuous everywhere, so it
/// is safe under the §1.6 fp16 ULP oracle.
pub fn curves(rgba: &mut [f32], p: &CurveTables) {
    if p.neutral {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let graded_a = curve_at(a, &p.t[4]);
        for c in 0..3 {
            let v = curve_at(curve_at(u[c], &p.t[c + 1]), &p.t[0]);
            let graded = v * graded_a;
            px[c] = px[c] * (1.0 - p.mix) + graded * p.mix;
        }
        px[3] = a * (1.0 - p.mix) + graded_a * p.mix;
    }
}

/// One channel of the Levels map (docs/08 §3.31). `r` is indexed
/// `[row][channel]`: input black, the reciprocal input span, the reciprocal
/// gamma, output black, the output span — every reciprocal computed host-side
/// so neither path divides per pixel.
#[must_use]
pub fn level_at(x: f32, r: &[[f32; 4]; 5], c: usize) -> f32 {
    // Clamped at zero before the power exactly as §3.19 clamps: a power of a
    // negative base is undefined, and the clamp must be byte-identical on both
    // paths. There is deliberately no clamp above: a value past the input
    // white travels on rather than clipping (§2.1, the one divergence from
    // AE's 0..1 Levels).
    let mut n = ((x - r[0][c]) * r[1][c]).max(0.0);
    if r[2][c] != 1.0 {
        n = n.powf(r[2][c]);
    }
    r[3][c] + r[4][c] * n
}

/// Whether a Levels row set is the identity on all four channels.
fn levels_neutral(r: &[[f32; 4]; 5]) -> bool {
    r[0] == [0.0; 4] && r[1] == [1.0; 4] && r[2] == [1.0; 4] && r[3] == [0.0; 4] && r[4] == [1.0; 4]
}

/// Levels (docs/08 §3.31): input black/white, gamma and output black/white per
/// channel, on unpremultiplied colour (§2.2), re-premultiplied on the way out.
/// Per-channel first, then Master, matching Curves. Fully neutral rows
/// short-circuit the whole effect (bit-exact identity; the WGSL twin matches).
/// Alpha is untouched.
pub fn levels(rgba: &mut [f32], r: [[f32; 4]; 5], mix: f32) {
    if levels_neutral(&r) {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        for c in 0..3 {
            let v = level_at(level_at(u[c], &r, c + 1), &r, 0);
            let graded = v * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// Brightness (docs/08 §3.32, AE's Brightness & Contrast): one affine grade
/// `(u + b − pivot)·k + pivot` per RGB channel about the same mid-grey pivot
/// Contrast uses, on unpremultiplied colour (§2.2), re-premultiplied on the way
/// out — affine, so it does not commute with premultiplied alpha. `b` and `k`
/// are computed host-side (`Brightness ÷ 100`, `1 + Contrast ÷ 100`) so both
/// paths multiply by identical numbers. The neutral pair `(0, 1)`
/// short-circuits the whole effect (bit-exact identity; the WGSL twin matches).
/// Purely continuous — no round/clamp/quantize — and highlights are never
/// clipped (§2.1). Alpha is untouched.
pub fn brightness(rgba: &mut [f32], b: f32, k: f32, mix: f32) {
    brightness_matted(rgba, b, k, mix, &[]);
}

/// [`brightness`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength pulls its **Brightness toward 0 and Contrast toward 1**
/// ([`matte_toward`]) before the grade runs. An empty matte is the unmatted
/// path to the byte (K-258).
pub fn brightness_matted(rgba: &mut [f32], b: f32, k: f32, mix: f32, matte: &[f32]) {
    if b == 0.0 && k == 1.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let m = matte_strength(matte, i * 4);
        let (b, k) = (matte_toward(b, 0.0, m), matte_toward(k, 1.0, m));
        let a = px[3];
        let u = unpremult(px);
        for c in 0..3 {
            let v = (u[c] + b - CONTRAST_PIVOT) * k + CONTRAST_PIVOT;
            let graded = v * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// The HSV hue of an unpremultiplied colour, in degrees 0..360, given its
/// value (the channel maximum) and chroma (maximum − minimum). A neutral
/// colour has no hue and answers 0; the range weighting scales by saturation,
/// so that 0 costs a grey nothing (docs/08 §3.33).
#[must_use]
pub fn hsv_hue(u: [f32; 3], v: f32, c: f32) -> f32 {
    if c <= 0.0 {
        return 0.0;
    }
    let sixth = if v == u[0] {
        (u[1] - u[2]) / c
    } else if v == u[1] {
        (u[2] - u[0]) / c + 2.0
    } else {
        (u[0] - u[1]) / c + 4.0
    };
    let h = sixth * 60.0;
    if h < 0.0 {
        h + 360.0
    } else {
        h
    }
}

/// HSV back to RGB, with **V unbounded above** so scene-linear headroom
/// survives the round trip (docs/08 §3.33). `h` is degrees 0..360, `s` is
/// 0..1.
#[must_use]
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let hh = h / 60.0;
    let sector = hh.floor();
    let f = hh - sector;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    // Wrapped, not clamped. `h` arrives folded into 0..360, but the fold can
    // land on exactly 360 when the turn rounds — and a clamp would answer that
    // with sector 5 (magenta) where the colour is red, which is a hue jump at
    // one exact value. `rem_euclid` sends 6 back to 0, where the Hermite-free
    // arithmetic below reproduces red exactly. The WGSL twin spells the same
    // wrap.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    match (sector as i32).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Hue and saturation (docs/08 §3.33): a master adjustment plus six colour
/// ranges, each hue/saturation/lightness, through HSV on unpremultiplied
/// colour (§2.2), re-premultiplied on the way out.
///
/// `bands` is indexed `[band][hue, saturation %, lightness %, unused]`, band 0
/// being Master and 1..6 the ranges centred on red, yellow, green, cyan, blue
/// and magenta. Each range's weight is a hat function 120° wide centred every
/// 60°, so the six sum to exactly 1 for any hue and there is no boundary to
/// cross; the weights are then scaled by the pixel's own saturation, so a grey
/// (whose hue reads 0°, which is red) takes the Master adjustment alone. All
/// twenty-one adjustments at zero short-circuits the whole effect (bit-exact
/// identity; the WGSL twin matches). Alpha is untouched.
pub fn hue_saturation(rgba: &mut [f32], bands: [[f32; 4]; 7], mix: f32) {
    hue_saturation_matted(rgba, bands, mix, &[]);
}

/// [`hue_saturation`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales **every range's Hue, Saturation and Lightness toward
/// 0**. The scale is applied to the pixel's summed adjustment, which is the
/// same number as scaling all twenty-one controls first (the sum is linear in
/// them). An empty matte is the unmatted path to the byte (K-258).
pub fn hue_saturation_matted(rgba: &mut [f32], bands: [[f32; 4]; 7], mix: f32, matte: &[f32]) {
    if bands
        .iter()
        .all(|b| b[0] == 0.0 && b[1] == 0.0 && b[2] == 0.0)
    {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let k = matte_strength(matte, i * 4);
        let a = px[3];
        let u = unpremult(px);
        let v = u[0].max(u[1]).max(u[2]);
        let mn = u[0].min(u[1]).min(u[2]);
        let chroma = v - mn;
        let s = if v > 0.0 {
            (chroma / v).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let h = hsv_hue(u, v, chroma);
        let (mut dh, mut ds, mut dl) = (bands[0][0], bands[0][1], bands[0][2]);
        for (i, band) in bands.iter().enumerate().skip(1) {
            #[allow(clippy::cast_precision_loss)]
            let centre = (i - 1) as f32 * 60.0;
            let d = (h - centre).abs();
            let d = if d > 180.0 { 360.0 - d } else { d };
            let w = (1.0 - d / 60.0).max(0.0) * s;
            dh += w * band[0];
            ds += w * band[1];
            dl += w * band[2];
        }
        // The matte, after the sum and before the turn (the twin matches).
        let (dh, ds, dl) = (dh * k, ds * k, dl * k);
        // Folded into 0..360 by the subtract-the-floor form rather than
        // `rem_euclid`, because that is the form WGSL can spell op-for-op.
        let turned = h + dh;
        let h2 = turned - (turned / 360.0).floor() * 360.0;
        let s2 = (s * (1.0 + ds / 100.0)).clamp(0.0, 1.0);
        let v2 = (v * (1.0 + dl / 100.0)).max(0.0);
        let out = hsv_to_rgb(h2, s2, v2);
        for c in 0..3 {
            let graded = out[c] * a;
            px[c] = px[c] * (1.0 - mix) + graded * mix;
        }
    }
}

/// Fill (docs/08 §3.34): flood the layer's own coverage with one colour.
///
/// The source colour is never read — `colour · a` *is* the premultiplied form of
/// "this colour at this coverage", so the effect works directly on premultiplied
/// values (§2.2) with no round trip. Alpha is untouched, and the colour's own
/// alpha lane is ignored as it is on every colour parameter. There is no neutral
/// short-circuit: a Fill that changed nothing would be a Fill nobody applied.
/// Mix 0 is the bit-exact identity (the WGSL twin matches).
pub fn fill(rgba: &mut [f32], colour: [f32; 3], mix: f32) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        for c in 0..3 {
            let filled = colour[c] * a;
            px[c] = px[c] * (1.0 - mix) + filled * mix;
        }
    }
}

/// One resolved Gradient (docs/08 §3.35), reduced to what both paths read.
/// Every reciprocal is computed host-side (`Gradient::packed`) and floored
/// there, so a zero-length axis collapses the ramp to one flat colour instead of
/// dividing by zero (docs/14 §4) and neither path divides per pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientParams {
    /// Radial rather than linear.
    pub radial: bool,
    /// The start point in raster pixels (the §2.3 preview factor already applied).
    pub start: [f32; 2],
    /// `end − start`, raster pixels.
    pub axis: [f32; 2],
    /// `1 ÷ |axis|²` for the linear projection, floored.
    pub inv_len2: f32,
    /// `1 ÷ |axis|` for the radial distance, floored.
    pub inv_len: f32,
    /// Scene-linear start colour (alpha ignored: the ramp is opaque).
    pub c0: [f32; 3],
    /// Scene-linear end colour.
    pub c1: [f32; 3],
    /// Dither of `t`, 0..1 (Scatter ÷ 100).
    pub scatter: f32,
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Gradient (docs/08 §3.35): a linear or radial two-colour ramp with optional
/// scatter — a **generator**, so it replaces the frame edge to edge and writes
/// opaque alpha. Interpolation is in the working space (scene-linear, §2.1), so
/// the ramp is photometrically even and can drive another effect's matte. Mix 0
/// is the bit-exact identity (the WGSL twin matches).
pub fn gradient(rgba: &mut [f32], w: u32, h: u32, p: &GradientParams) {
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let dx = x as f32 + 0.5 - p.start[0];
            let dy = y as f32 + 0.5 - p.start[1];
            let mut t = if p.radial {
                (dx * dx + dy * dy).sqrt() * p.inv_len
            } else {
                (dx * p.axis[0] + dy * p.axis[1]) * p.inv_len2
            };
            if p.scatter > 0.0 {
                t += (hash01(p.seed, 0, x as i32, y as i32, 0) - 0.5) * p.scatter;
            }
            let t = t.clamp(0.0, 1.0);
            for c in 0..3 {
                let g = p.c0[c] + (p.c1[c] - p.c0[c]) * t;
                rgba[i + c] = rgba[i + c] * (1.0 - p.mix) + g * p.mix;
            }
            rgba[i + 3] = rgba[i + 3] * (1.0 - p.mix) + p.mix;
        }
    }
}

/// Noise (docs/08 §3.36): per-pixel uniform or gaussian grain, mono or per
/// channel, on unpremultiplied colour (§2.2) and re-premultiplied on the way
/// out — a **modifier**, not a generator, so alpha is untouched.
///
/// Gaussian is four uniform draws averaged rather than a Box–Muller pair: it is
/// exact in the same integer hash both paths already share, and it has bounded
/// support, so a gaussian grain cannot produce the single wild outlier a true
/// normal eventually will. Nothing is clipped at either end (§2.1, the one
/// deliberate divergence from AE). `tick` arrives already discretised from layer
/// time, so the kernel never sees a clock (§2.4). Amount 0 short-circuits to the
/// bit-exact identity, and Mix 0 likewise (the WGSL twin matches).
#[allow(clippy::too_many_arguments)]
pub fn noise(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount: f32,
    gaussian: bool,
    colour: bool,
    seed: u32,
    tick: i32,
    mix: f32,
) {
    if amount == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = rgba[i + 3];
            let u = unpremult(&rgba[i..i + 4]);
            for c in 0..3 {
                // Mono grain draws channel 0 for all three, which is what makes
                // it read as luminance noise rather than a tint.
                let ch = if colour { c as u32 } else { 0 };
                let n = noise_draw(seed, ch, x as i32, y as i32, tick, gaussian);
                let grained = (u[c] + n * amount) * a;
                rgba[i + c] = rgba[i + c] * (1.0 - mix) + grained * mix;
            }
        }
    }
}

/// One grain draw in `−1..=1` (docs/08 §3.36): a single uniform lattice draw, or
/// four averaged for the gaussian. The four channels are offset by 4 so a mono
/// gaussian and a colour gaussian never share a draw.
fn noise_draw(seed: u32, channel: u32, x: i32, y: i32, tick: i32, gaussian: bool) -> f32 {
    let draw = |k: u32| hash01(seed, channel + k * 4, x, y, tick) * 2.0 - 1.0;
    if gaussian {
        (draw(0) + draw(1) + draw(2) + draw(3)) * 0.5
    } else {
        draw(0)
    }
}

/// One resolved Fractal noise (docs/08 §3.37), reduced to what both paths read.
/// The rotation arrives as a host-computed cosine/sine pair and every scale as a
/// reciprocal, so the kernel runs no trigonometry and no division (§1.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FractalNoiseParams {
    /// The field's own shape: seed, octaves, gain, lacunarity, type, cycle.
    pub field: FractalField,
    /// `(cos, sin)` of the Rotation control, host-computed.
    pub cos_sin: [f32; 2],
    /// The field origin in raster pixels (the §2.3 preview factor applied).
    pub offset: [f32; 2],
    /// `1 ÷ cell size` per axis, in raster pixels.
    pub inv_scale: [f32; 2],
    /// The depth coordinate: Evolution ÷ 360, folded into the cycle when one is set.
    pub z: f32,
    /// Contrast ÷ 100.
    pub contrast: f32,
    /// Brightness ÷ 100.
    pub brightness: f32,
    pub invert: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Fractal noise (docs/08 §3.37): the seeded multi-octave generator. Replaces
/// the frame edge to edge with opaque grey noise, shaped by contrast and
/// brightness and clamped to 0..1 (§3.37 decision 5 — a generator that cannot be
/// read as a matte is not worth having). Mix 0 is the bit-exact identity (the
/// WGSL twin matches).
pub fn fractal_noise(rgba: &mut [f32], w: u32, h: u32, p: &FractalNoiseParams) {
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5 - p.offset[0];
            let py = y as f32 + 0.5 - p.offset[1];
            // R(−rotation) applied to the pixel offset: the field turns, not
            // the frame.
            let qx = px * p.cos_sin[0] + py * p.cos_sin[1];
            let qy = py * p.cos_sin[0] - px * p.cos_sin[1];
            let n = fractal(&p.field, qx * p.inv_scale[0], qy * p.inv_scale[1], p.z);
            let n01 = n * 0.5 + 0.5;
            let mut v = ((n01 - 0.5) * p.contrast + 0.5 + p.brightness).clamp(0.0, 1.0);
            if p.invert {
                v = 1.0 - v;
            }
            for c in 0..3 {
                rgba[i + c] = rgba[i + c] * (1.0 - p.mix) + v * p.mix;
            }
            rgba[i + 3] = rgba[i + 3] * (1.0 - p.mix) + p.mix;
        }
    }
}

/// One pixel's matte strength: the clamped premultiplied Rec. 709 luma of the
/// matte under it (docs/08 §2.6).
///
/// The one reading of "how much matte is here" — the same expression
/// [`matte_mix`] and the WGSL twins use, so a matte means the same thing whether
/// it dissolves an effect or steers one. Invert is not read here: it is applied
/// exactly once, by [`matte_prepare`] at the seam (K-425), before any kernel
/// sees the matte. A matte shorter than the picture leaves the remaining pixels
/// at full strength: degrade, never fault (14-ENGINEERING-RULES §4).
fn matte_strength(matte: &[f32], i: usize) -> f32 {
    match matte.get(i..i + 3) {
        Some(m) => (m[0] * LUMA[0] + m[1] * LUMA[1] + m[2] * LUMA[2]).clamp(0.0, 1.0),
        None => 1.0,
    }
}

/// A control pulled toward its neutral value by one pixel's matte strength
/// (docs/08 §2.6, the owner's rule for mattes): `neutral·(1 − k) + value·k`.
///
/// **In plain terms.** The matte scales the *amount* of an effect, not its
/// result: white keeps the control where the user set it, black puts it at
/// the value that does nothing (an Amount of 0, a Gamma of 1, a Saturation
/// of 100), grey lands between. Every kernel in the blur, sharpen, colour and
/// distortion families applies this to its named control *before* its maths
/// runs, which is what makes a half-grey matte on Gamma give
/// `pow(x, 1/lerp(1, g, ½))` rather than a fade between the graded and the
/// ungraded picture — and a half-grey matte on a 200° Twirl give the 100°
/// twirl rather than a ghost of the 200° one (K-426, K-427). A distortion's
/// neutral is the pixel's own position, so its kernels pass the sample
/// coordinate as `value` and the pixel centre as `neutral`.
///
/// Spelled in exactly this form, and the WGSL twins spell it the same way
/// (never `mix()`), because at `k = 1` it is `value` to the bit — `v·1 + n·0`
/// — so an empty matte, which [`matte_strength`] reads as 1 everywhere,
/// leaves every number the kernel multiplies by byte-identical to the
/// unmatted path (K-258). At `k = 0` it is `neutral` to the bit for the same
/// reason.
#[inline]
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    neutral * (1.0 - k) + value * k
}

/// One resolved Turbulent displace (docs/08 §3.38), reduced to what both paths
/// read. The size and the pin band arrive as reciprocals and the Displacement
/// choice as a pair of axis multipliers, so the kernel runs no division and no
/// branch on the mode (§1.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurbulentDisplaceParams {
    /// The x field's shape (§3.37's core), seed included.
    pub field: FractalField,
    /// The y field's seed — the same shape under a salted seed (§3.38 decision 3).
    pub seed_y: u32,
    /// `1 ÷ Size`, in raster pixels.
    pub inv_size: f32,
    /// The field origin in raster pixels.
    pub offset: [f32; 2],
    /// The depth coordinate: Evolution ÷ 360, folded into the cycle.
    pub z: f32,
    /// Amount, raster pixels; signed.
    pub amount: f32,
    /// Which components survive: `[1,1]` Turbulent, `[1,0]` Horizontal, `[0,1]`
    /// Vertical.
    pub axes: [f32; 2],
    /// Per axis, 1 when that axis's pair of edges is pinned.
    pub pin: [f32; 2],
    /// `1 ÷ |Amount|`, the reciprocal of the pin ramp's width.
    pub inv_pin_band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Turbulent displace (docs/08 §3.38): the fractal-driven warp — each pixel is
/// pulled along a vector read out of §3.37's noise core, then sampled with one
/// bilinear tap under Repeat edges.
///
/// **The matte scales the vector** rather than dissolving the result (§2.6's
/// K-395 override): `k` multiplies the displacement, so a grey matte warps the
/// picture *less* instead of showing a warped copy over an unwarped one. An
/// empty `matte` is full strength everywhere, which makes this function the
/// byte-for-byte no-matte path as well.
///
/// Amount 0 and Mix 0 are both the bit-exact identity: a zero displacement
/// samples the pixel's own centre, which bilinear reproduces exactly.
pub fn turbulent_displace(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    p: &TurbulentDisplaceParams,
    matte: &[f32],
) {
    let original = rgba.to_vec();
    // The second field is the first under a salted seed — one shape, two
    // decorrelated draws (§3.38 decision 3).
    let mut field_y = p.field;
    field_y.seed = p.seed_y;
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let qx = (px - p.offset[0]) * p.inv_size;
            let qy = (py - p.offset[1]) * p.inv_size;
            let nx = fractal(&p.field, qx, qy, p.z);
            let ny = fractal(&field_y, qx, qy, p.z);
            // Pinning ramps the WHOLE vector to zero across the last |Amount|
            // pixels before a pinned edge, so a pinned corner cannot be reached
            // from outside the frame. Distance is measured to the OUTERMOST
            // PIXEL CENTRE rather than to the border half a pixel beyond it, so
            // the border row is exactly still rather than nearly so — "pinned"
            // has to mean pinned. Written as a lerp toward 1 rather than a
            // branch, so the WGSL `mix` matches op-for-op.
            let ramp = |d: f32| (d * p.inv_pin_band).clamp(0.0, 1.0);
            let pin_x = 1.0 + p.pin[0] * (ramp((px - 0.5).min(fw - 0.5 - px)) - 1.0);
            let pin_y = 1.0 + p.pin[1] * (ramp((py - 0.5).min(fh - 0.5 - py)) - 1.0);
            let s = p.amount * pin_x * pin_y * matte_strength(matte, i);
            let v = bilinear_edge(
                &original,
                w,
                h,
                px + nx * p.axes[0] * s,
                py + ny * p.axes[1] * s,
                1,
            );
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Tile (docs/08 §3.39). The four per cents stay *fractions of the
/// raster* rather than lengths: the kernel already knows the raster, and a
/// length would stop the same resolved op being usable at another one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileParams {
    /// The stamped rectangle's centre, raster pixels.
    pub centre: [f32; 2],
    /// Tile width and height as fractions of the frame.
    pub tile_frac: [f32; 2],
    /// Output width and height as fractions of the frame.
    pub output_frac: [f32; 2],
    /// Phase ÷ 360 — how far each row slides, in tiles.
    pub phase: f32,
    pub mirror_edges: bool,
    pub horizontal_phase_shift: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Tile (docs/08 §3.39): one rectangle of the picture stamped across the frame.
/// Outside the output window the result is transparent; inside, the pixel's
/// position within its tile picks the sample, mirrored on odd tiles when Mirror
/// edges is on. Mix 0 is the bit-exact identity, and so is a 100 % tile at the
/// frame centre with no phase.
pub fn tile(rgba: &mut [f32], w: u32, h: u32, p: &TileParams) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let tw = (fw * p.tile_frac[0]).max(1e-3);
    let th = (fh * p.tile_frac[1]).max(1e-3);
    let (half_w, half_h) = (fw * p.output_frac[0] * 0.5, fh * p.output_frac[1] * 0.5);
    let (cx, cy) = (fw * 0.5, fh * 0.5);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let v = if (px - cx).abs() > half_w || (py - cy).abs() > half_h {
                [0.0f32; 4]
            } else {
                let mut u = (px - p.centre[0]) / tw + 0.5;
                let mut t = (py - p.centre[1]) / th + 0.5;
                // The phase shift is applied along one axis using the OTHER
                // axis's whole tile index, so the two floors have to be taken in
                // the order the switch chooses.
                let (iu, it) = if p.horizontal_phase_shift {
                    let iu = u.floor();
                    t += iu * p.phase;
                    (iu, t.floor())
                } else {
                    let it = t.floor();
                    u += it * p.phase;
                    (u.floor(), it)
                };
                let mut fu = u - iu;
                let mut ft = t - it;
                if p.mirror_edges {
                    // Two's complement `& 1` is odd-ness for negative indices too,
                    // on both paths.
                    if (iu as i64) & 1 != 0 {
                        fu = 1.0 - fu;
                    }
                    if (it as i64) & 1 != 0 {
                        ft = 1.0 - ft;
                    }
                }
                bilinear_edge(
                    &original,
                    w,
                    h,
                    p.centre[0] + (fu - 0.5) * tw,
                    p.centre[1] + (ft - 0.5) * th,
                    1,
                )
            };
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// Wrap-addressed bilinear sample: the frame is a torus, so a tap that leaves
/// one side arrives at the other. Offset's own sampler (docs/08 §3.40) — the
/// three [`EdgesMode`](super::EdgesMode) policies do not include wrapping,
/// because no other effect wants it and a fourth policy on every one of them
/// would be a control nobody sets. Same arithmetic order as [`bilinear`].
fn bilinear_wrap(rgba: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> [f32; 4] {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let (wi, hi) = (w as i64, h as i64);
    let at = |x: i64, y: i64| {
        let xw = ((x % wi) + wi) % wi;
        let yw = ((y % hi) + hi) % hi;
        let s = ((yw * wi + xw) * 4) as usize;
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

/// Offset (docs/08 §3.40): the frame slid by `shift` raster pixels, wrapping
/// round both axes. A zero shift and Mix 0 are both the bit-exact identity — a
/// sample at the pixel's own centre is reproduced exactly by bilinear.
pub fn offset(rgba: &mut [f32], w: u32, h: u32, shift: [f32; 2], mix: f32) {
    offset_matted(rgba, w, h, shift, mix, &[]);
}

/// [`offset`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **the shift** it is read through, so a grey matte slides that part of
/// the frame less and a black one not at all. An empty matte is the unmatted
/// path to the byte (K-258).
pub fn offset_matted(rgba: &mut [f32], w: u32, h: u32, shift: [f32; 2], mix: f32, matte: &[f32]) {
    let original = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = matte_strength(matte, i);
            let v = bilinear_wrap(
                &original,
                w,
                h,
                x as f32 + 0.5 - shift[0] * k,
                y as f32 + 0.5 - shift[1] * k,
            );
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + v[c] * mix;
            }
        }
    }
}

/// Mirror (docs/08 §3.41): the half of the frame the axis normal points into is
/// replaced by the reflection of the other half. `normal` is the host-computed
/// `(cos, sin)` of Angle (§1.6). Samples that land outside the frame read as
/// transparent — a repeat there would smear the border pixel into a fan.
/// Mix 0 is the bit-exact identity.
pub fn mirror(rgba: &mut [f32], w: u32, h: u32, centre: [f32; 2], normal: [f32; 2], mix: f32) {
    let original = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = (px - centre[0]) * normal[0] + (py - centre[1]) * normal[1];
            let (sx, sy) = if d > 0.0 {
                (px - 2.0 * d * normal[0], py - 2.0 * d * normal[1])
            } else {
                (px, py)
            };
            let v = bilinear_edge(&original, w, h, sx, sy, 0);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + v[c] * mix;
            }
        }
    }
}

/// The largest ray angle Lens distort's forward `tan` is allowed — 89°, past
/// which the tangent runs away to infinity and takes the sample position with
/// it. The WGSL twin clamps at the identical literal.
pub const LENS_MAX_THETA: f32 = 1.553_343;

/// One resolved Lens distort (docs/08 §3.42). The one trig call that can be
/// lifted out of the pixel loop is (`tan_half_fov`); the two that cannot are
/// named in §3.42's fourth note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensDistortParams {
    /// False below [`LensDistort::MIN_FOV_DEG`](super::effects::lens_distort::
    /// LensDistort::MIN_FOV_DEG): the exact identity, rather than a division by
    /// a zero tangent.
    pub active: bool,
    /// `tan(Field of view ÷ 2)`, host-computed.
    pub tan_half_fov: f32,
    /// Remove the fisheye rather than add it — the exact inverse mapping.
    pub reverse: bool,
    /// Which half-extent the field of view spans: 0 width, 1 height, 2 diagonal.
    pub half_kind: u32,
    /// The optical centre, raster pixels.
    pub centre: [f32; 2],
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Lens distort (docs/08 §3.42): barrel and pincushion by field of view. The
/// focal length `f = half ÷ tan(fov ÷ 2)` is the frame's own optics; the
/// forward map `r' = f·tan(r ÷ f)` adds a fisheye and `r' = f·atan(r ÷ f)`
/// removes exactly the same one. Field of view 0 and Mix 0 are both the
/// bit-exact identity.
pub fn lens_distort(rgba: &mut [f32], w: u32, h: u32, p: &LensDistortParams) {
    lens_distort_matted(rgba, w, h, p, &[]);
}

/// [`lens_distort`] driven by a matte (K-395, K-427): each pixel's matte
/// strength scales **the distortion's displacement** at that pixel toward
/// none ([`matte_toward`] on the sample position), so the field of view's
/// effect fades to the identity where the matte darkens. Read at the
/// destination pixel. An empty matte is the unmatted path to the byte (K-258).
pub fn lens_distort_matted(rgba: &mut [f32], w: u32, h: u32, p: &LensDistortParams, matte: &[f32]) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let half = match p.half_kind {
        1 => fh * 0.5,
        2 => (fw * fw + fh * fh).sqrt() * 0.5,
        _ => fw * 0.5,
    };
    // Floored so an inactive effect cannot divide by zero on its way to the
    // short-circuit below.
    let f = half / p.tan_half_fov.max(1e-6);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - p.centre[0];
            let dy = py - p.centre[1];
            let r = (dx * dx + dy * dy).sqrt();
            let (sx, sy) = if !p.active || r <= 0.0 {
                (px, py)
            } else {
                let theta = r / f;
                let radius = if p.reverse {
                    f * theta.atan()
                } else {
                    f * theta.min(LENS_MAX_THETA).tan()
                };
                let scale = radius / r;
                let k = matte_strength(matte, i);
                (
                    matte_toward(p.centre[0] + dx * scale, px, k),
                    matte_toward(p.centre[1] + dy * scale, py, k),
                )
            };
            let v = bilinear_edge(&original, w, h, sx, sy, p.edge);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Corner pin (docs/08 §3.48), reduced to what both paths read.
/// The whole projective derivation — the unit-square-to-quad map, its adjugate,
/// the sign normalisation — happens once in
/// [`CornerPin::packed`](super::effects::corner_pin::CornerPin::packed); the
/// kernel runs one matrix multiply, one divide and one tap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerPinParams {
    /// The inverse homography, row-major, taking a raster pixel to the unit
    /// square. Defined only up to a scale (the perspective divide cancels it),
    /// and sign-normalised so `w > 0` means "in front of the horizon".
    pub inv: [[f32; 3]; 3],
    /// False for a degenerate quad: the exact identity, rather than a division
    /// by a zero determinant.
    pub active: bool,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Corner pin (docs/08 §3.48): the picture pulled onto four points. Each output
/// pixel is carried back through the inverse homography into the frame's own
/// coordinates and sampled there; a pixel whose homogeneous `w` comes out
/// non-positive lies **behind** the projection's horizon and is transparent,
/// which is what stops a hard pin drawing a mirrored ghost of the picture.
///
/// Mix 0 and a degenerate quad are both the bit-exact identity.
pub fn corner_pin(rgba: &mut [f32], w: u32, h: u32, p: &CornerPinParams) {
    corner_pin_matted(rgba, w, h, p, &[]);
}

/// [`corner_pin`] driven by a matte (K-395, K-427): each pixel's matte
/// strength scales **the displacement from the frame's own corners** toward
/// none — the matte multiplies the offset the handles set, so where it is
/// black the pixel stays where it was. Read at the destination pixel. A pixel
/// behind the projection's horizon stays transparent whatever the matte says:
/// there is no position to pull it back from. An empty matte is the unmatted
/// path to the byte (K-258).
pub fn corner_pin_matted(rgba: &mut [f32], w: u32, h: u32, p: &CornerPinParams, matte: &[f32]) {
    if !p.active {
        return;
    }
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let u = p.inv[0][0] * px + p.inv[0][1] * py + p.inv[0][2];
            let t = p.inv[1][0] * px + p.inv[1][1] * py + p.inv[1][2];
            let d = p.inv[2][0] * px + p.inv[2][1] * py + p.inv[2][2];
            let v = if d > 0.0 {
                let k = matte_strength(matte, i);
                bilinear_edge(
                    &original,
                    w,
                    h,
                    matte_toward(u / d * fw, px, k),
                    matte_toward(t / d * fh, py, k),
                    p.edge,
                )
            } else {
                [0.0f32; 4]
            };
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Displacement map (docs/08 §3.49), reduced to what both paths
/// read. The Matte row's Invert is not here: it rides beside the layer binding,
/// exactly as Set matte's does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplacementMapParams {
    /// `CHANNEL_OPTIONS` indices: which channel of the map steers x, and which y.
    pub channels: [u32; 2],
    /// The farthest a pixel can be pushed on each axis, raster pixels; signed.
    pub amount: [f32; 2],
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Displacement map (docs/08 §3.49) — the CPU reference and §1.6 oracle.
///
/// **The matte IS the map** (§2.6's K-395 override, the seventh): its chosen
/// channels say which way and how far each pixel is pushed, with **mid-grey the
/// neutral** — 0.5 moves nothing, 1 pushes a full Amount one way and 0 a full
/// Amount the other, which is AE's convention and the only one a single map can
/// push both ways under.
///
/// `map` is the referenced layer's picture at this raster, RGBA rather than one
/// channel because which channel steers which axis is the effect's own control.
/// An empty slice is the unbound case — the labelled no-op every layer-input
/// effect follows — and leaves the picture untouched.
pub fn displacement_map(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    p: &DisplacementMapParams,
    map: &[f32],
    invert: bool,
) {
    if map.is_empty() {
        return;
    }
    let original = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            // A map shorter than the picture leaves the rest alone — degrade,
            // never fault (14-ENGINEERING-RULES §4).
            let Some(m) = map.get(i..i + 4) else {
                return;
            };
            let mut kx = channel_of(m, p.channels[0]);
            let mut ky = channel_of(m, p.channels[1]);
            if invert {
                kx = 1.0 - kx;
                ky = 1.0 - ky;
            }
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let v = bilinear_edge(
                &original,
                w,
                h,
                px + (kx - 0.5) * 2.0 * p.amount[0],
                py + (ky - 0.5) * 2.0 * p.amount[1],
                p.edge,
            );
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Polar coordinates (docs/08 §3.50). The centre and the radius
/// scale are deliberately absent: both are functions of the raster, which the
/// kernel knows and the host does not (§3.39's precedent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolarParams {
    /// True for Rectangular to polar (rows become rings), false for its exact
    /// inverse.
    pub to_polar: bool,
    /// Interpolation ÷ 100: how far along its own path into the other space each
    /// pixel is drawn from. 0 is the bit-exact identity.
    pub interp: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Polar coordinates (docs/08 §3.50): the frame bent into a circle, and back.
/// The radius spans **half the frame diagonal**, so the frame's corners are
/// inside the mapped disc and a wrapped picture has no bald corners. The angle
/// starts straight up and turns clockwise, the catalogue's convention, which is
/// also where the seam of a wrapped picture falls.
///
/// Mix 0 and Interpolation 0 are both the bit-exact identity: a zero step along
/// the path samples the pixel's own centre, which bilinear reproduces exactly.
pub fn polar_coordinates(rgba: &mut [f32], w: u32, h: u32, p: &PolarParams) {
    use std::f32::consts::TAU;
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let (cx, cy) = (fw * 0.5, fh * 0.5);
    let radius = 0.5 * (fw * fw + fh * fh).sqrt();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let (qx, qy) = if p.to_polar {
                let dx = px - cx;
                let dy = py - cy;
                // atan2(x, −y) is "from straight up, clockwise" on a raster
                // whose y grows downward, the same reading §3.46 and §3.47 use.
                let turns = dx.atan2(-dy) / TAU;
                let wrapped = turns - turns.floor();
                (wrapped * fw, (dx * dx + dy * dy).sqrt() / radius * fh)
            } else {
                let theta = px / fw * TAU;
                let r = py / fh * radius;
                (cx + r * theta.sin(), cy - r * theta.cos())
            };
            let v = bilinear_edge(
                &original,
                w,
                h,
                px + (qx - px) * p.interp,
                py + (qy - py) * p.interp,
                0,
            );
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Twirl (docs/08 §3.51). The radius arrives as a reciprocal so the
/// kernel runs no division; the angle stays an angle because it is multiplied by
/// a per-pixel falloff before any trigonometry is taken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwirlParams {
    /// The twirl's middle, raster pixels.
    pub centre: [f32; 2],
    /// The twirled circle's radius, raster pixels.
    pub radius: f32,
    /// `1 ÷ radius`, floored so a zero radius does not divide.
    pub inv_radius: f32,
    /// Angle in radians; positive turns the picture clockwise on screen.
    pub angle: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Twirl (docs/08 §3.51): the picture wrung round a point, hardest at the middle
/// and not at all at the rim. The falloff is **squared**, so the twist eases out
/// with zero slope at the rim rather than stopping at a crease.
///
/// A rotation about the centre preserves the radius, so a twirl never samples
/// outside its own circle; the only samples that leave the frame are the ones
/// whose circle already hung over the edge, and those read transparent.
///
/// Mix 0, Angle 0 and Radius 0 are all the bit-exact identity.
pub fn twirl(rgba: &mut [f32], w: u32, h: u32, p: &TwirlParams) {
    twirl_matted(rgba, w, h, p, &[]);
}

/// [`twirl`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **Angle** before the turn is taken, read at the destination pixel,
/// so a grey matte is a gentler twirl there and not a full twirl faded back.
/// An empty matte is the unmatted path to the byte (K-258).
pub fn twirl_matted(rgba: &mut [f32], w: u32, h: u32, p: &TwirlParams, matte: &[f32]) {
    let original = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - p.centre[0];
            let dy = py - p.centre[1];
            let r = (dx * dx + dy * dy).sqrt();
            let (sx, sy) = if r >= p.radius {
                (px, py)
            } else {
                let t = 1.0 - r * p.inv_radius;
                let angle = p.angle * matte_strength(matte, i);
                let (sin, cos) = (angle * t * t).sin_cos();
                // R(−φ) applied to the offset: the picture turns by +φ.
                (
                    p.centre[0] + dx * cos + dy * sin,
                    p.centre[1] - dx * sin + dy * cos,
                )
            };
            let v = bilinear_edge(&original, w, h, sx, sy, 0);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Spherize (docs/08 §3.52).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpherizeParams {
    /// The ball's middle, raster pixels.
    pub centre: [f32; 2],
    /// The ball's radius, raster pixels.
    pub radius: f32,
    /// `1 ÷ radius`, floored so a zero radius does not divide.
    pub inv_radius: f32,
    /// Bulge ÷ 100, −1..1. The sign chooses the map, the magnitude blends
    /// toward it; 0 is the exact identity.
    pub bulge: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Spherize (docs/08 §3.52): a glass ball held over the picture. The two
/// directions are **mutually inverse radial maps** — `(2 ÷ π)·asin ρ` magnifies
/// the middle and `sin(ρ·π ÷ 2)` is exactly its undo — so a bulge and a pinch of
/// the same strength, radius and centre cancel to sampling error.
///
/// Mix 0, Bulge 0 and Radius 0 are all the bit-exact identity: a zero bulge
/// leaves the sample radius exactly its own, and a sample at the pixel's own
/// centre is reproduced exactly by bilinear.
pub fn spherize(rgba: &mut [f32], w: u32, h: u32, p: &SpherizeParams) {
    spherize_matted(rgba, w, h, p, &[]);
}

/// [`spherize`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **Bulge** toward 0 before the map is blended, read at the
/// destination pixel. An empty matte is the unmatted path to the byte (K-258).
pub fn spherize_matted(rgba: &mut [f32], w: u32, h: u32, p: &SpherizeParams, matte: &[f32]) {
    use std::f32::consts::PI;
    let original = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - p.centre[0];
            let dy = py - p.centre[1];
            let r = (dx * dx + dy * dy).sqrt();
            // Bulge 0 short-circuits, as Lens distort's Field of view 0 does
            // (§3.42): the blend below would leave `scale` at rho ÷ rho, and a
            // GPU that compiles that division as a reciprocal-multiply answers a
            // hair under 1 — which is a whole picture of resampling for an
            // effect the user has turned off.
            let bulge = p.bulge * matte_strength(matte, i);
            let (sx, sy) = if r >= p.radius || r <= 0.0 || bulge == 0.0 {
                (px, py)
            } else {
                // Clamped: a radius rounded a hair below `r` would hand `asin`
                // an argument above 1 and it would answer NaN.
                let rho = (r * p.inv_radius).min(1.0);
                let target = if bulge >= 0.0 {
                    (2.0 / PI) * rho.asin()
                } else {
                    (rho * PI * 0.5).sin()
                };
                let scale = (rho + (target - rho) * bulge.abs()) / rho;
                (p.centre[0] + dx * scale, p.centre[1] + dy * scale)
            };
            let v = bilinear_edge(&original, w, h, sx, sy, 0);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Ripple (docs/08 §3.53). Both lengths arrive as reciprocals so
/// the kernel runs no division, and Wave height arrives already multiplied by
/// the envelope's peak reciprocal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RippleParams {
    /// The rings' middle, raster pixels.
    pub centre: [f32; 2],
    /// How far the rings reach, raster pixels.
    pub radius: f32,
    /// `1 ÷ radius`, floored so a zero radius does not divide.
    pub inv_radius: f32,
    /// The farthest a pixel moves, raster pixels: Wave height times `27⁄4`
    /// (docs/08 §3.53 decision 1).
    pub amount: f32,
    /// `1 ÷ Wave width`, raster pixels.
    pub inv_width: f32,
    /// Evolution ÷ 360: whole waves sent outward.
    pub turns: f32,
    /// Asymmetric adds the tangential half of the wave, a quarter-turn out of
    /// phase, so a pixel walks a small circle instead of sliding.
    pub asymmetric: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Ripple (docs/08 §3.53): rings spreading from a point.
///
/// The envelope `27⁄4·ρ(1 − ρ)²` is zero at the centre as well as at the rim,
/// which removes the direction singularity at `r = 0` exactly and is also the
/// true shape of a spreading disturbance; the constant makes Wave height
/// literally the farthest a pixel moves.
///
/// Mix 0, Radius 0 and Wave height 0 are all the bit-exact identity.
pub fn ripple(rgba: &mut [f32], w: u32, h: u32, p: &RippleParams) {
    ripple_matted(rgba, w, h, p, &[]);
}

/// [`ripple`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **Wave height** before the rings move it, read at the destination
/// pixel. An empty matte is the unmatted path to the byte (K-258).
pub fn ripple_matted(rgba: &mut [f32], w: u32, h: u32, p: &RippleParams, matte: &[f32]) {
    use std::f32::consts::TAU;
    let original = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - p.centre[0];
            let dy = py - p.centre[1];
            let r = (dx * dx + dy * dy).sqrt();
            let amount = p.amount * matte_strength(matte, i);
            let (sx, sy) = if r >= p.radius || r <= 0.0 || amount == 0.0 {
                (px, py)
            } else {
                let rho = (r * p.inv_radius).min(1.0);
                let one = 1.0 - rho;
                let env = rho * one * one * amount;
                let phase = TAU * (r * p.inv_width - p.turns);
                let (sin, cos) = phase.sin_cos();
                // The unit radial, and the unit tangential a quarter-turn
                // clockwise from it on a raster whose y grows downward.
                let inv_r = 1.0 / r;
                let nx = dx * inv_r;
                let ny = dy * inv_r;
                if p.asymmetric {
                    (
                        px + (nx * sin - ny * cos) * env,
                        py + (ny * sin + nx * cos) * env,
                    )
                } else {
                    (px + nx * sin * env, py + ny * sin * env)
                }
            };
            // Repeat edges, as §3.54's and §3.38's warps use: a ring wider than
            // the frame's own half-height reaches outside it, and a transparent
            // edge would punch a bite out of the picture where the crest was.
            let v = bilinear_edge(&original, w, h, sx, sy, 1);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Wave warp (docs/08 §3.54). The direction's sine and cosine are
/// spent host-side into the two unit vectors, so neither path runs trigonometry
/// beyond the wave shape itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveWarpParams {
    /// The direction the wave travels, host-computed `(sin θ, −cos θ)`.
    pub dir: [f32; 2],
    /// That vector turned a quarter-turn clockwise on screen: the direction the
    /// picture slides.
    pub perp: [f32; 2],
    /// How far the picture slides at a crest, raster pixels; signed.
    pub height: f32,
    /// `1 ÷ Wave width`, raster pixels.
    pub inv_width: f32,
    /// Phase ÷ 360, in whole waves.
    pub turns: f32,
    /// 0 Sine, 1 Square, 2 Triangle, 3 Sawtooth, 4 Circle.
    pub shape: u32,
    /// Per edge — left, right, top, bottom — 1 when that edge is pinned.
    pub pin: [f32; 4],
    /// `1 ÷ |Wave height|` — the pin ramp's width, reciprocated.
    pub inv_pin_band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The five wave shapes (docs/08 §3.54's table), each running `−1..=1` over one
/// whole wave. Written as one function so the CPU reference and the WGSL kernel
/// cannot drift on the awkward ones.
#[must_use]
pub fn wave_shape(shape: u32, t: f32) -> f32 {
    use std::f32::consts::TAU;
    let f = t - t.floor();
    match shape {
        1 => {
            if f < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        2 => {
            let q = t + 0.25;
            1.0 - 4.0 * ((q - q.floor()) - 0.5).abs()
        }
        3 => 2.0 * f - 1.0,
        4 => {
            // Two half-circles a wave, the second one below the line.
            let b = 2.0 * f;
            let u = 2.0 * (b - b.floor()) - 1.0;
            let arc = (1.0 - u * u).max(0.0).sqrt();
            if f < 0.5 {
                arc
            } else {
                -arc
            }
        }
        _ => (TAU * t).sin(),
    }
}

/// Wave warp (docs/08 §3.54): a travelling wave across the frame — the
/// transverse one, so the picture slides *across* the direction the wave runs
/// in, which is what a flag does.
///
/// The edges repeat rather than fading: an unpinned wave carries the picture off
/// the frame and a transparent edge would leave a hole where the crest was.
///
/// Mix 0 and Wave height 0 are both the bit-exact identity.
pub fn wave_warp(rgba: &mut [f32], w: u32, h: u32, p: &WaveWarpParams) {
    wave_warp_matted(rgba, w, h, p, &[]);
}

/// [`wave_warp`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **Wave height** before the slide, read at the destination pixel. The
/// pinned edges keep their ramp width (`inv_pin_band` is the host's), so a
/// pinned border is still exactly still. An empty matte is the unmatted path
/// to the byte (K-258).
pub fn wave_warp_matted(rgba: &mut [f32], w: u32, h: u32, p: &WaveWarpParams, matte: &[f32]) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let (cx, cy) = (fw * 0.5, fh * 0.5);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let along = (px - cx) * p.dir[0] + (py - cy) * p.dir[1];
            let wave = wave_shape(p.shape, along * p.inv_width - p.turns);
            // Each pinned edge ramps the WHOLE slide to zero across the last
            // |Wave height| pixels before it, measured to the outermost pixel
            // centre so the border row is exactly still. A lerp toward 1 rather
            // than a branch, so the four factors simply multiply.
            let ramp = |d: f32| (d * p.inv_pin_band).clamp(0.0, 1.0);
            let pin = (1.0 + p.pin[0] * (ramp(px - 0.5) - 1.0))
                * (1.0 + p.pin[1] * (ramp(fw - 0.5 - px) - 1.0))
                * (1.0 + p.pin[2] * (ramp(py - 0.5) - 1.0))
                * (1.0 + p.pin[3] * (ramp(fh - 0.5 - py) - 1.0));
            let s = p.height * matte_strength(matte, i) * wave * pin;
            let v = bilinear_edge(&original, w, h, px + p.perp[0] * s, py + p.perp[1] * s, 1);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// One resolved Bezier warp (docs/08 §3.55).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BezierWarpParams {
    /// The twelve points in AE's clockwise walk from the upper left — corner,
    /// two handles, corner, two handles, … — raster pixels.
    pub pts: [[f32; 2]; 12],
    /// Newton steps a pixel, 1..=12 (the Quality control).
    pub steps: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The narrowest Jacobian determinant the patch inversion will divide by. Below
/// it the patch has folded on itself and there is no single answer; the solver
/// stops where it stands rather than dividing by zero (14-ENGINEERING-RULES §4).
const BEZ_MIN_DET: f32 = 1e-9;

/// How far the solved point may still be from the pixel it was solving for,
/// raster pixels (docs/08 §3.55 decision 3). **A Newton solve has to be checked,
/// not trusted**: outside the patch there is no answer, and an unchecked
/// iteration wanders until it happens to land in `0..1` — which draws a scatter
/// of stray pixels across the empty part of the frame. One more patch evaluation
/// says whether the answer is an answer.
const BEZ_MAX_RESIDUAL_PX: f32 = 1.0;

/// How near its own centre a sample has to land to be snapped to it, raster
/// pixels (docs/08 §3.55 decision 4). Four orders of magnitude below anything a
/// resampler could show, and it is what makes an unbent region of a bent frame
/// bit-exact rather than softened.
const BEZ_SNAP_PX: f32 = 1e-3;

/// One cubic Bezier and its derivative at `t`.
fn bez(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2], t: f32) -> ([f32; 2], [f32; 2]) {
    let s = 1.0 - t;
    let (w0, w1, w2, w3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
    let (g0, g1, g2) = (3.0 * s * s, 6.0 * s * t, 3.0 * t * t);
    let mut pos = [0.0f32; 2];
    let mut tan = [0.0f32; 2];
    for k in 0..2 {
        pos[k] = a[k] * w0 + b[k] * w1 + c[k] * w2 + d[k] * w3;
        tan[k] = (b[k] - a[k]) * g0 + (c[k] - b[k]) * g1 + (d[k] - c[k]) * g2;
    }
    (pos, tan)
}

/// The Coons patch on the twelve points, and its two partial derivatives, at
/// `(u, v)` — the surface `S` of docs/08 §3.55 and the Jacobian Newton needs.
///
/// Written once and called from both the CPU reference and (op-for-op) the WGSL
/// kernel: the two boundary curves in each direction blended across, minus the
/// bilinear surface on the four corners, which the two blends between them count
/// twice.
fn coons(pts: &[[f32; 2]; 12], u: f32, v: f32) -> ([f32; 2], [f32; 2], [f32; 2]) {
    let (ul, ur, lr, ll) = (pts[0], pts[3], pts[6], pts[9]);
    // Top left → right, bottom left → right, left top → bottom, right top →
    // bottom: the clockwise walk read in each curve's own increasing direction.
    let (top, dtop) = bez(ul, pts[1], pts[2], ur, u);
    let (bot, dbot) = bez(ll, pts[8], pts[7], lr, u);
    let (lef, dlef) = bez(ul, pts[11], pts[10], ll, v);
    let (rig, drig) = bez(ur, pts[4], pts[5], lr, v);
    let mut s = [0.0f32; 2];
    let mut su = [0.0f32; 2];
    let mut sv = [0.0f32; 2];
    for k in 0..2 {
        let corners = (1.0 - u) * (1.0 - v) * ul[k]
            + u * (1.0 - v) * ur[k]
            + (1.0 - u) * v * ll[k]
            + u * v * lr[k];
        s[k] = (1.0 - v) * top[k] + v * bot[k] + (1.0 - u) * lef[k] + u * rig[k] - corners;
        su[k] = (1.0 - v) * dtop[k] + v * dbot[k] - lef[k] + rig[k]
            - (-(1.0 - v) * ul[k] + (1.0 - v) * ur[k] - v * ll[k] + v * lr[k]);
        sv[k] = -top[k] + bot[k] + (1.0 - u) * dlef[k] + u * drig[k]
            - (-(1.0 - u) * ul[k] - u * ur[k] + (1.0 - u) * ll[k] + u * lr[k]);
    }
    (s, su, sv)
}

/// Bezier warp (docs/08 §3.55) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** The twelve points bend the frame's four edges into cubic
/// curves and the inside follows smoothly. Rendering asks "where did this output
/// pixel come from", so every pixel *solves* the patch backwards by Newton's
/// method from its own position — which is the identity patch's own answer, so
/// an untouched frame converges before it starts.
///
/// Outside the patch is transparent; a sample landing within
/// [`BEZ_SNAP_PX`] of its own centre is snapped to it, so an unbent region of a
/// bent frame is bit-exact rather than resampled.
pub fn bezier_warp(rgba: &mut [f32], w: u32, h: u32, p: &BezierWarpParams) {
    bezier_warp_matted(rgba, w, h, p, &[]);
}

/// [`bezier_warp`] driven by a matte (K-395, K-427): each pixel's matte
/// strength scales **the displacement from the identity patch** toward none,
/// after the solve and its snap ([`matte_toward`] on the sample position), so
/// the matte multiplies the offset the handles set and a black matte leaves
/// the pixel where it was. Read at the destination pixel; a pixel outside the
/// patch stays transparent, since there is no solution to pull toward. An
/// empty matte is the unmatted path to the byte (K-258).
pub fn bezier_warp_matted(rgba: &mut [f32], w: u32, h: u32, p: &BezierWarpParams, matte: &[f32]) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let mut u = px / fw;
            let mut v = py / fh;
            for _ in 0..p.steps {
                let (s, du, dv) = coons(&p.pts, u, v);
                let fx = s[0] - px;
                let fy = s[1] - py;
                let det = du[0] * dv[1] - du[1] * dv[0];
                if det.abs() < BEZ_MIN_DET {
                    break;
                }
                let inv = 1.0 / det;
                u -= (fx * dv[1] - fy * dv[0]) * inv;
                v -= (du[0] * fy - du[1] * fx) * inv;
            }
            // The solve, verified: in range *and* actually solving.
            let (back, _, _) = coons(&p.pts, u, v);
            let miss = (back[0] - px).abs().max((back[1] - py).abs());
            let v4 = if !(0.0..=1.0).contains(&u)
                || !(0.0..=1.0).contains(&v)
                || miss > BEZ_MAX_RESIDUAL_PX
            {
                [0.0f32; 4]
            } else {
                let mut sx = u * fw;
                let mut sy = v * fh;
                if (sx - px).abs() < BEZ_SNAP_PX && (sy - py).abs() < BEZ_SNAP_PX {
                    sx = px;
                    sy = py;
                }
                let k = matte_strength(matte, i);
                bilinear_edge(
                    &original,
                    w,
                    h,
                    matte_toward(sx, px, k),
                    matte_toward(sy, py, k),
                    0,
                )
            };
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + v4[c] * p.mix;
            }
        }
    }
}

/// One resolved Warp (docs/08 §3.56).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpParams {
    /// Which of the thirteen bends, in §3.56's table order.
    pub style: u32,
    /// Bend ÷ 100, −1..1. 0 is the exact identity for every style.
    pub bend: f32,
    /// Horizontal distortion ÷ 100, clamped to ±0.9 so the taper's divisor
    /// never reaches zero.
    pub h_distort: f32,
    /// Vertical distortion ÷ 100; see [`h_distort`](Self::h_distort).
    pub v_distort: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The thirteen bends of docs/08 §3.56, on a frame normalised to `−1..=1` on
/// each axis. Written once and mirrored op-for-op in WGSL.
///
/// Each map is written so that `a = 0` returns its argument untouched, which is
/// what makes Bend 0 the identity for every style.
///
/// `ar` is the frame's aspect ratio (half-width over half-height) and is read by
/// **Twist alone** (docs/08 §3.56's second note): a rotation has to happen in a
/// space where both axes measure the same thing, or a "quarter turn" on a 16∶9
/// frame is really a huge horizontal shear. Every other style is deliberately
/// elliptical.
#[must_use]
pub fn warp_style(style: u32, u: f32, v: f32, a: f32, ar: f32) -> [f32; 2] {
    use std::f32::consts::PI;
    let d = 1.0 - u * u;
    let e = 1.0 - v * v;
    match style {
        // Arc upper / Arc lower: the same bow weighted to one edge.
        1 => [u, v + a * d * (1.0 - v) * 0.5],
        2 => [u, v + a * d * (1.0 + v) * 0.5],
        // Arch: top and bottom bow apart. The coefficient is SUBTRACTED here and
        // in the four styles below: this is a gather, so pulling the sample
        // *inward* is what makes the picture swell outward, and a positive Bend
        // has to do what the style's name promises.
        3 => [u, v * (1.0 - a * d)],
        // Bulge: the middle swells on both axes.
        4 => [u * (1.0 - a * e * 0.5), v * (1.0 - a * d * 0.5)],
        // Flag: one wave across the width, every row in step.
        5 => [u, v + a * 0.35 * (PI * u).sin()],
        // Wave: the same wave with the two edges out of phase.
        6 => [u, v - a * 0.35 * v * (PI * u).sin()],
        // Fish: the sides bow out and the ends taper.
        7 => [u * (1.0 - a * e * 0.5), v],
        // Rise: a diagonal lift.
        8 => [u, v + a * (u + 1.0) * 0.5],
        // Fisheye and Inflate: radial swells differing only in their falloff.
        9 => {
            let rho = (u * u + v * v).sqrt().min(1.0);
            let k = 1.0 - a * (1.0 - rho * rho) * 0.6;
            [u * k, v * k]
        }
        10 => {
            let rho = (u * u + v * v).sqrt().min(1.0);
            let k = 1.0 - a * (1.0 - rho) * 0.6;
            [u * k, v * k]
        }
        // Squeeze: rows crowded toward the middle.
        11 => [u, v * (1.0 + a * e)],
        // Twist: the top turns one way and the bottom the other. R(−φ) applied
        // to the point, as §3.51's rotation is, in the isotropic space `ar`
        // buys. The horizontal component is carried back as a *difference* so
        // that a zero angle returns `u` to the bit rather than `u·ar ÷ ar`.
        12 => {
            let x = u * ar;
            let (sin, cos) = (a * PI * 0.5 * v).sin_cos();
            let rx = x * cos + v * sin;
            [u + (rx - x) / ar, -x * sin + v * cos]
        }
        // Arc: the whole picture bows one way.
        _ => [u, v + a * d],
    }
}

/// Warp (docs/08 §3.56): the thirteen bend presets, one kernel.
///
/// The sample is built from the **difference** between the style's output and
/// the pixel's own normalised position, not rebuilt from the normalised
/// coordinate — which is what makes Bend 0 with both distortions 0 the bit-exact
/// identity rather than a rounding away from one.
pub fn warp(rgba: &mut [f32], w: u32, h: u32, p: &WarpParams) {
    warp_matted(rgba, w, h, p, &[]);
}

/// [`warp`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **Bend and both distortions** toward 0 before the style runs, read
/// at the destination pixel. An empty matte is the unmatted path to the byte
/// (K-258).
pub fn warp_matted(rgba: &mut [f32], w: u32, h: u32, p: &WarpParams, matte: &[f32]) {
    let original = rgba.to_vec();
    let half_w = w as f32 * 0.5;
    let half_h = h as f32 * 0.5;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let u = px / half_w - 1.0;
            let v = py / half_h - 1.0;
            let k = matte_strength(matte, i);
            let m = warp_style(p.style, u, v, p.bend * k, half_w / half_h);
            // The two perspective tapers, both taken from the style's output so
            // neither feeds the other (§3.56's fourth note).
            let du = m[0] / (1.0 + p.v_distort * k * m[1]);
            let dv = m[1] / (1.0 + p.h_distort * k * m[0]);
            let sx = px + (du - u) * half_w;
            let sy = py + (dv - v) * half_h;
            let val = bilinear_edge(&original, w, h, sx, sy, 0);
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + val[c] * p.mix;
            }
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

/// Side of the square tiles the dominant-motion reduction works in, in pixels
/// (docs/impl/optical-flow.md §4.5 item 3). The WGSL uses the same constant;
/// changing one without the other breaks the §1.6 parity.
pub const MB_TILE: u32 = 16;

/// How much of the neighbourhood's dominant streak an entirely unconfident
/// pixel draws. **The centre of the v2 reconstruction**: v1 multiplied the
/// streak by confidence, so an uncertain pixel collapsed to no blur at all and
/// read as a frozen speck in the middle of a moving frame. Here an uncertain
/// pixel instead *borrows* the motion its neighbourhood is certain about, at a
/// tempered length — visibly blurred, plausibly directed, never sharp against a
/// smeared surround. Tempered rather than full because a borrowed vector is a
/// guess about this pixel: it should read as motion, not assert a length it
/// cannot know.
pub const MB_DOM_TEMPER: f32 = 0.6;

/// The weight a wholly unconfident vector still carries when tiles are scored.
/// **Not zero, and that is the point.** Scoring a tile by `conf · ‖v‖` alone
/// means a region where nothing matched — smoke, a muzzle flash, fast water —
/// scores zero everywhere and the tile reads as *still*, so the pixels that most
/// need a borrowed direction are handed a zero one. Flooring the weight lets an
/// untrusted vector represent its tile when there is nothing better, while a
/// trusted vector four times shorter still outranks it.
pub const MB_SCORE_FLOOR: f32 = 0.25;

/// The dominant motion per tile: the `(u, v)` of the highest-scoring pixel in
/// each `MB_TILE`-square tile, where score is
/// `(MB_SCORE_FLOOR + (1 − MB_SCORE_FLOOR) · conf) · ‖(u, v)‖` — confidence
/// weighted, so one wild vector in a badly matched patch cannot capture the tile
/// away from a slower vector the measurement actually believes, but a tile with
/// no confident vector at all still reports the motion it saw.
///
/// Returns `(tiles, tiles_x, tiles_y)`, each tile `[u, v, score]`, row-major.
/// The GPU twin is `fx_mb_tilemax.wgsl`, one thread per tile, scanning the same
/// pixels in the same raster order with the same strictly-greater comparison —
/// so ties resolve to the same pixel on both and the choice is a copy, never an
/// average, and therefore bit-identical rather than merely close.
#[must_use]
pub fn motion_blur_tiles(
    u: &[f32],
    v: &[f32],
    conf: &[f32],
    w: u32,
    h: u32,
) -> (Vec<[f32; 3]>, u32, u32) {
    let tiles_x = w.div_ceil(MB_TILE);
    let tiles_y = h.div_ceil(MB_TILE);
    let mut tiles = vec![[0.0f32; 3]; (tiles_x * tiles_y) as usize];
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let mut best = [0.0f32; 3];
            let mut best_score = -1.0f32;
            for py in (ty * MB_TILE)..((ty + 1) * MB_TILE).min(h) {
                for px in (tx * MB_TILE)..((tx + 1) * MB_TILE).min(w) {
                    let i = (py * w + px) as usize;
                    let (uu, vv) = (u[i], v[i]);
                    let trust = MB_SCORE_FLOOR + (1.0 - MB_SCORE_FLOOR) * conf[i].clamp(0.0, 1.0);
                    let score = trust * (uu * uu + vv * vv).sqrt();
                    if score > best_score {
                        best_score = score;
                        best = [uu, vv, score];
                    }
                }
            }
            tiles[(ty * tiles_x + tx) as usize] = best;
        }
    }
    (tiles, tiles_x, tiles_y)
}

/// The dominant streak for the pixel in tile `(tx, ty)`: the highest-scoring
/// tile of the 3×3 neighbourhood (clamped at the frame edge, so a border tile
/// simply reads itself more than once). Guertin's neighbour-max — a tile only
/// knows about motion inside itself, and an object one tile away is exactly the
/// thing whose smear should reach in.
///
/// This answers "which way might something have flown into me", and an extremum
/// is the right summary for it: the point is to catch the fast thing.
fn neighbour_max(tiles: &[[f32; 3]], tiles_x: u32, tiles_y: u32, tx: u32, ty: u32) -> (f32, f32) {
    let mut dom = (0.0f32, 0.0f32);
    let mut best = -1.0f32;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let nx = (tx as i32 + dx).clamp(0, tiles_x as i32 - 1) as u32;
            let ny = (ty as i32 + dy).clamp(0, tiles_y as i32 - 1) as u32;
            let t = tiles[(ny * tiles_x + nx) as usize];
            if t[2] > best {
                best = t[2];
                dom = (t[0], t[1]);
            }
        }
    }
    dom
}

/// The motion an uncertain pixel **borrows**: the tile field sampled bilinearly
/// between tile centres, rather than one tile's winning vector.
///
/// # In plain terms, because this is the subtle one
///
/// Borrowing and scattering are different questions and they want different
/// summaries. [`neighbour_max`] answers "what is the fastest thing near me",
/// and an extremum is right for that. Borrowing asks "what is my neighbourhood
/// *doing*", and an extremum is badly wrong for it: it is the single most
/// unusual vector out of two hundred and fifty-six, chosen where — by
/// construction — the measurement is least trustworthy. Two tiles side by side
/// then win two unrelated wild vectors, the pixels between them borrow
/// different directions, and the result is smear in rectangular patches. That
/// artefact was measured on real footage, not imagined: a fast zoom on cel
/// animation, 70% of the frame below half confidence, tile-shaped blocks of
/// differently-angled blur across the characters' faces.
///
/// Interpolating between tile centres fixes both halves of it. The borrowed
/// direction becomes continuous, so no tile edge can show. And **disagreement
/// cancels**: four neighbouring tiles that agree reinforce into a full-strength
/// vector, while four that point at random average toward zero — so where there
/// is a consensus to borrow the blur commits to it, and where there is none it
/// quietly backs off toward not blurring at all. That is the correct behaviour
/// falling out of the arithmetic rather than being special-cased.
fn tile_bilinear(tiles: &[[f32; 3]], tiles_x: u32, tiles_y: u32, x: u32, y: u32) -> (f32, f32) {
    // Tile (tx, ty) speaks for the pixel at its centre, ((tx + 0.5) · TILE).
    let fx = (x as f32 + 0.5) / MB_TILE as f32 - 0.5;
    let fy = (y as f32 + 0.5) / MB_TILE as f32 - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let at = |cx: i32, cy: i32| {
        let cx = cx.clamp(0, tiles_x as i32 - 1) as u32;
        let cy = cy.clamp(0, tiles_y as i32 - 1) as u32;
        let t = tiles[(cy * tiles_x + cx) as usize];
        (t[0], t[1])
    };
    let (xi, yi) = (x0 as i32, y0 as i32);
    let (c00, c10, c01, c11) = (
        at(xi, yi),
        at(xi + 1, yi),
        at(xi, yi + 1),
        at(xi + 1, yi + 1),
    );
    let top = (
        c00.0 * (1.0 - tx) + c10.0 * tx,
        c00.1 * (1.0 - tx) + c10.1 * tx,
    );
    let bottom = (
        c01.0 * (1.0 - tx) + c11.0 * tx,
        c01.1 * (1.0 - tx) + c11.1 * tx,
    );
    (
        top.0 * (1.0 - ty) + bottom.0 * ty,
        top.1 * (1.0 - ty) + bottom.1 * ty,
    )
}

/// `1` where `d` is well inside a streak of length `l`, falling to `0` at its
/// end — "could a thing of this streak length have covered this distance".
fn mb_cone(d: f32, l: f32) -> f32 {
    (1.0 - d / l.max(1e-4)).clamp(0.0, 1.0)
}

/// `1` inside a streak of length `l`, `0` outside, with a soft edge — the term
/// that keeps a *uniformly* moving region integrating like the box a shutter is
/// (docs/impl/optical-flow.md §4), rather than the triangle the two cones alone
/// would give.
fn mb_cylinder(d: f32, l: f32) -> f32 {
    let l = l.max(1e-4);
    let (e0, e1) = (0.95 * l, 1.05 * l);
    let t = ((d - e0) / (e1 - e0)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The §1.6 oracle for Fast motion blur (docs/08 §3.2): the CPU twin of
/// `fx_motionblur.wgsl`, op-for-op. `rgba` is linear premultiplied RGBA,
/// mutated in place; `u`/`v` are the per-pixel forward flow (pixels of this
/// raster, one entry per pixel) the decode worker measured between the current
/// source frame and the next, and `conf` is the matching per-pixel confidence
/// in 0..1 ([`lumit_flow::confidence`]).
///
/// # In plain terms
///
/// v1 smeared every pixel along its *own* vector and shortened that streak by
/// its confidence. Two things were wrong with it, and this is the Guertin-class
/// reconstruction that fixes both (docs/impl/optical-flow.md §4.5 item 3,
/// K-390).
///
/// *A fast object never smeared onto what it passed.* Gathering along your own
/// vector means a still background pixel gathers from itself, so the aeroplane
/// stays inside its own outline while the sky behind it stays razor sharp.
/// Here each pixel also gathers along the **dominant** motion of its
/// neighbourhood ([`motion_blur_tiles`] + [`neighbour_max`]), and each tap is
/// weighted by whether the sample it found could plausibly have travelled here
/// — its own streak reaching out ([`mb_cone`]), this pixel's streak reaching in,
/// and a [`mb_cylinder`] term for the ordinary case where the two agree. That is
/// "scatter as gather": the maths of throwing paint, computed by asking each
/// destination who might have thrown at it.
///
/// *An uncertain pixel froze.* Scaling the streak by confidence sent uncertain
/// pixels to zero blur, which in a moving frame reads as a hole of frozen detail
/// — worse than a wrong direction. Now confidence *blends* between this pixel's
/// own vector and the neighbourhood's dominant one at [`MB_DOM_TEMPER`] length,
/// so low confidence borrows rather than freezes. Zero blur survives in exactly
/// one place: where the tile itself is still, in which case both terms are zero
/// and every tap lands on the pixel, so with `mix == 1.0` the result is the
/// bit-exact input — as it also is for `shutter_frac == 0.0`.
///
/// `samples` is the *cap* on taps, not the count: the count adapts to the
/// streak (§4's `S = ceil(‖v‖ / 2)`), so a barely-moving pixel does not pay for
/// 32 taps landing on top of each other. `quality` halves that spacing and
/// re-samples the field partway along each own-direction tap, which bends the
/// trail around a rotating object. Fixed tap order for determinism (§2.4);
/// edges clamp (the shared [`bilinear`] rule) so a full-frame smear never
/// darkens the border. `view` selects the output; the diagnostic views ignore
/// `mix` — they show the field itself.
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
    quality: MbQuality,
) {
    let original = rgba.to_vec();
    let cap = samples.max(1);
    let spacing = quality.tap_spacing();
    let curved = quality.curved();
    let (tiles, tiles_x, tiles_y) = motion_blur_tiles(u, v, conf, w, h);
    // The confidence blend, in one place because it is applied twice: once for
    // this pixel, and once at every tap position to learn that sample's reach.
    // `lend` is the *borrowed* motion (smooth, consensus-driven), never the
    // neighbour-max — see [`tile_bilinear`] for why those must not be the same
    // number.
    let blended = |uu: f32, vv: f32, c: f32, lend: (f32, f32)| {
        let c = c.clamp(0.0, 1.0);
        let borrow = MB_DOM_TEMPER * (1.0 - c);
        (
            lend.0 * shutter_frac * borrow + uu * shutter_frac * c,
            lend.1 * shutter_frac * borrow + vv * shutter_frac * c,
        )
    };
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let i = idx * 4;
            // Two summaries of the neighbourhood, for two different questions:
            // the fastest thing near me (where a smear could arrive from), and
            // what the neighbourhood agrees it is doing (what to borrow).
            let dom = neighbour_max(&tiles, tiles_x, tiles_y, x / MB_TILE, y / MB_TILE);
            let lend = tile_bilinear(&tiles, tiles_x, tiles_y, x, y);
            let out: [f32; 4] = match view {
                MbView::Rendered => {
                    let pos = (x as f32 + 0.5, y as f32 + 0.5);
                    let sv = blended(u[idx], v[idx], conf[idx], lend);
                    let dom_s = (dom.0 * shutter_frac, dom.1 * shutter_frac);
                    let len_sv = (sv.0 * sv.0 + sv.1 * sv.1).sqrt();
                    let len_dom = (dom_s.0 * dom_s.0 + dom_s.1 * dom_s.1).sqrt();
                    // Adaptive taps (§4): enough to keep them `spacing` apart
                    // over whichever of the two directions reaches furthest,
                    // never more than the user's cap.
                    let n = ((len_sv.max(len_dom) / spacing).ceil() as i32).clamp(1, cap);
                    let nf = n as f32;
                    let mut acc = [0.0f32; 4];
                    let mut wsum = 0.0f32;
                    for k in 0..n {
                        let t = (k as f32 + 0.5) / nf - 0.5;
                        // Guertin's two directions per tile, alternating: the
                        // neighbourhood's dominant sweep, then this pixel's own.
                        let dir = if k % 2 == 0 {
                            dom_s
                        } else if curved {
                            // Curved trail: re-read the field halfway along and
                            // steer by what is there (§4's destination-flow
                            // fixed point, per tap). Only the own-direction taps
                            // bend — the dominant sweep is one direction by
                            // construction.
                            let (mx, my) = (pos.0 + 0.5 * t * sv.0, pos.1 + 0.5 * t * sv.1);
                            let (mu, mv) = bilinear_uv(u, v, w, h, mx, my);
                            let mc = bilinear_scalar(conf, w, h, mx, my);
                            blended(mu, mv, mc, lend)
                        } else {
                            sv
                        };
                        let (ox, oy) = (t * dir.0, t * dir.1);
                        let d = (ox * ox + oy * oy).sqrt();
                        let (sx, sy) = (pos.0 + ox, pos.1 + oy);
                        // What the sample found there is moving by — the term
                        // that lets a fast object reach out over a still one.
                        let (tu, tv) = bilinear_uv(u, v, w, h, sx, sy);
                        let tc = bilinear_scalar(conf, w, h, sx, sy);
                        let tap = blended(tu, tv, tc, lend);
                        let len_tap = (tap.0 * tap.0 + tap.1 * tap.1).sqrt();
                        let wt = mb_cone(d, len_tap)
                            + mb_cone(d, len_sv)
                            + 2.0 * mb_cylinder(d, len_tap) * mb_cylinder(d, len_sv);
                        let s = bilinear(&original, w, h, sx, sy);
                        for cc in 0..4 {
                            acc[cc] += s[cc] * wt;
                        }
                        wsum += wt;
                    }
                    let mut o = [0.0f32; 4];
                    for cc in 0..4 {
                        let vv = acc[cc] / wsum;
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
                MbView::TileMax => {
                    // The *borrowed* field, not the neighbour-max: this is what
                    // an uncertain pixel is actually steered by, so a picture
                    // that looks wrong and this view looking wrong are the same
                    // fact. On Motion vectors' exact scale, so flipping between
                    // the two shows where the engine stopped trusting the pixel
                    // and started trusting its surroundings.
                    let k = 1.0 / 32.0;
                    [
                        (0.5 + lend.0 * k).clamp(0.0, 1.0),
                        (0.5 + lend.1 * k).clamp(0.0, 1.0),
                        0.5,
                        1.0,
                    ]
                }
            };
            rgba[i..i + 4].copy_from_slice(&out);
        }
    }
}

/// Clamp-addressed bilinear sample of a single-channel field, the exact
/// arithmetic order [`bilinear`] uses so the WGSL matches op-for-op. Used for
/// the confidence channel beside [`bilinear_uv`]'s vectors.
fn bilinear_scalar(a: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> f32 {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let xi = x0 as i32;
    let yi = y0 as i32;
    let at = |cx: i32, cy: i32| {
        let cx = cx.clamp(0, w as i32 - 1) as u32;
        let cy = cy.clamp(0, h as i32 - 1) as u32;
        a[(cy * w + cx) as usize]
    };
    let top = at(xi, yi) * (1.0 - tx) + at(xi + 1, yi) * tx;
    let bottom = at(xi, yi + 1) * (1.0 - tx) + at(xi + 1, yi + 1) * tx;
    top * (1.0 - ty) + bottom * ty
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

/// The generic Matte strength semantic (K-395), the CPU twin of
/// `fx_matte_mix.wgsl`.
///
/// **In plain terms.** Every effect can be handed a second picture — a matte —
/// whose brightness says *how much of the effect* each pixel gets. White means
/// the effect applies in full, black means the pixel is left as it came in, grey
/// means part way. This is that dissolve, and it is one function because it is
/// the same dissolve for all thirty-odd effects: the effect itself never learns
/// about the matte.
///
/// `processed` is the effect's output (in place, and the result), `input` the
/// picture the effect was given, `matte` the driving picture at the same raster.
/// The strength is the **premultiplied** Rec. 709 luma of the matte, clamped to
/// 0..1 and then inverted if asked — the flare reads a matte's luma the same
/// way (K-257), and unpremultiplying would make a half-transparent white matte
/// drive harder than it looks.
///
/// The lerp is spelled `a·(1 − k) + b·k`, which is WGSL's own definition of
/// `mix`, so the two paths associate their arithmetic identically: `k = 1` is
/// exactly the effect's output and `k = 0` exactly its input, on both.
///
/// A shorter `matte` than the picture leaves the remaining pixels at the
/// effect's output — degrade, never fault (14-ENGINEERING-RULES §4).
pub fn matte_mix(processed: &mut [f32], input: &[f32], matte: &[f32], invert: bool) {
    for i in (0..processed.len().min(input.len()).min(matte.len())).step_by(4) {
        let luma = matte[i] * LUMA[0] + matte[i + 1] * LUMA[1] + matte[i + 2] * LUMA[2];
        let k = luma.clamp(0.0, 1.0);
        let k = if invert { 1.0 - k } else { k };
        for c in 0..4 {
            processed[i + c] = input[i + c] * (1.0 - k) + processed[i + c] * k;
        }
    }
}

/// The matte's Channel pick and Invert, applied once at the seam (K-425,
/// docs/08 §2.6) — the CPU twin of `fx_matte_prepare.wgsl`.
///
/// **In plain terms.** Every kernel that reads a matte reads its luma, and the
/// dissolve does too. Rather than teach each of them which channel the user
/// chose and whether to flip it, the seam rewrites the matte *once* into a grey
/// picture whose red, green and blue are all the chosen channel — clamped to
/// 0..1, flipped if Invert is on — with alpha 1. Everything downstream then
/// reads luma of that grey and gets the chosen channel back, and Invert is
/// applied in exactly one place.
///
/// `channel` is a [`CHANNEL_OPTIONS`](super::CHANNEL_OPTIONS) index read
/// through [`channel_of`]: Luminance is the premultiplied Rec. 709 luma the
/// kernels have always read, the colour channels are the raw premultiplied
/// values, and Alpha is the coverage. The seam skips this pass altogether for
/// Luminance with Invert off ([`matte_needs_prepare`]) — not for speed alone,
/// but because the kernels already read exactly that and a pass through an
/// fp16 texture would requantise what they read, and K-258's byte-for-byte
/// promise is a promise about bytes.
pub fn matte_prepare(matte: &mut [f32], channel: u32, invert: bool) {
    for px in matte.chunks_exact_mut(4) {
        let k = channel_of(px, channel).clamp(0.0, 1.0);
        let k = if invert { 1.0 - k } else { k };
        px[0] = k;
        px[1] = k;
        px[2] = k;
        px[3] = 1.0;
    }
}

/// Whether [`matte_prepare`] would change what a kernel reads: a matte read by
/// Luminance with Invert off is what every kernel reads already, so the seam
/// runs no pass (K-258). One predicate for both render paths.
#[must_use]
pub fn matte_needs_prepare(channel: u32, invert: bool) -> bool {
    channel != 0 || invert
}

/// The encoded-domain half of [`blend_pixel`]: the compositor's sRGB curve on
/// one clamped channel, the same expression `composite.wgsl` and
/// `fx_blend_mix.wgsl` spell.
fn blend_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

fn blend_decode(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// One channel of the W3C separable blends, `s` the effect's output and `d` its
/// input, both in the encoded domain, by [`BlendMode::ALL`](crate::model::
/// BlendMode::ALL) index.
fn blend_separable(mode: u32, s: f32, d: f32) -> f32 {
    let colour_burn = |s: f32, d: f32| {
        if d >= 1.0 {
            1.0
        } else if s <= 0.0 {
            0.0
        } else {
            1.0 - ((1.0 - d) / s).min(1.0)
        }
    };
    let colour_dodge = |s: f32, d: f32| {
        if d <= 0.0 {
            0.0
        } else if s >= 1.0 {
            1.0
        } else {
            (d / (1.0 - s)).min(1.0)
        }
    };
    let hard_light = |s: f32, d: f32| {
        if s <= 0.5 {
            2.0 * s * d
        } else {
            1.0 - 2.0 * (1.0 - s) * (1.0 - d)
        }
    };
    let vivid = |s: f32, d: f32| {
        if s <= 0.5 {
            colour_burn(2.0 * s, d)
        } else {
            colour_dodge(2.0 * s - 1.0, d)
        }
    };
    match mode {
        // Colour burn.
        3 => colour_burn(s, d),
        // Linear burn.
        4 => (s + d - 1.0).clamp(0.0, 1.0),
        // Screen.
        8 => s + d - s * d,
        // Colour dodge.
        9 => colour_dodge(s, d),
        // Overlay: hard light with the backdrop as the switch.
        11 => hard_light(d, s),
        // Soft light (W3C).
        12 => {
            let dd = if d <= 0.25 {
                ((16.0 * d - 12.0) * d + 4.0) * d
            } else {
                d.sqrt()
            };
            if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                d + (2.0 * s - 1.0) * (dd - d)
            }
        }
        // Hard light.
        13 => hard_light(s, d),
        // Linear light.
        14 => (d + 2.0 * s - 1.0).clamp(0.0, 1.0),
        // Vivid light.
        15 => vivid(s, d),
        // Pin light.
        16 => {
            if s <= 0.5 {
                d.min(2.0 * s)
            } else {
                d.max(2.0 * s - 1.0)
            }
        }
        // Hard mix.
        17 => {
            if vivid(s, d) >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        // Difference.
        18 => (s - d).abs(),
        // Exclusion.
        19 => s + d - 2.0 * s * d,
        // Divide.
        21 => (d / s.max(1e-6)).clamp(0.0, 1.0),
        _ => s,
    }
}

/// The non-separable (HSL) helpers, W3C compositing §non-separable, on encoded
/// RGB — the same arithmetic `composite.wgsl` uses for a layer's Mode.
fn blend_lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn blend_clip(c: [f32; 3]) -> [f32; 3] {
    let l = blend_lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut r = c;
    if n < 0.0 {
        for v in &mut r {
            *v = l + (*v - l) * (l / (l - n).max(1e-6));
        }
    }
    if x > 1.0 {
        for v in &mut r {
            *v = l + (*v - l) * ((1.0 - l) / (x - l).max(1e-6));
        }
    }
    r
}

fn blend_set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - blend_lum(c);
    blend_clip([c[0] + d, c[1] + d, c[2] + d])
}

fn blend_sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn blend_set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mn = c[0].min(c[1]).min(c[2]);
    let mx = c[0].max(c[1]).max(c[2]);
    if mx > mn {
        let k = s / (mx - mn).max(1e-6);
        [(c[0] - mn) * k, (c[1] - mn) * k, (c[2] - mn) * k]
    } else {
        [0.0; 3]
    }
}

/// One pixel of the effect Blend (K-425): the effect's output `s` combined
/// with its input `d` by [`BlendMode::ALL`](crate::model::BlendMode::ALL)
/// index `mode`, both premultiplied linear RGBA. The CPU twin of
/// `fx_blend_mix.wgsl`'s `blend_pixel`, in the same arithmetic order.
///
/// The domains follow the compositor's (docs/06 §blend domains), so an
/// effect's Blend looks like the same word on a layer: Add, Multiply, Lighten,
/// Darken and Subtract run per channel in linear light; everything else
/// encodes both sides to sRGB, applies the W3C formula, and decodes. Alpha is
/// the effect's own — the blend is about colour, and an effect that changed
/// coverage has said so in its alpha already. Normal is `s`, untouched, and
/// the seam never calls this for it.
pub fn blend_pixel(mode: u32, d: [f32; 4], s: [f32; 4]) -> [f32; 4] {
    let mut o = [0.0f32; 4];
    o[3] = s[3];
    match mode {
        0 => o = s,
        // Linear, per channel: Add, Multiply, Lighten, Darken, Subtract.
        6 => {
            for c in 0..3 {
                o[c] = d[c] + s[c];
            }
        }
        2 => {
            for c in 0..3 {
                o[c] = d[c] * s[c];
            }
        }
        7 => {
            for c in 0..3 {
                o[c] = d[c].max(s[c]);
            }
        }
        1 => {
            for c in 0..3 {
                o[c] = d[c].min(s[c]);
            }
        }
        20 => {
            for c in 0..3 {
                o[c] = (d[c] - s[c]).max(0.0);
            }
        }
        // The encoded (perceptual) set.
        m => {
            let se = [
                blend_encode(s[0].clamp(0.0, 1.0)),
                blend_encode(s[1].clamp(0.0, 1.0)),
                blend_encode(s[2].clamp(0.0, 1.0)),
            ];
            let de = [
                blend_encode(d[0].clamp(0.0, 1.0)),
                blend_encode(d[1].clamp(0.0, 1.0)),
                blend_encode(d[2].clamp(0.0, 1.0)),
            ];
            let b = match m {
                // Darker colour / Lighter colour: whole-pixel picks by luma.
                5 => {
                    if blend_lum(se) < blend_lum(de) {
                        se
                    } else {
                        de
                    }
                }
                10 => {
                    if blend_lum(se) > blend_lum(de) {
                        se
                    } else {
                        de
                    }
                }
                // Hue, Saturation, Colour, Luminosity.
                22 => blend_set_lum(blend_set_sat(se, blend_sat(de)), blend_lum(de)),
                23 => blend_set_lum(blend_set_sat(de, blend_sat(se)), blend_lum(de)),
                24 => blend_set_lum(se, blend_lum(de)),
                25 => blend_set_lum(de, blend_lum(se)),
                _ => [
                    blend_separable(m, se[0], de[0]),
                    blend_separable(m, se[1], de[1]),
                    blend_separable(m, se[2], de[2]),
                ],
            };
            for c in 0..3 {
                o[c] = blend_decode(b[c]);
            }
        }
    }
    o
}

/// The effect Blend and Mix, once at the seam (K-425, docs/08 §1.5) — the CPU
/// twin of `fx_blend_mix.wgsl`. `processed` is the kernel's output **at Mix
/// 100** (in place, and the result), `input` the picture it was given, `mode`
/// a [`BlendMode::ALL`](crate::model::BlendMode::ALL) index and `mix` the
/// effect's own Mix as 0..1. Each pixel becomes
/// `input·(1 − mix) + blend(input, processed)·mix`, the lerp spelled as WGSL's
/// `mix` so the two paths agree. Never called for Normal.
pub fn blend_mix(processed: &mut [f32], input: &[f32], mode: u32, mix: f32) {
    for i in (0..processed.len().min(input.len())).step_by(4) {
        let d = [input[i], input[i + 1], input[i + 2], input[i + 3]];
        let s = [
            processed[i],
            processed[i + 1],
            processed[i + 2],
            processed[i + 3],
        ];
        let b = blend_pixel(mode, d, s);
        for c in 0..4 {
            processed[i + c] = d[c] * (1.0 - mix) + b[c] * mix;
        }
    }
}

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
    rgb_split_matted(rgba, w, h, amount_px, angle_deg, scale, tints, mix, &[]);
}

/// [`rgb_split`] driven by a matte (K-395, K-427): each pixel's matte strength
/// scales **Amount** — the offset vector — before the three taps are read. An
/// empty matte is the unmatted path to the byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn rgb_split_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    angle_deg: f32,
    scale: [f32; 3],
    tints: [[f32; 3]; 3],
    mix: f32,
    matte: &[f32],
) {
    let original = rgba.to_vec();
    let (dx, dy) = super::rgb_split_offset(amount_px, angle_deg);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = matte_strength(matte, i);
            let (dx, dy) = (dx * k, dy * k);
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
    spectral_split_matted(
        rgba,
        w,
        h,
        amount_px,
        angle_deg,
        radial,
        samples,
        tints,
        mix,
        &[],
    );
}

/// [`spectral_split`] driven by a matte (K-395, K-427): each pixel's matte
/// strength scales **Amount** — the offset the taps spread across, linear or
/// radial — before they are read. An empty matte is the unmatted path to the
/// byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn spectral_split_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    angle_deg: f32,
    radial: bool,
    samples: i32,
    tints: [[f32; 3]; 3],
    mix: f32,
    matte: &[f32],
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
            let m = matte_strength(matte, i);
            let (ox, oy) = (ox * m, oy * m);
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
    chromatic_aberration_matted(rgba, w, h, amount_px, tints, mix, &[]);
}

/// [`chromatic_aberration`] driven by a matte (K-395, K-427): each pixel's
/// matte strength scales **Amount** — the radial offset — before the three taps
/// are read. An empty matte is the unmatted path to the byte (K-258).
pub fn chromatic_aberration_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount_px: f32,
    tints: [[f32; 3]; 3],
    mix: f32,
    matte: &[f32],
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
            let m = matte_strength(matte, i);
            let (ox, oy) = ((pos.0 - cx) * k * m, (pos.1 - cy) * k * m);
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
    sharpen_matted(
        rgba,
        w,
        h,
        amount,
        radius_px,
        threshold,
        luma_only,
        mix,
        &[],
    );
}

/// [`sharpen`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength scales its **Amount** — less detail added back, not full detail
/// faded back. The unsharp gaussian is not affected. An empty matte is the
/// unmatted path to the byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn sharpen_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount: f32,
    radius_px: f32,
    threshold: f32,
    luma_only: bool,
    mix: f32,
    matte: &[f32],
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
        let amount = amount * matte_strength(matte, i);
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
    sharpen_simple_matted(rgba, w, h, amount, radius, mix, &[]);
}

/// [`sharpen_simple`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales its **Amount** before the high-pass is added. An
/// empty matte is the unmatted path to the byte (K-258).
pub fn sharpen_simple_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    amount: f32,
    radius: f32,
    mix: f32,
    matte: &[f32],
) {
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
            let amount = amount * matte_strength(matte, i);
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
    blur_directional_matted(rgba, w, h, length_px, angle_deg, edge, mix, &[]);
}

/// [`blur_directional`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales its **Length**, so the streak is genuinely shorter
/// where the matte is grey — the same evenly spaced taps, packed closer. The
/// tap count stays the host's (from the full Length), so the two paths sample
/// the same number of times whatever the matte says. An empty matte is the
/// unmatted path to the byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn blur_directional_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    length_px: f32,
    angle_deg: f32,
    edge: u32,
    mix: f32,
    matte: &[f32],
) {
    let original = rgba.to_vec();
    let (dx, dy) = super::rgb_split_offset(1.0, angle_deg); // unit vector
    let n = dir_blur_taps(length_px);
    let nf = n as f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let length_px = length_px * matte_strength(matte, i);
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
    blur_radial_matted(rgba, w, h, centre_frac, amount_px, spin, edge, mix, &[]);
}

/// [`blur_radial`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales its **Amount**, so the sweep is genuinely shorter
/// where the matte is grey. The tap count stays the host's, as the
/// directional blur's does. An empty matte is the unmatted path to the byte
/// (K-258).
#[allow(clippy::too_many_arguments)]
pub fn blur_radial_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    centre_frac: [f32; 2],
    amount_px: f32,
    spin: bool,
    edge: u32,
    mix: f32,
    matte: &[f32],
) {
    let original = rgba.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let centre = (centre_frac[0] * fw, centre_frac[1] * fh);
    let diag = (fw * fw + fh * fh).sqrt();
    let k_full = if diag > 0.0 {
        amount_px / (0.5 * diag)
    } else {
        0.0
    };
    let n = radial_blur_taps(amount_px);
    let nf = n as f32;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = k_full * matte_strength(matte, i);
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

/// **The Matte-driven Gaussian blur** (K-395, docs/08 §2.6) — the §1.6 oracle
/// for `fx_blur.wgsl`'s matted path, op-for-op.
///
/// # In plain terms
///
/// Gaussian blur is one of the effects that claim the matte inside their own
/// maths instead of taking the generic strength dissolve, and this is what it
/// does with it: each pixel's own matte luma **scales the radius**, so white
/// blurs at the full Radius, mid-grey at half of it, and black not at all.
///
/// That is a picture the generic dissolve cannot make, and the difference is
/// worth being clear about. Dissolving a 40 px blur to 50 % gives a sharp image
/// with a wide soft halo laid over it — every pixel still gathered from 40 px
/// away. Halving the radius gives a 20 px blur: genuinely less soft, gathering
/// from half as far. On a face-shaped matte the first reads as a veil, the
/// second as a lens racking focus.
///
/// Both separable passes read the **destination** pixel's matte, which is what
/// makes the two halves agree about how wide this pixel's kernel is.
///
/// An empty `matte` is delegated straight to [`blur_gaussian`] — not "the same
/// arithmetic with k = 1", but the identical function, so an unset Matte row is
/// byte-for-byte the blur that shipped before K-395 (K-258). The matted path
/// cannot precompute one weight table (every pixel's is different), so it
/// accumulates unnormalised and divides at the end, exactly as the WGSL does.
#[allow(clippy::too_many_arguments)]
pub fn blur_gaussian_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    radius_px: f32,
    edge: u32,
    mix: f32,
    matte: &[f32],
) {
    if matte.is_empty() {
        return blur_gaussian(rgba, w, h, radius_px, edge, mix);
    }
    let (wi, hi) = (w as i64, h as i64);
    let original = rgba.to_vec();
    let mut pass = vec![0.0f32; rgba.len()];
    // One pixel's kernel, built from its own matte: the radius scaled by the
    // clamped premultiplied luma (the reading `matte_mix` uses), then the WGSL's
    // `ceil` and `max(σ, 1e-3)` in that order. A matte shorter than the picture
    // leaves the remaining pixels at the full radius — degrade, never fault
    // (14-ENGINEERING-RULES §4).
    let kernel = |d: usize| -> (i64, f32) {
        let rad = radius_px * matte_strength(matte, d);
        (rad.ceil() as i64, (rad * 0.5).max(1e-3))
    };
    // Horizontal.
    for y in 0..hi {
        for x in 0..wi {
            let d = ((y * wi + x) * 4) as usize;
            let (r, sigma) = kernel(d);
            let mut acc = [0.0f32; 4];
            if r == 0 {
                acc.copy_from_slice(&original[d..d + 4]);
            } else {
                let mut wsum = 0.0f32;
                for i in -r..=r {
                    let dd = i as f32 / sigma.max(1e-3);
                    let wt = (-0.5 * dd * dd).exp();
                    wsum += wt;
                    if let Some(sx) = edge_index(x + i, wi, edge) {
                        let s = ((y * wi + sx) * 4) as usize;
                        for c in 0..4 {
                            acc[c] += original[s + c] * wt;
                        }
                    }
                }
                for v in &mut acc {
                    *v /= wsum;
                }
            }
            pass[d..d + 4].copy_from_slice(&acc);
        }
    }
    // Vertical, blending the host Mix against the untouched input.
    for y in 0..hi {
        for x in 0..wi {
            let d = ((y * wi + x) * 4) as usize;
            let (r, sigma) = kernel(d);
            let mut acc = [0.0f32; 4];
            if r == 0 {
                acc.copy_from_slice(&pass[d..d + 4]);
            } else {
                let mut wsum = 0.0f32;
                for i in -r..=r {
                    let dd = i as f32 / sigma.max(1e-3);
                    let wt = (-0.5 * dd * dd).exp();
                    wsum += wt;
                    if let Some(sy) = edge_index(y + i, hi, edge) {
                        let s = ((sy * wi + x) * 4) as usize;
                        for c in 0..4 {
                            acc[c] += pass[s + c] * wt;
                        }
                    }
                }
                for v in &mut acc {
                    *v /= wsum;
                }
            }
            for c in 0..4 {
                rgba[d + c] = original[d + c] * (1.0 - mix) + acc[c] * mix;
            }
        }
    }
}

/// One resolved Channel blur (docs/08 §3.45) is four radii, an edge code and a
/// mix, so it takes no struct — see [`channel_blur`].
///
/// Channel blur: the separable gaussian of [`blur_gaussian`] with a radius per
/// channel.
///
/// **In plain terms.** Each of red, green, blue and alpha is blurred by its own
/// amount. A channel whose radius is zero is copied through untouched, which is
/// what makes the common case (one channel softened, three left alone) cost one
/// channel's gather rather than four.
///
/// The weights are built **in the loop and normalised at the end** rather than
/// precomputed as one table, because the four channels no longer share a table —
/// the same arrangement [`blur_gaussian_matted`] uses, and the arrangement the
/// WGSL twin mirrors op-for-op (§1.6).
pub fn channel_blur(rgba: &mut [f32], w: u32, h: u32, radii: [f32; 4], edge: u32, mix: f32) {
    channel_blur_matted(rgba, w, h, radii, edge, mix, &[]);
}

/// [`channel_blur`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales **all four radii**, so every channel's blur is
/// genuinely narrower where the matte is grey — the Gaussian blur's own
/// override, four times over. Both passes read the destination pixel's matte,
/// as [`blur_gaussian_matted`] does. An empty matte is the unmatted path to
/// the byte (K-258): the radius is `radius · 1`, and the tap count and σ fall
/// out of it exactly as the host computed them.
#[allow(clippy::too_many_arguments)]
pub fn channel_blur_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    radii: [f32; 4],
    edge: u32,
    mix: f32,
    matte: &[f32],
) {
    let (wi, hi) = (w as i64, h as i64);
    let original = rgba.to_vec();
    let mut pass = vec![0.0f32; rgba.len()];
    // Per channel at this pixel: how far the gather reaches, and the σ it
    // falls off over — the WGSL's `ceil` and `max(σ, 1e-3)` in that order.
    let kernel = |d: usize, c: usize| -> (i64, f32) {
        let rad = radii[c] * matte_strength(matte, d);
        (rad.ceil() as i64, (rad * 0.5).max(1e-3))
    };
    // Horizontal, then vertical: the same loop over a different axis.
    for y in 0..hi {
        for x in 0..wi {
            let d = ((y * wi + x) * 4) as usize;
            for c in 0..4 {
                let (r, sigma) = kernel(d, c);
                if r == 0 {
                    pass[d + c] = original[d + c];
                    continue;
                }
                let (mut acc, mut wsum) = (0.0f32, 0.0f32);
                for i in -r..=r {
                    let dd = i as f32 / sigma;
                    let wt = (-0.5 * dd * dd).exp();
                    wsum += wt;
                    if let Some(sx) = edge_index(x + i, wi, edge) {
                        acc += original[(((y * wi + sx) * 4) as usize) + c] * wt;
                    }
                }
                pass[d + c] = acc / wsum;
            }
        }
    }
    for y in 0..hi {
        for x in 0..wi {
            let d = ((y * wi + x) * 4) as usize;
            for c in 0..4 {
                let (r, sigma) = kernel(d, c);
                let v = if r == 0 {
                    pass[d + c]
                } else {
                    let (mut acc, mut wsum) = (0.0f32, 0.0f32);
                    for i in -r..=r {
                        let dd = i as f32 / sigma;
                        let wt = (-0.5 * dd * dd).exp();
                        wsum += wt;
                        if let Some(sy) = edge_index(y + i, hi, edge) {
                            acc += pass[(((sy * wi + x) * 4) as usize) + c] * wt;
                        }
                    }
                    acc / wsum
                };
                rgba[d + c] = original[d + c] * (1.0 - mix) + v * mix;
            }
        }
    }
}

/// One resolved Drop shadow (docs/08 §3.43), reduced to what both paths read.
/// The direction's sine and cosine are already spent into `offset` host-side
/// (`DropShadow::packed`), so neither render path runs trigonometry (§1.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropShadowParams {
    /// Scene-linear RGB; the shadow's coverage supplies the rest.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// Where the shadow sits relative to the shape, raster pixels.
    pub offset: [f32; 2],
    /// The gaussian half-width the shape is softened by, raster pixels.
    pub softness_px: f32,
    /// Draw the shadow alone, without the layer that cast it.
    pub shadow_only: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Drop shadow (docs/08 §3.43) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Blur the layer's shape, read that blurred shape at a
/// shifted position, paint it in one colour, and put it *underneath* the layer.
///
/// **The blur is taken where the shape stands, not where the shadow goes.** A
/// translation and a convolution commute, so softening first and reading at the
/// offset is exactly the same picture as offsetting first and softening — for
/// one gaussian instead of a gaussian plus a resample.
///
/// The shadow's edges fall away into transparency (edge policy 0), which is the
/// only honest reading: a shape touching the frame border casts a shadow that
/// leaves the frame, and repeating the border pixel outward would smear it.
pub fn drop_shadow(rgba: &mut [f32], w: u32, h: u32, p: &DropShadowParams) {
    let original = rgba.to_vec();
    // The shared §3.8 gaussian, on the whole picture: only the alpha is read
    // back out of it, and blurring four channels costs exactly what the WGSL
    // pass this mirrors costs, since it is the same kernel.
    let mut soft = original.clone();
    blur_gaussian(&mut soft, w, h, p.softness_px, 0, 1.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let k = bilinear_edge(
                &soft,
                w,
                h,
                x as f32 + 0.5 - p.offset[0],
                y as f32 + 0.5 - p.offset[1],
                0,
            )[3] * p.opacity;
            let shadow = [p.colour[0] * k, p.colour[1] * k, p.colour[2] * k, k];
            let src_a = original[d + 3];
            for c in 0..4 {
                // Source OVER shadow, premultiplied — the shadow is BELOW,
                // which is the whole reason this is an effect and not a
                // duplicated layer.
                let over = if p.shadow_only {
                    shadow[c]
                } else {
                    original[d + c] + shadow[c] * (1.0 - src_a)
                };
                rgba[d + c] = original[d + c] * (1.0 - p.mix) + over * p.mix;
            }
        }
    }
}

/// One resolved Roughen edges (docs/08 §3.57), reduced to what both paths read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoughenEdgesParams {
    /// The noise field's shape, seed and loop — §3.37's core, shared.
    pub field: FractalField,
    /// `1 ÷ Scale`, raster pixels.
    pub inv_scale: f32,
    /// The field's origin, raster pixels.
    pub offset: [f32; 2],
    /// The field's depth coordinate (Evolution ÷ 360, folded into the cycle).
    pub z: f32,
    /// Border, raster pixels: the gaussian radius of the first pass, and so the
    /// width of the band the second one works in.
    pub border_px: f32,
    /// False at Border 0: the exact identity, rather than a re-cut of the
    /// picture's own antialiasing.
    pub active: bool,
    /// Fractal influence ÷ 100: how far the noise shifts the cut.
    pub influence: f32,
    /// Half the width of the cut, in alpha, floored so the smoothstep never
    /// divides by zero.
    pub half_width: f32,
    /// Scene-linear RGB the chewed band is painted in, when
    /// [`colour_on`](Self::colour_on) says so.
    pub colour: [f32; 3],
    /// 1 to paint the band, 0 to leave it. A float rather than a bool so the
    /// kernel multiplies instead of branching.
    pub colour_on: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The smoothstep both paths run, written out rather than borrowed, so the CPU
/// reference and WGSL's builtin cannot differ on the clamp or the polynomial.
fn smoothstep_between(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Roughen edges (docs/08 §3.57) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Blur the picture by Border, which turns its alpha into a
/// soft ramp whose half-way line is exactly where the original edge was; then
/// cut that ramp again at a threshold the fractal field wobbles per pixel. What
/// comes back is the same shape with its outline chewed.
///
/// The colour is carried straight (§2.2): a pixel keeps its own colour and gets
/// a new coverage, and a pixel the chewing *grew* into — one that had no colour
/// of its own — borrows the blurred neighbourhood's, because premultiplied black
/// is what "no colour" looks like and a grown edge painted with it would read as
/// soot.
pub fn roughen_edges(rgba: &mut [f32], w: u32, h: u32, p: &RoughenEdgesParams) {
    if !p.active {
        return;
    }
    let original = rgba.to_vec();
    // The shared §3.8 gaussian, on the whole picture — the same reuse §3.43
    // makes, and here the blurred alpha *is* the distance field.
    let mut soft = original.clone();
    blur_gaussian(&mut soft, w, h, p.border_px, 0, 1.0);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let bar = soft[i + 3];
            let n = fractal(
                &p.field,
                (px - p.offset[0]) * p.inv_scale,
                (py - p.offset[1]) * p.inv_scale,
                p.z,
            );
            // 1 on the outline, 0 well inside or well outside it. It weights
            // the noise as well as the edge colour, and that is what confines
            // the chewing to the band: deep inside the shape the shift is
            // exactly zero, so no amount of Fractal influence can punch a hole
            // in the middle of a solid layer (docs/08 §3.57 decision 3).
            let band = 1.0 - (2.0 * bar - 1.0).abs();
            let t = bar + n * p.influence * 0.5 * band - 0.5;
            let k = smoothstep_between(-p.half_width, p.half_width, t);
            let a = original[i + 3];
            let mut col = if a > 1e-4 {
                [original[i] / a, original[i + 1] / a, original[i + 2] / a]
            } else {
                let sa = soft[i + 3].max(1e-4);
                [soft[i] / sa, soft[i + 1] / sa, soft[i + 2] / sa]
            };
            // The same band paints the chewed border, and nothing else.
            let paint = band * p.colour_on;
            for (c, edge) in col.iter_mut().zip(p.colour) {
                *c += (edge - *c) * paint;
            }
            for c in 0..3 {
                rgba[i + c] = original[i + c] * (1.0 - p.mix) + col[c] * k * p.mix;
            }
            rgba[i + 3] = original[i + 3] * (1.0 - p.mix) + k * p.mix;
        }
    }
}

/// Set matte (docs/08 §3.44) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** The chosen channel of `matte` becomes this picture's
/// alpha. The colour is carried across *straight* (§2.2): the pixel is
/// unpremultiplied, given its new coverage, and re-premultiplied, so changing
/// how much of a pixel there is does not also change what colour it is.
///
/// `matte` is the referenced layer's picture at this raster, RGBA rather than a
/// single channel, because which channel carries the shape is one of this
/// effect's controls. An empty slice is the unbound case — the labelled no-op
/// every layer-input effect follows — and leaves the picture untouched.
pub fn set_matte(
    rgba: &mut [f32],
    matte: &[f32],
    channel: u32,
    invert: bool,
    combine: bool,
    mix: f32,
) {
    if matte.is_empty() {
        return;
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let d = i * 4;
        let Some(m) = matte.get(d..d + 4) else {
            // A matte shorter than the picture leaves the rest alone — degrade,
            // never fault (14-ENGINEERING-RULES §4).
            break;
        };
        let mut k = channel_of(m, channel);
        if invert {
            k = 1.0 - k;
        }
        let a = if combine { px[3] * k } else { k };
        let straight = unpremult(px);
        for c in 0..3 {
            px[c] = px[c] * (1.0 - mix) + straight[c] * a * mix;
        }
        px[3] = px[3] * (1.0 - mix) + a * mix;
    }
}

/// One resolved Linear wipe (docs/08 §3.46), reduced to what both paths read.
/// The frame's extent along the sweep is deliberately absent: it is a function
/// of the raster, which the kernel knows and the host does not (§3.39).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearWipeParams {
    /// Where the wipe line pivots, raster pixels.
    pub centre: [f32; 2],
    /// The sweep direction, host-computed `(sin θ, −cos θ)`.
    pub normal: [f32; 2],
    /// Completion ÷ 100.
    pub completion: f32,
    /// The feather's width in raster pixels, floored above zero so the hard
    /// case is a step rather than a divide by zero.
    pub band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The kept fraction of one pixel under a Linear wipe — the expression both
/// paths evaluate, written once so [`linear_wipe`] and the WGSL twin cannot
/// disagree about it.
///
/// The edge travels half a feather *past* each end of the frame, which is what
/// makes Completion 0 keep the whole frame exactly and 100 remove it exactly. A
/// wipe that cannot fully finish is not a transition.
#[must_use]
pub fn linear_wipe_keep(px: f32, py: f32, w: f32, h: f32, p: &LinearWipeParams) -> f32 {
    let d = (px - p.centre[0]) * p.normal[0] + (py - p.centre[1]) * p.normal[1];
    let extent = 0.5 * ((w * p.normal[0]).abs() + (h * p.normal[1]).abs());
    let edge = -(extent + p.band * 0.5) + p.completion * (2.0 * extent + p.band);
    ((d - edge) / p.band + 0.5).clamp(0.0, 1.0)
}

/// Linear wipe (docs/08 §3.46) — the CPU reference and §1.6 oracle. The picture
/// is scaled by its kept fraction, all four channels, which is the
/// premultiplied form of "less of this pixel".
pub fn linear_wipe(rgba: &mut [f32], w: u32, h: u32, p: &LinearWipeParams) {
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let keep = linear_wipe_keep(x as f32 + 0.5, y as f32 + 0.5, fw, fh, p);
            // Written as `1 − mix·(1 − keep)` rather than `(1−mix) + keep·mix`
            // so a fully kept pixel scales by *exactly* 1 at any Mix: the
            // second form rounds twice and Completion 0 would stop being the
            // bit-exact identity below full Mix.
            let f = 1.0 - p.mix * (1.0 - keep);
            for c in 0..4 {
                rgba[d + c] *= f;
            }
        }
    }
}

/// One resolved Radial wipe (docs/08 §3.47), reduced to what both paths read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialWipeParams {
    /// Where the hand pivots, raster pixels.
    pub centre: [f32; 2],
    /// Start angle in radians, from straight up, clockwise.
    pub start: f32,
    /// Where the wedge's middle sits from `start`: +1 clockwise, −1
    /// anticlockwise, 0 for Both (the wedge opens symmetrically).
    pub dir: f32,
    /// Completion ÷ 100.
    pub completion: f32,
    /// The feather's width in raster pixels, measured at the arc; floored above
    /// zero.
    pub feather: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The kept fraction of one pixel under a Radial wipe — one expression for all
/// three sweep directions (docs/08 §3.47), written once so [`radial_wipe`] and
/// the WGSL twin cannot disagree about it.
///
/// The wrap into −π..π uses `floor(x + ½)` and **not** `round`: Rust rounds
/// halves away from zero, WGSL rounds them to even, and one pixel landing on the
/// wrong side of the wedge is exactly what §1.6 exists to catch.
#[must_use]
pub fn radial_wipe_keep(px: f32, py: f32, p: &RadialWipeParams) -> f32 {
    use std::f32::consts::{PI, TAU};
    let dx = px - p.centre[0];
    let dy = py - p.centre[1];
    // From straight up, clockwise, on a raster whose y grows downward.
    let phi = dy.atan2(dx) + PI * 0.5;
    let r = (dx * dx + dy * dy).sqrt();
    // A constant-width soft edge: the angle a `feather`-wide band subtends at
    // this radius. Clamped at π because near the centre it grows without bound.
    let band = (p.feather / r.max(1.0)).clamp(1e-4, PI);
    // The wedge's half-width, with a half-band lead-in at each end so
    // Completion 0 and 100 are the exact identity and the exact empty frame.
    let hw = p.completion * (PI + band) - band * 0.5;
    let mid = p.start + hw * p.dir;
    let mut delta = phi - mid;
    delta -= TAU * (delta / TAU + 0.5).floor();
    (0.5 - (hw - delta.abs()) / band).clamp(0.0, 1.0)
}

/// Radial wipe (docs/08 §3.47) — the CPU reference and §1.6 oracle.
pub fn radial_wipe(rgba: &mut [f32], w: u32, h: u32, p: &RadialWipeParams) {
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let keep = radial_wipe_keep(x as f32 + 0.5, y as f32 + 0.5, p);
            // See [`linear_wipe`] for why this form and not the other.
            let f = 1.0 - p.mix * (1.0 - keep);
            for c in 0..4 {
                rgba[d + c] *= f;
            }
        }
    }
}

/// One resolved Venetian blinds (docs/08 §3.70), reduced to what both paths
/// read. The slats' anchor is deliberately absent: they sit on the frame's own
/// middle, which the kernel knows and the host does not (§3.46's precedent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VenetianBlindsParams {
    /// The direction the slats close along, host-computed `(sin θ, −cos θ)`.
    pub normal: [f32; 2],
    /// One slat's width in raster pixels, floored at one so the fold has a
    /// period.
    pub period: f32,
    /// Completion ÷ 100.
    pub completion: f32,
    /// The feather's width in raster pixels, floored above zero.
    pub band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The kept fraction of one pixel under Venetian blinds — the expression both
/// paths evaluate, written once so [`venetian_blinds`] and the WGSL twin cannot
/// disagree about it.
///
/// It is [`linear_wipe_keep`] with the distance folded into one slat first, and
/// the same half-band lead-in at each end: the gap opens at the slat's middle
/// and reaches exactly half a feather past its edges, so Completion 0 keeps the
/// whole frame exactly and 100 removes it exactly.
///
/// The fold is `floor(x + ½)` and **not** `round`, for [`radial_wipe_keep`]'s
/// reason: Rust rounds halves away from zero and WGSL rounds them to even.
#[must_use]
pub fn venetian_blinds_keep(px: f32, py: f32, w: f32, h: f32, p: &VenetianBlindsParams) -> f32 {
    let d = (px - w * 0.5) * p.normal[0] + (py - h * 0.5) * p.normal[1];
    let u = d - p.period * (d / p.period + 0.5).floor();
    let hw = p.completion * (p.period * 0.5 + p.band) - p.band * 0.5;
    ((u.abs() - hw) / p.band + 0.5).clamp(0.0, 1.0)
}

/// Venetian blinds (docs/08 §3.70) — the CPU reference and §1.6 oracle. The
/// picture is scaled by its kept fraction, all four channels, which is the
/// premultiplied form of "less of this pixel".
pub fn venetian_blinds(rgba: &mut [f32], w: u32, h: u32, p: &VenetianBlindsParams) {
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let keep = venetian_blinds_keep(x as f32 + 0.5, y as f32 + 0.5, fw, fh, p);
            // See [`linear_wipe`] for why this form and not the other.
            let f = 1.0 - p.mix * (1.0 - keep);
            for c in 0..4 {
                rgba[d + c] *= f;
            }
        }
    }
}

/// One resolved Iris wipe (docs/08 §3.71), reduced to what both paths read.
/// The polygon is already solved into one sector here: two vertices become a
/// point on the edge and that edge's outward unit normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrisWipeParams {
    /// Where the iris opens, raster pixels.
    pub centre: [f32; 2],
    /// The sector's first vertex, in the sector's own frame (radius along +x).
    pub vertex: [f32; 2],
    /// The outward unit normal of the edge leaving that vertex.
    pub normal: [f32; 2],
    /// One sector, radians: `2π ÷ Points`.
    pub period: f32,
    /// Rotation in radians, from straight up, clockwise.
    pub rotation: f32,
    /// The feather's width in raster pixels, floored above zero.
    pub band: f32,
    /// False when Outer radius is 0: there is no polygon, so the frame passes
    /// through untouched (§3.71's fifth note).
    pub active: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The kept fraction of one pixel under an Iris wipe — the expression both paths
/// evaluate, written once so [`iris_wipe`] and the WGSL twin cannot disagree
/// about it.
///
/// The pixel's angle is folded into one sector and mirrored about that sector's
/// bisector, which reduces the whole boundary — polygon or star — to the single
/// straight edge the host solved. What comes out is a **true perpendicular
/// distance in pixels**, which is what lets Feather be a width rather than an
/// angle.
#[must_use]
pub fn iris_wipe_keep(px: f32, py: f32, p: &IrisWipeParams) -> f32 {
    use std::f32::consts::PI;
    if !p.active {
        return 1.0;
    }
    let dx = px - p.centre[0];
    let dy = py - p.centre[1];
    // From straight up, clockwise, on a raster whose y grows downward, then
    // de-rotated so the sector's first vertex sits on the +x axis.
    let phi = dy.atan2(dx) + PI * 0.5 - p.rotation;
    let r = (dx * dx + dy * dy).sqrt();
    // `floor(x + ½)`, never `round` — [`radial_wipe_keep`]'s reason.
    let a = (phi - p.period * (phi / p.period + 0.5).floor()).abs();
    let point = [r * a.cos(), r * a.sin()];
    let dist = (point[0] - p.vertex[0]) * p.normal[0] + (point[1] - p.vertex[1]) * p.normal[1];
    (dist / p.band + 0.5).clamp(0.0, 1.0)
}

/// Iris wipe (docs/08 §3.71) — the CPU reference and §1.6 oracle.
pub fn iris_wipe(rgba: &mut [f32], w: u32, h: u32, p: &IrisWipeParams) {
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let keep = iris_wipe_keep(x as f32 + 0.5, y as f32 + 0.5, p);
            // See [`linear_wipe`] for why this form and not the other.
            let f = 1.0 - p.mix * (1.0 - keep);
            for c in 0..4 {
                rgba[d + c] *= f;
            }
        }
    }
}

/// How far the Card wipe's camera stands from a card, in card half-widths
/// (docs/08 §3.72). Fixed, and deliberately so: Lumit has no 3D camera on an
/// effect, so every card is projected in its own local frame at the same viewing
/// distance whatever the grid. Exactly representable, so both paths multiply by
/// the identical number; `fx_cardwipe.wgsl` spells the same literal.
pub const CARD_VIEW_DISTANCE: f32 = 3.0;

/// One resolved Card wipe (docs/08 §3.72), reduced to what both paths read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardWipeParams {
    /// Columns then rows.
    pub grid: [i32; 2],
    /// Completion ÷ 100.
    pub completion: f32,
    /// `100 ÷ Transition width`, so the kernel multiplies.
    pub inv_width: f32,
    /// `1 − Transition width ÷ 100`.
    pub one_minus_width: f32,
    /// Which grid axis the Flip order ramp runs along: 0 columns, 1 rows.
    pub order_axis: u32,
    /// The ramp's offset — 0 forwards along that axis, 1 backwards.
    pub order_bias: f32,
    /// The ramp's slope: +1 forwards, −1 backwards.
    pub order_scale: f32,
    /// `FLIP_AXIS_OPTIONS` index: 0 horizontal, 1 vertical, 2 per card.
    pub axis: u32,
    /// `FLIP_DIRECTION_OPTIONS` index: 0 forwards, 1 backwards, 2 per card.
    pub direction: u32,
    /// Randomness ÷ 100.
    pub randomness: f32,
    /// Which shuffle this instance gets (§2.4).
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The pixel range of the card `x` falls in, out of `n` across a `len`-pixel
/// axis (docs/08 §3.72). Whole-number arithmetic throughout, for §3.65's reason
/// — a division that comes out exact puts a pixel in a different card on the two
/// paths, and here that would be a seam rather than a block.
///
/// Note the ceilings. [`mosaic_span`] takes the floor at both ends, which is
/// fine when the span is only being *sampled*; a card is drawn to its span, so
/// the two ends have to be the exact inverse of `(x·n) ÷ len` or the last pixel
/// of a card would fall outside the card it was assigned to.
#[must_use]
pub fn card_span(x: i32, len: i32, n: i32) -> (i32, i32) {
    let i = (x * n) / len;
    ((i * len + n - 1) / n, ((i + 1) * len + n - 1) / n)
}

/// How far the card at `(i, j)` has turned, 0 (not started) to 1 (gone) —
/// written once so [`card_wipe`] and the WGSL twin cannot disagree.
#[must_use]
pub fn card_wipe_progress(i: i32, j: i32, p: &CardWipeParams) -> f32 {
    let along = if p.order_axis == 0 {
        (i as f32 + 0.5) / p.grid[0] as f32
    } else {
        (j as f32 + 0.5) / p.grid[1] as f32
    };
    let base = p.order_bias + p.order_scale * along;
    let shuffled = base + (super::noise::hash01(p.seed, 0, i, j, 0) - base) * p.randomness;
    ((p.completion - shuffled * p.one_minus_width) * p.inv_width).clamp(0.0, 1.0)
}

/// Card wipe (docs/08 §3.72) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Each pixel finds its card, asks how far that card has
/// turned, and then solves the one-point projection backwards to find which
/// point of the flat card is standing where it is. A card is never drawn; it is
/// read.
///
/// Mix 0 and Completion 0 are both the bit-exact identity, and Completion 100 is
/// the exactly empty frame — both ends are *tested for* rather than arrived at
/// through a cosine, because `cos(½π)` in `f32` is 6·10⁻⁸ and not zero.
pub fn card_wipe(rgba: &mut [f32], w: u32, h: u32, p: &CardWipeParams) {
    use std::f32::consts::PI;
    let (wi, hi) = (w as i32, h as i32);
    if wi <= 0 || hi <= 0 {
        return;
    }
    let cols = p.grid[0].clamp(1, 256);
    let rows = p.grid[1].clamp(1, 256);
    let d_view = CARD_VIEW_DISTANCE;
    let src = rgba.to_vec();
    for y in 0..hi {
        let (y0, y1) = card_span(y, hi, rows);
        let j = (y * rows) / hi;
        for x in 0..wi {
            let (x0, x1) = card_span(x, wi, cols);
            let i = (x * cols) / wi;
            let o = ((y as i64 * i64::from(wi) + i64::from(x)) * 4) as usize;
            let t = card_wipe_progress(i, j, p);
            let v = if t <= 0.0 {
                [src[o], src[o + 1], src[o + 2], src[o + 3]]
            } else if t >= 1.0 {
                [0.0; 4]
            } else {
                let hx = 0.5 * (x1 - x0) as f32;
                let hy = 0.5 * (y1 - y0) as f32;
                let mx = x0 as f32 + hx;
                let my = y0 as f32 + hy;
                let lx = (x as f32 + 0.5 - mx) / hx;
                let ly = (y as f32 + 0.5 - my) / hy;
                let axis = if p.axis == 2 {
                    u32::from(super::noise::hash01(p.seed, 1, i, j, 0) >= 0.5)
                } else {
                    p.axis
                };
                let sign = match p.direction {
                    1 => -1.0,
                    2 => {
                        if super::noise::hash01(p.seed, 2, i, j, 0) < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    _ => 1.0,
                };
                // The flip coordinate and the one across it, with the card's
                // half-extent on each.
                let (f, g, hf, hg) = if axis == 0 {
                    (ly, lx, hy, hx)
                } else {
                    (lx, ly, hx, hy)
                };
                let (sin, cos) = (sign * t * (PI * 0.5)).sin_cos();
                // The one-point projection, inverted: f = s·cos θ·D ÷ (D − s·sin θ)
                // is a Möbius map in s, so it comes back in one divide.
                let s = f * d_view / (d_view * cos + f * sin);
                let k = d_view / (d_view - s * sin);
                let across = g / k;
                // The card's own edges, in screen units, and the box overlap of
                // this pixel with them — `clamp(a) + clamp(b) − 1` rather than a
                // pair of smoothsteps, so a band narrower than a pixel comes out
                // as its width and not as a half-strength line.
                let near = cos * d_view / (d_view - sin);
                let far = -cos * d_view / (d_view + sin);
                let cov_f = ((near - f) * hf + 0.5).clamp(0.0, 1.0)
                    + ((f - far) * hf + 0.5).clamp(0.0, 1.0)
                    - 1.0;
                let cov_g = ((k - g) * hg + 0.5).clamp(0.0, 1.0)
                    + ((g + k) * hg + 0.5).clamp(0.0, 1.0)
                    - 1.0;
                let cover = cov_f.clamp(0.0, 1.0) * cov_g.clamp(0.0, 1.0);
                // Clamped before sampling so a tap never leaves the card, which
                // is what stops one card bleeding into its neighbour.
                let sc = s.clamp(-1.0, 1.0);
                let ac = across.clamp(-1.0, 1.0);
                let (sx, sy) = if axis == 0 {
                    (mx + ac * hx, my + sc * hy)
                } else {
                    (mx + sc * hx, my + ac * hy)
                };
                let c = bilinear_edge(&src, w, h, sx, sy, 1);
                [c[0] * cover, c[1] * cover, c[2] * cover, c[3] * cover]
            };
            for c in 0..4 {
                rgba[o + c] = src[o + c] * (1.0 - p.mix) + v[c] * p.mix;
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
    block_glitch_matted(
        rgba,
        w,
        h,
        intensity,
        seed,
        tick,
        block_size_px,
        jitter_frac,
        amount_px,
        chan_px,
        slice_frac,
        mix,
        &[],
    );
}

/// [`block_glitch`] driven by a matte (K-395, K-427): each pixel's matte
/// strength scales **Intensity** before any hash is read, so the jitter, the
/// displacement, the channel split and the slice odds all shrink together
/// where the matte darkens. The neutral short-circuit reads the host's
/// Intensity, as it always did. An empty matte is the unmatted path to the
/// byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn block_glitch_matted(
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
    matte: &[f32],
) {
    if intensity == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let bw = block_size_px.max(1.0);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let intensity = intensity * matte_strength(matte, i);
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
    scanlines_matted(
        rgba,
        w,
        h,
        intensity,
        period_px,
        roll_px,
        interlace,
        mix,
        &[],
    );
}

/// The floor on a matte strength before it divides Scanlines' Line period
/// ([`scanlines_matted`]): black reads as a period ten thousand times the set
/// one, which no frame is tall enough to show a line of. The WGSL twin floors
/// at the identical literal.
pub const SCANLINES_MIN_K: f32 = 1e-4;

/// [`scanlines`] driven by a matte (K-395, K-427): each pixel's matte strength
/// `k` **widens the Line period to `period ÷ k`** — the lines spread apart as
/// the matte darkens and vanish at black ([`SCANLINES_MIN_K`] keeps the divide
/// finite) — because scaling Intensity would be the generic dissolve to the
/// bit, and the owner's rule names that as the test. Intensity is untouched.
/// An empty matte is the unmatted path to the byte (K-258).
#[allow(clippy::too_many_arguments)]
pub fn scanlines_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    intensity: f32,
    period_px: f32,
    roll_px: f32,
    interlace: bool,
    mix: f32,
    matte: &[f32],
) {
    if intensity == 0.0 {
        return; // neutral: bit-exact identity (the WGSL twin matches)
    }
    let original = rgba.to_vec();
    let period = period_px.max(1.0);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let period = period / matte_strength(matte, i).max(SCANLINES_MIN_K);
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

/// The perceptual position of a scene-linear value: a square root, floored at
/// zero (docs/08 §3.58 decision 2).
///
/// **Why a square root and not sRGB's 2.2.** Four of this batch's six effects
/// place a control — a rung, a cut, a stop, a pivot — on the tone range, and a
/// person expects the middle of that range to be mid-grey rather than 0.5 of the
/// light. Any transfer curve does that; only this one is a *single
/// correctly-rounded instruction* on both the CPU and the GPU, which is what
/// keeps a quantiser's or a threshold's answer from disagreeing by a whole step
/// between the two paths (§1.6, K-399's rule about a threshold).
#[must_use]
pub fn perceptual(v: f32) -> f32 {
    v.max(0.0).sqrt()
}

/// Posterize (docs/08 §3.58) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Each channel is snapped to the nearest of `n + 1` rungs,
/// with the rungs spaced evenly in [`perceptual`] rather than in light, so the
/// bands land where a person sees them. The ladder is not clipped at 1: a
/// highlight above white keeps climbing it (§2.1).
///
/// `n` is `Levels − 1`, computed host-side. Unpremultiplied (§2.2); alpha is
/// untouched. Mix 0 is the bit-exact identity.
pub fn posterize(rgba: &mut [f32], n: f32, mix: f32) {
    posterize_matted(rgba, n, mix, &[]);
}

/// The rung count a black matte posterizes to: 256 levels, which is `n = 255`
/// steps — the ladder of an 8-bit picture, where no step is visible.
pub const POSTERIZE_UNMATTED_STEPS: f32 = 255.0;

/// [`posterize`] driven by a matte (K-395, docs/08 §2.6): each pixel's matte
/// strength pulls its **Levels toward 256** ([`matte_toward`] on the step
/// count, from [`POSTERIZE_UNMATTED_STEPS`]), so a dark matte means finer
/// rungs and a black one none a person can see — not a coarse ladder faded
/// back over the picture. An empty matte is the unmatted path to the byte
/// (K-258).
pub fn posterize_matted(rgba: &mut [f32], n: f32, mix: f32, matte: &[f32]) {
    if n <= 0.0 {
        return;
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let n = matte_toward(n, POSTERIZE_UNMATTED_STEPS, matte_strength(matte, i * 4));
        let a = px[3];
        let u = unpremult(px);
        for c in 0..3 {
            let t = perceptual(u[c]) * n;
            // `floor(x + ½)` rather than `round`, because WGSL's `round` breaks
            // a tie to even and Rust's breaks it away from zero — on a value
            // that lands exactly between two rungs that is a whole rung of
            // difference between the two paths.
            let step = (t + 0.5).floor() / n;
            let v = step * step;
            px[c] = px[c] * (1.0 - mix) + v * a * mix;
        }
    }
}

/// Threshold (docs/08 §3.59) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** One question per pixel — is it brighter than `level`? —
/// answered white or black. The crossing is a smoothstep of half-width `hw`
/// rather than a step, floored host-side, so the cut is antialiased and the two
/// paths cannot disagree about a pixel that lands exactly on the line.
///
/// Unpremultiplied (§2.2); alpha is untouched, so a thresholded picture keeps
/// its shape. Mix 0 is the bit-exact identity.
pub fn threshold(rgba: &mut [f32], level: f32, hw: f32, mix: f32) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let t = perceptual(u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2]);
        let k = smoothstep_between(level - hw, level + hw, t);
        for ch in px.iter_mut().take(3) {
            *ch = *ch * (1.0 - mix) + k * a * mix;
        }
    }
}

/// One resolved Tritone (docs/08 §3.60): the three stops of the ramp, in
/// scene-linear RGB, and the mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TritoneParams {
    /// The colour the darkest pixels take.
    pub shadows: [f32; 3],
    /// The colour mid-grey takes.
    pub midtones: [f32; 3],
    /// The colour the brightest pixels take.
    pub highlights: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Tritone (docs/08 §3.60) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** The pixel's brightness picks a colour off a two-segment
/// ramp: Shadows at the bottom, Midtones in the middle, Highlights at the top.
/// The position is [`perceptual`], so Midtones lands on the grey a person points
/// at, and anything past white keeps its headroom by *scaling* the chosen colour
/// rather than clamping to it (§2.1).
///
/// Unpremultiplied (§2.2); alpha is untouched. Mix 0 is the bit-exact identity.
pub fn tritone(rgba: &mut [f32], p: &TritoneParams) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let t = perceptual(u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2]);
        let s = t.min(1.0);
        // Both halves are written `lo + (hi − lo)·x` — the same form the WGSL
        // twin spells, so neither path contracts the multiply-add differently
        // (§3.24's note).
        let (lo, hi, x) = if s < 0.5 {
            (p.shadows, p.midtones, s * 2.0)
        } else {
            (p.midtones, p.highlights, s * 2.0 - 1.0)
        };
        let head = t.max(1.0);
        for c in 0..3 {
            let v = (lo[c] + (hi[c] - lo[c]) * x) * head;
            px[c] = px[c] * (1.0 - p.mix) + v * a * p.mix;
        }
    }
}

/// One resolved Photo filter (docs/08 §3.61): the glass's scene-linear colour,
/// how much of it is in front of the lens, and whether the exposure is put back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoFilterParams {
    /// The filter's colour, already decoded to scene-linear host-side.
    pub filter: [f32; 3],
    /// Density ÷ 100. 0 is the exact identity.
    pub density: f32,
    /// 1 to restore the pixel's own luma afterwards, 0 to let the filter cost
    /// light as a real one does.
    pub preserve: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Photo filter (docs/08 §3.61) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Multiply by the filter's colour, fade that multiply in by
/// Density, and — with Preserve luminosity on — scale the result back to the
/// luma it started with, so the picture changes colour rather than exposure.
///
/// Unpremultiplied (§2.2); alpha is untouched. Density 0 and Mix 0 are both the
/// bit-exact identity.
pub fn photo_filter(rgba: &mut [f32], p: &PhotoFilterParams) {
    photo_filter_matted(rgba, p, &[]);
}

/// [`photo_filter`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales its **Density** — thinner glass, not a full filter
/// faded back. An empty matte is the unmatted path to the byte (K-258).
pub fn photo_filter_matted(rgba: &mut [f32], p: &PhotoFilterParams, matte: &[f32]) {
    if p.density == 0.0 {
        return; // no glass: bit-exact identity (the WGSL twin matches)
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let density = p.density * matte_strength(matte, i * 4);
        let a = px[3];
        let u = unpremult(px);
        let mut v = [0.0f32; 3];
        for c in 0..3 {
            v[c] = u[c] + (u[c] * p.filter[c] - u[c]) * density;
        }
        let before = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
        let after = v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2];
        // A filter dark enough to take the luma to nothing has nothing to
        // restore; the floor keeps the division finite (docs/14 §4).
        let gain = before / after.max(1e-6);
        let k = 1.0 + (gain - 1.0) * p.preserve;
        for c in 0..3 {
            px[c] = px[c] * (1.0 - p.mix) + v[c] * k * a * p.mix;
        }
    }
}

/// One resolved Black and white (docs/08 §3.62): the six weights as fractions,
/// in red, yellow, green, cyan, blue, magenta order, and the tint already
/// divided through by its own luma.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackAndWhiteParams {
    /// Reds, Yellows, Greens, Cyans, Blues, Magentas — each ÷ 100.
    pub weights: [f32; 6],
    /// The tint colour, normalised to luma 1 so it changes hue and not exposure.
    pub tint: [f32; 3],
    /// 1 to tint, 0 to leave the grey grey.
    pub tint_on: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The grey one colour makes under six weights (docs/08 §3.62).
///
/// **In plain terms.** Every colour is exactly a grey, plus one secondary
/// (yellow, cyan or magenta), plus one primary (red, green or blue): the
/// smallest channel is the grey, the middle minus the smallest is the secondary
/// between the two larger channels, and the largest minus the middle is the
/// primary. Weighting those two parts is what lets a red jumper and green grass
/// come out as different greys.
///
/// The six branches agree wherever two channels are equal — the term that
/// distinguishes them is zero there — so the function is continuous and a
/// gradient has no seam. The WGSL twin spells the same six branches.
#[must_use]
pub fn bw_grey(u: [f32; 3], w: &[f32; 6]) -> f32 {
    let (r, g, b) = (u[0], u[1], u[2]);
    // (grey, secondary amount, secondary weight, primary amount, primary weight)
    let (base, sec, sw, pri, pw) = if r >= g && g >= b {
        (b, g - b, w[1], r - g, w[0]) // yellow, red
    } else if g >= r && r >= b {
        (b, r - b, w[1], g - r, w[2]) // yellow, green
    } else if g >= b && b >= r {
        (r, b - r, w[3], g - b, w[2]) // cyan, green
    } else if b >= g && g >= r {
        (r, g - r, w[3], b - g, w[4]) // cyan, blue
    } else if b >= r && r >= g {
        (g, r - g, w[5], b - r, w[4]) // magenta, blue
    } else {
        (g, b - g, w[5], r - b, w[0]) // magenta, red
    };
    base + sec * sw + pri * pw
}

/// Black and white (docs/08 §3.62) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** [`bw_grey`] per pixel, floored at zero because a negative
/// weight would otherwise ask for negative light, and then optionally multiplied
/// by the tint. Nothing is clipped above, so a weight of 300 on a specular
/// highlight keeps its headroom (§2.1).
///
/// Unpremultiplied (§2.2); alpha is untouched. Mix 0 is the bit-exact identity.
pub fn black_and_white(rgba: &mut [f32], p: &BlackAndWhiteParams) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let grey = bw_grey(u, &p.weights).max(0.0);
        for (ch, tint) in px.iter_mut().zip(p.tint) {
            let tinted = grey * tint;
            let v = grey + (tinted - grey) * p.tint_on;
            *ch = *ch * (1.0 - p.mix) + v * a * p.mix;
        }
    }
}

/// One resolved Shadow highlight (docs/08 §3.63).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowHighlightParams {
    /// Shadow amount ÷ 100 × 2: 100 trebles a fully-masked shadow.
    pub shadow: f32,
    /// Highlight amount ÷ 100 × 2: 100 takes a fully-masked highlight to a
    /// third.
    pub highlight: f32,
    /// Shadow tonal width ÷ 100, floored so the smoothstep never divides by
    /// zero.
    pub shadow_width: f32,
    /// Highlight tonal width ÷ 100, floored likewise.
    pub highlight_width: f32,
    /// Radius, raster pixels: the gaussian that answers "how bright is this
    /// pixel's neighbourhood?".
    pub radius_px: f32,
    /// 1 + Midtone contrast ÷ 100, about the perceptual middle.
    pub contrast: f32,
    /// Colour correction ÷ 100: the saturation put back where the gain moved.
    pub colour_correction: f32,
    /// False when nothing is being lifted, pulled or steepened: the exact
    /// identity, and the gaussian is not even run.
    pub active: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Shadow highlight (docs/08 §3.63) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Blur the picture at Radius and read its luma: that is how
/// bright each pixel's *neighbourhood* is, and it decides whether the pixel is
/// being treated as a shadow or a highlight. The pixel's own luma is then
/// multiplied by a gain those two masks set, its colour rides along with the
/// gain, and Colour correction puts back the saturation an opened shadow loses.
///
/// The blurred picture is a *question*, not an answer: it never contributes a
/// colour, so nothing here softens the picture.
pub fn shadow_highlight(rgba: &mut [f32], w: u32, h: u32, p: &ShadowHighlightParams) {
    shadow_highlight_matted(rgba, w, h, p, &[]);
}

/// [`shadow_highlight`] driven by a matte (K-395, docs/08 §2.6): each pixel's
/// matte strength scales its **Shadow amount and Highlight amount** before the
/// gain is worked out, so a grey matte lifts less rather than fading a full
/// lift back. The neighbourhood blur, the widths and the midtone contrast are
/// untouched by it. An empty matte is the unmatted path to the byte (K-258).
pub fn shadow_highlight_matted(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    p: &ShadowHighlightParams,
    matte: &[f32],
) {
    if !p.active {
        return;
    }
    // The shared §3.8 gaussian, on the whole picture — the third reuse, after
    // §3.43's softening and §3.57's distance field. Repeat edges, so the frame's
    // own border does not read as a dark neighbourhood.
    let mut soft = rgba.to_vec();
    blur_gaussian(&mut soft, w, h, p.radius_px, 1, 1.0);
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let a = px[3];
        let u = unpremult(px);
        let d = i * 4;
        let ub = unpremult(&soft[d..d + 4]);
        let l = (u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2]).max(0.0);
        let lb = ub[0] * LUMA[0] + ub[1] * LUMA[1] + ub[2] * LUMA[2];
        // Where the neighbourhood sits on the tone range, perceptually. Clamped
        // at 1 because a highlight mask has to saturate somewhere, and past
        // white "brighter" no longer means "more of a highlight".
        let t = perceptual(lb).min(1.0);
        let ms = 1.0 - smoothstep_between(0.0, p.shadow_width, t);
        let mh = smoothstep_between(1.0 - p.highlight_width, 1.0, t);
        // A multiply, not a gamma (§3.63): monotone, no clamp, no inverse.
        let m = matte_strength(matte, d);
        let lifted = l * (1.0 + ms * (p.shadow * m)) / (1.0 + mh * (p.highlight * m));
        // Midtone contrast about the perceptual middle.
        let q = ((perceptual(lifted) - 0.5) * p.contrast + 0.5).max(0.0);
        let out_l = q * q;
        // A black pixel has no colour to scale and no ratio to take.
        let k = if l > 1e-6 { out_l / l } else { 1.0 };
        let mut v = [0.0f32; 3];
        for c in 0..3 {
            v[c] = u[c] * k;
        }
        let g = v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2];
        // The boost applies exactly where the gain differs from 1, so Colour
        // correction 0 is the identity in colour and a pixel the effect did not
        // move is not quietly saturated.
        let sat = 1.0 + p.colour_correction * (k - 1.0).abs().min(1.0);
        for c in 0..3 {
            let out = (g + (v[c] - g) * sat).max(0.0);
            px[c] = px[c] * (1.0 - p.mix) + out * a * p.mix;
        }
    }
}

/// One resolved Median (docs/08 §3.64): the window's half-width in whole raster
/// pixels, whether the coverage is medianed with the colour, and the mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MedianParams {
    /// Half the window's width, in whole raster pixels, `0..=`
    /// [`MEDIAN_MAX_RADIUS`]. 0 is the exact identity.
    pub radius: i32,
    /// AE's "Operate on Alpha Channel".
    pub alpha: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl MedianParams {
    /// The radius held inside the cap §3.64 decision 2 sets. Applied by the
    /// declaration's `packed`, and again here, so a value that reached this
    /// struct any other way cannot overrun the fixed-size selection array.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            radius: self.radius.clamp(0, MEDIAN_MAX_RADIUS),
            ..self
        }
    }
}

/// The hard cap on Median's raster radius (docs/08 §3.64 decision 2). Mirrored
/// by `lumit_core::fx::effects::median::Median::MAX_RADIUS` and by the WGSL
/// kernel's window bound; the §1.6 oracle asserts the three agree.
pub const MEDIAN_MAX_RADIUS: i32 = 3;

/// The length of the sorted run the selection network carries: `⌈N ÷ 2⌉` at the
/// cap, where `N = (2·MEDIAN_MAX_RADIUS + 1)²`.
pub const MEDIAN_KEEP: usize = 25;

/// A value no real pixel reaches, used to pad the selection network's array
/// (docs/08 §3.64 decision 1). It sorts above every sample, so it can never
/// become the answer, and it is finite so nothing here does arithmetic on an
/// infinity.
const MEDIAN_PAD: f32 = 1e30;

/// Median (docs/08 §3.64) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Every pixel is replaced by the middle value of the little
/// square of pixels around it, per channel. The selection is a
/// **compare-exchange network**: sweep the window once, carrying the `⌈N ÷ 2⌉`
/// smallest values seen so far in a sorted array, and insert each new sample by
/// bubbling it down with `min`/`max` pairs. Nothing branches on a value, which
/// is what lets the WGSL twin run the identical comparisons — and because `min`
/// and `max` are exact, the two paths agree bit-for-bit whatever order they
/// sweep in.
///
/// Edges repeat: a transparent surround would win the vote on a corner pixel and
/// eat the frame's own border. Unpremultiplied (§2.2); the coverage is medianed
/// only when `alpha` says so. Radius 0 is the bit-exact identity.
pub fn median(rgba: &mut [f32], w: u32, h: u32, p: &MedianParams) {
    let r = p.radius.clamp(0, MEDIAN_MAX_RADIUS);
    if r == 0 {
        return;
    }
    let n = (2 * r + 1) * (2 * r + 1);
    // The 1-based rank of the median among `n` samples, and therefore how many
    // of the smallest the network has to carry.
    let keep = ((n + 1) / 2) as usize;
    let src = rgba.to_vec();
    let (wi, hi) = (w as i64, h as i64);
    for y in 0..hi {
        for x in 0..wi {
            let mut sorted = [[MEDIAN_PAD; 4]; MEDIAN_KEEP];
            for dy in -r..=r {
                let sy = (y + i64::from(dy)).clamp(0, hi - 1);
                for dx in -r..=r {
                    let sx = (x + i64::from(dx)).clamp(0, wi - 1);
                    let s = ((sy * wi + sx) * 4) as usize;
                    let u = unpremult(&src[s..s + 4]);
                    let mut v = [u[0], u[1], u[2], src[s + 3]];
                    // The bubble: each rung keeps the smaller of what it held
                    // and what is passing through, and hands the larger on.
                    for slot in sorted.iter_mut().take(keep) {
                        for c in 0..4 {
                            let lo = slot[c].min(v[c]);
                            let hi_c = slot[c].max(v[c]);
                            slot[c] = lo;
                            v[c] = hi_c;
                        }
                    }
                }
            }
            let med = sorted[keep - 1];
            let d = ((y * wi + x) * 4) as usize;
            let out_a = if p.alpha { med[3] } else { rgba[d + 3] };
            for c in 0..3 {
                rgba[d + c] = rgba[d + c] * (1.0 - p.mix) + med[c] * out_a * p.mix;
            }
            rgba[d + 3] = rgba[d + 3] * (1.0 - p.mix) + out_a * p.mix;
        }
    }
}

/// One resolved Mosaic (docs/08 §3.65): the grid, the sharp-colour switch and
/// the mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MosaicParams {
    /// Blocks across and blocks down, each `1..=2000`.
    pub blocks: [i32; 2],
    /// On, the block takes its centre pixel's colour; off, the mean of a
    /// stratified sample of it.
    pub sharp: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The most samples Mosaic takes along one axis of a block (docs/08 §3.65 note
/// 2). Mirrored by the WGSL kernel and by
/// `lumit_core::fx::effects::mosaic::Mosaic::MAX_SAMPLES`.
pub const MOSAIC_MAX_SAMPLES: i32 = 8;

/// The block bounds one axis of a Mosaic pixel falls in (docs/08 §3.65 note 1).
///
/// **Integer division throughout**, deliberately: a block edge decided by
/// `floor(x ÷ block_width)` in floating point puts a pixel in different blocks
/// on the two paths wherever the division comes out exact, which is K-399's rule
/// about a threshold arriving on a coordinate.
#[must_use]
pub fn mosaic_span(x: i32, len: i32, blocks: i32) -> (i32, i32) {
    let i = (x * blocks) / len;
    ((i * len) / blocks, ((i + 1) * len) / blocks)
}

/// The `k`-th of `n` stratified sample positions across a block `span` wide
/// starting at `lo` (docs/08 §3.65). Integer arithmetic, for [`mosaic_span`]'s
/// reason.
#[must_use]
pub fn mosaic_sample(lo: i32, span: i32, n: i32, k: i32) -> i32 {
    lo + (2 * k * span + span) / (2 * n)
}

/// Mosaic (docs/08 §3.65) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** The frame is cut into a grid and every pixel takes its
/// block's colour: the centre pixel's with Sharp colours on, otherwise the mean
/// of an at-most-8×8 stratified sample of the block. A block under eight pixels
/// across is sampled completely, so a fine mosaic is an exact mean.
///
/// Premultiplied (§2.2) — the alpha is blocked with the colour, so a mosaicked
/// cut-out gets blocky edges. Mix 0 is the bit-exact identity.
pub fn mosaic(rgba: &mut [f32], w: u32, h: u32, p: &MosaicParams) {
    let (wi, hi) = (w as i32, h as i32);
    if wi <= 0 || hi <= 0 {
        return;
    }
    let bx = p.blocks[0].clamp(1, 2000);
    let by = p.blocks[1].clamp(1, 2000);
    let src = rgba.to_vec();
    let at = |x: i32, y: i32| {
        let s = ((i64::from(y.clamp(0, hi - 1)) * i64::from(wi) + i64::from(x.clamp(0, wi - 1)))
            * 4) as usize;
        [src[s], src[s + 1], src[s + 2], src[s + 3]]
    };
    for y in 0..hi {
        let (y0, y1) = mosaic_span(y, hi, by);
        for x in 0..wi {
            let (x0, x1) = mosaic_span(x, wi, bx);
            let v = if p.sharp {
                at(x0 + (x1 - x0) / 2, y0 + (y1 - y0) / 2)
            } else {
                let nx = (x1 - x0).clamp(1, MOSAIC_MAX_SAMPLES);
                let ny = (y1 - y0).clamp(1, MOSAIC_MAX_SAMPLES);
                let mut acc = [0.0f32; 4];
                for j in 0..ny {
                    let sy = mosaic_sample(y0, y1 - y0, ny, j);
                    for i in 0..nx {
                        let sx = mosaic_sample(x0, x1 - x0, nx, i);
                        let c = at(sx, sy);
                        for k in 0..4 {
                            acc[k] += c[k];
                        }
                    }
                }
                let inv = 1.0 / (nx * ny) as f32;
                [acc[0] * inv, acc[1] * inv, acc[2] * inv, acc[3] * inv]
            };
            let d = ((i64::from(y) * i64::from(wi) + i64::from(x)) * 4) as usize;
            for c in 0..4 {
                rgba[d + c] = rgba[d + c] * (1.0 - p.mix) + v[c] * p.mix;
            }
        }
    }
}

/// The 3×3 Sobel pair, in raster reading order (docs/08 §3.66). Written out
/// rather than generated, because the WGSL twin spells the same nine numbers in
/// the same order and the two must sum them identically.
const SOBEL_X: [f32; 9] = [-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0];
/// See [`SOBEL_X`].
const SOBEL_Y: [f32; 9] = [-1.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0];

/// Find edges (docs/08 §3.66) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** A Sobel gradient per channel, taken on the **perceptual**
/// value rather than on the light (§3.58's curve again): in light the step from
/// 3.0 to 4.0 in a sunlit sky would outrank the step from 0.01 to 0.05 in a
/// shadow, though the eye sees the second and not the first.
///
/// `invert` is 1 for bright edges on black and 0 for AE's default, dark edges on
/// white. Unpremultiplied (§2.2); alpha is untouched, so the drawing keeps the
/// layer's shape. Edges repeat. Mix 0 is the bit-exact identity.
pub fn find_edges(rgba: &mut [f32], w: u32, h: u32, invert: f32, mix: f32) {
    let src = rgba.to_vec();
    let (wi, hi) = (w as i64, h as i64);
    for y in 0..hi {
        for x in 0..wi {
            let mut gx = [0.0f32; 3];
            let mut gy = [0.0f32; 3];
            for j in 0..3i64 {
                let sy = (y + j - 1).clamp(0, hi - 1);
                for i in 0..3i64 {
                    let sx = (x + i - 1).clamp(0, wi - 1);
                    let s = ((sy * wi + sx) * 4) as usize;
                    let u = unpremult(&src[s..s + 4]);
                    let k = (j * 3 + i) as usize;
                    for c in 0..3 {
                        let t = perceptual(u[c]);
                        gx[c] += t * SOBEL_X[k];
                        gy[c] += t * SOBEL_Y[k];
                    }
                }
            }
            let d = ((y * wi + x) * 4) as usize;
            let a = rgba[d + 3];
            for c in 0..3 {
                let e = (gx[c] * gx[c] + gy[c] * gy[c]).sqrt().min(1.0);
                // `1 − e` for the pencil drawing, `e` for the glow. Written as
                // one lerp so neither path takes a branch on the switch.
                let q = (1.0 - e) + (e - (1.0 - e)) * invert;
                rgba[d + c] = rgba[d + c] * (1.0 - mix) + q * q * a * mix;
            }
        }
    }
}

/// One resolved Emboss (docs/08 §3.67): the light's offset in raster pixels, the
/// gain on the difference, and the mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbossParams {
    /// The vector from the pixel toward the light, in raster pixels — Direction
    /// and Relief already folded together host-side.
    pub offset: [f32; 2],
    /// Contrast ÷ 100.
    pub contrast: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Emboss (docs/08 §3.67) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** Two taps either side of the pixel along the light's axis,
/// differenced perceptually and written as grey to all three channels — the
/// stamped-metal look. Relief 0 is flat mid-grey rather than the identity: with
/// no separation between the taps there is no relief to see.
///
/// Unpremultiplied (§2.2); alpha is untouched. Edges repeat. Mix 0 is the
/// bit-exact identity.
pub fn emboss(rgba: &mut [f32], w: u32, h: u32, p: &EmbossParams) {
    let src = rgba.to_vec();
    let luma_at = |sx: f32, sy: f32| {
        let t = bilinear_edge(&src, w, h, sx, sy, 1);
        let u = unpremult(&t);
        perceptual(u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2])
    };
    for y in 0..h {
        for x in 0..w {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let a = luma_at(px - p.offset[0], py - p.offset[1]);
            let b = luma_at(px + p.offset[0], py + p.offset[1]);
            let g = (0.5 + (b - a) * p.contrast).max(0.0);
            let v = g * g;
            let d = (y as usize * w as usize + x as usize) * 4;
            let alpha = rgba[d + 3];
            for c in 0..3 {
                rgba[d + c] = rgba[d + c] * (1.0 - p.mix) + v * alpha * p.mix;
            }
        }
    }
}

/// One resolved Texturize (docs/08 §3.68).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexturizeParams {
    /// The vector from the pixel toward the light, in raster pixels — Light
    /// direction and Relief already folded together host-side.
    pub offset: [f32; 2],
    /// Texture contrast ÷ 100.
    pub contrast: f32,
    /// `100 ÷ Scale`: how many copies of the texture span the frame.
    pub inv_scale: f32,
    /// 0 Stretch (hold the texture's edge outward), 1 Tile (wrap), 2 Centre
    /// (no relief outside one copy).
    pub placement: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// Texturize (docs/08 §3.68) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** The texture layer is embossed exactly as §3.67 embosses a
/// picture, and the light and shade that come out multiply this layer's colour.
/// `texture` is the referenced layer already rendered at this raster
/// (docs/impl/layer-input.md), which is why Placement is a *fitting* rather than
/// a resize: see §3.68 decision 2.
///
/// Premultiplied (§2.2) — the relief is a scalar multiply, which is the same
/// operation on premultiplied and on straight colour — but the *texture's* taps
/// are unpremultiplied, so a texture with a soft edge does not read as black
/// there. Mix 0 is the bit-exact identity.
pub fn texturize(rgba: &mut [f32], texture: &[f32], w: u32, h: u32, p: &TexturizeParams) {
    let (fw, fh) = (w as f32, h as f32);
    let du = p.offset[0] * p.inv_scale / fw;
    let dv = p.offset[1] * p.inv_scale / fh;
    let tap = |u: f32, v: f32| -> Option<f32> {
        let (su, sv) = match p.placement {
            // Tile: wrap into 0..1 by subtracting the floor, the form WGSL
            // spells op-for-op (§3.38's note about `rem_euclid`).
            1 => (u - u.floor(), v - v.floor()),
            // Centre: nothing outside one copy is textured.
            2 if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) => return None,
            // Stretch (and Centre inside the copy): hold the edge outward, which
            // `bilinear_edge`'s repeat policy already does.
            _ => (u, v),
        };
        let t = bilinear_edge(texture, w, h, su * fw, sv * fh, 1);
        let c = unpremult(&t);
        Some(perceptual(c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]))
    };
    for y in 0..h {
        for x in 0..w {
            let u = ((x as f32 + 0.5) / fw - 0.5) * p.inv_scale + 0.5;
            let v = ((y as f32 + 0.5) / fh - 0.5) * p.inv_scale + 0.5;
            let r = match (tap(u + du, v + dv), tap(u - du, v - dv)) {
                (Some(b), Some(a)) => (b - a) * p.contrast,
                _ => 0.0,
            };
            let d = (y as usize * w as usize + x as usize) * 4;
            for c in 0..3 {
                let out = (rgba[d + c] * (1.0 + r)).max(0.0);
                rgba[d + c] = rgba[d + c] * (1.0 - p.mix) + out * p.mix;
            }
        }
    }
}

/// One resolved Broadcast safe (docs/08 §3.69).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BroadcastSafeParams {
    /// The largest `Y + C` the pixel may carry — Maximum signal and the
    /// standard's setup pedestal already folded together host-side, which is why
    /// neither kernel branches on NTSC versus PAL.
    pub target: f32,
    /// 0 Reduce brightness, 1 Reduce saturation, 2 Key out unsafe, 3 Key out
    /// safe.
    pub mode: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The composite chroma amplitude of an encoded colour (docs/08 §3.69): the
/// classic `U`/`V` weights, whose magnitude is what rides on top of the luma in
/// a composite signal.
#[must_use]
pub fn broadcast_chroma(v: [f32; 3], y: f32) -> f32 {
    let cu = 0.493 * (v[2] - y);
    let cv = 0.877 * (v[0] - y);
    (cu * cu + cv * cv).sqrt()
}

/// Broadcast safe (docs/08 §3.69) — the CPU reference and §1.6 oracle.
///
/// **In plain terms.** The pixel is encoded (the batch's √, §3.69 decision 2),
/// its composite amplitude `Y + C` is measured, and where that is over the
/// target one of four things happens: the pixel is scaled down, drained of
/// colour, keyed out, or kept as the only thing left. A pixel already under the
/// target is untouched by the two repair modes, by construction rather than by
/// short-circuit.
///
/// Unpremultiplied (§2.2). Mix 0 is the bit-exact identity.
pub fn broadcast_safe(rgba: &mut [f32], p: &BroadcastSafeParams) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        let u = unpremult(px);
        let v = [perceptual(u[0]), perceptual(u[1]), perceptual(u[2])];
        let y = v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2];
        let c = broadcast_chroma(v, y);
        let amp = y + c;
        let mut out = v;
        let mut out_a = a;
        match p.mode {
            // Scale the whole signal: Y and C are both linear in it, so the
            // factor that lands the amplitude on the target is exact.
            0 => {
                let k = (p.target / amp.max(1e-6)).min(1.0);
                for ch in &mut out {
                    *ch *= k;
                }
            }
            // Pull toward the grey of the same luma: Y is unchanged and C
            // scales, so the factor is again exact. A pixel whose luma alone is
            // over the target ends fully desaturated and still hot — §3.69
            // decision 3 says so rather than hiding it.
            1 => {
                let m = (p.target - y).clamp(0.0, c) / c.max(1e-6);
                for ch in &mut out {
                    *ch = y + (*ch - y) * m;
                }
            }
            // The two diagnostic views. The comparison is `>` in one and `<=`
            // in the other, so the two are exact complements.
            2 if amp > p.target => out_a = 0.0,
            3 if amp <= p.target => out_a = 0.0,
            _ => {}
        }
        for c in 0..3 {
            let lit = out[c] * out[c];
            px[c] = px[c] * (1.0 - p.mix) + lit * out_a * p.mix;
        }
        px[3] = a * (1.0 - p.mix) + out_a * p.mix;
    }
}

/// One resolved Beam (docs/08 §3.73), reduced to what both paths read. The two
/// ends of the drawn interval are already clamped, and `active` records the
/// degenerate case so neither path divides by a zero-length beam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamParams {
    /// Where the beam starts, raster pixels.
    pub start: [f32; 2],
    /// `End − Start`, raster pixels.
    pub axis: [f32; 2],
    /// `1 ÷ |axis|²`, floored.
    pub inv_len2: f32,
    /// The tail, as a fraction of the axis, already clamped.
    pub u0: f32,
    /// The head; never below [`u0`](Self::u0).
    pub u1: f32,
    /// `1 ÷ (u1 − u0)`, floored; only read when `active`.
    pub inv_span: f32,
    /// The half-thickness at the tail, raster pixels.
    pub half0: f32,
    /// The half-thickness at the head, raster pixels.
    pub half1: f32,
    /// Softness ÷ 100, floored above zero.
    pub soft: f32,
    /// The core's colour, scene-linear RGB.
    pub inside: [f32; 3],
    /// The rim's colour, scene-linear RGB.
    pub outside: [f32; 3],
    /// False when the drawn interval is empty — Time 0 (§3.73's fourth note).
    pub active: bool,
    /// Whether the layer that arrived is kept under the beam.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One pixel's beam: its premultiplied colour and its coverage — the expression
/// both paths evaluate, written once so [`beam`] and the WGSL twin cannot
/// disagree about it.
#[must_use]
pub fn beam_sample(px: f32, py: f32, p: &BeamParams) -> ([f32; 3], f32) {
    if !p.active {
        return ([0.0; 3], 0.0);
    }
    let rx = px - p.start[0];
    let ry = py - p.start[1];
    let s = ((rx * p.axis[0] + ry * p.axis[1]) * p.inv_len2).clamp(p.u0, p.u1);
    let qx = rx - s * p.axis[0];
    let qy = ry - s * p.axis[1];
    let r = (qx * qx + qy * qy).sqrt();
    let f = (s - p.u0) * p.inv_span;
    let half = p.half0 + (p.half1 - p.half0) * f;
    // The colour crossover: the core is the inside colour, the rim the outside
    // one, and Softness is the share of the half-width the rim occupies. The
    // crossover takes the rim's INNER HALF, so the outside colour is reached
    // before the edge and is a band rather than a hairline nobody can see.
    let k = ((r / half.max(1e-3) - (1.0 - p.soft)) / (p.soft * 0.5)).clamp(0.0, 1.0);
    let cov = (half + 0.5 - r).clamp(0.0, 1.0);
    let mut c = [0.0f32; 3];
    for (i, ch) in c.iter_mut().enumerate() {
        *ch = (p.inside[i] + (p.outside[i] - p.inside[i]) * k) * cov;
    }
    (c, cov)
}

/// Beam (docs/08 §3.73) — the CPU reference and §1.6 oracle. The beam is written
/// over the picture (or over nothing, with Composite on original off) in
/// premultiplied form, all four channels.
pub fn beam(rgba: &mut [f32], w: u32, h: u32, p: &BeamParams) {
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let (c, cov) = beam_sample(x as f32 + 0.5, y as f32 + 0.5, p);
            let keep = if p.composite { 1.0 - cov } else { 0.0 };
            for i in 0..3 {
                let lit = rgba[d + i] * keep + c[i];
                rgba[d + i] = rgba[d + i] * (1.0 - p.mix) + lit * p.mix;
            }
            let lit = rgba[d + 3] * keep + cov;
            rgba[d + 3] = rgba[d + 3] * (1.0 - p.mix) + lit * p.mix;
        }
    }
}

/// The most segments a bolt and its forks may occupy (docs/08 §3.74's first
/// decision). Three kilobytes of uniform, and more bolt than Forking 100 asks
/// for.
pub const LIGHTNING_SEGMENTS: usize = 192;

/// One resolved Lightning (docs/08 §3.74), reduced to what both paths read.
///
/// **The geometry is already built** — §3.74's first decision. Every segment is
/// `(ax, ay, bx, by)` in raster pixels and carries its own fade, so the kernel
/// does no randomness at all and the two paths are handed the identical numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightningParams {
    /// The bolt, as straight segments in raster pixels.
    pub segments: [[f32; 4]; LIGHTNING_SEGMENTS],
    /// Each segment's brightness, 0..1, already carrying Decay and the fork dim.
    pub fades: [f32; LIGHTNING_SEGMENTS],
    /// How many of the above are real.
    pub count: u32,
    /// The core's half-width in raster pixels.
    pub core_radius: f32,
    /// The glow's reach in raster pixels.
    pub glow_radius: f32,
    /// Glow opacity ÷ 100.
    pub glow_opacity: f32,
    /// The core's colour, scene-linear RGB.
    pub core_colour: [f32; 3],
    /// The glow's colour, scene-linear RGB.
    pub glow_colour: [f32; 3],
    /// Whether the layer that arrived is kept under the bolt.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The distance from a point to one segment — the capsule both paths measure,
/// with the projection clamped so the ends are round rather than infinite.
#[must_use]
fn segment_distance(px: f32, py: f32, s: &[f32; 4]) -> f32 {
    let dx = s[2] - s[0];
    let dy = s[3] - s[1];
    let rx = px - s[0];
    let ry = py - s[1];
    let t = ((rx * dx + ry * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
    let ox = rx - t * dx;
    let oy = ry - t * dy;
    (ox * ox + oy * oy).sqrt()
}

/// One pixel's bolt: the core and glow weights — the expression both paths
/// evaluate. Taken as a **maximum** over the segments, never a sum (§3.74's
/// fourth decision), so the joints and the forks do not bead.
#[must_use]
pub fn lightning_sample(px: f32, py: f32, p: &LightningParams) -> (f32, f32) {
    let mut core = 0.0f32;
    let mut glow = 0.0f32;
    let core_r = p.core_radius;
    let glow_r = p.glow_radius;
    for i in 0..p.count as usize {
        let d = segment_distance(px, py, &p.segments[i]);
        let fade = p.fades[i];
        let c = ((core_r + 0.5 - d) / core_r.max(0.5)).clamp(0.0, 1.0);
        core = core.max(fade * c);
        let g = ((glow_r - d) / glow_r.max(1e-3)).clamp(0.0, 1.0);
        glow = glow.max(fade * g * g);
    }
    (core, glow)
}

/// Lightning (docs/08 §3.74) — the CPU reference and §1.6 oracle. Two weights a
/// pixel become one premultiplied colour and one coverage.
pub fn lightning(rgba: &mut [f32], w: u32, h: u32, p: &LightningParams) {
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let (core, glow) = lightning_sample(x as f32 + 0.5, y as f32 + 0.5, p);
            // The glow lights what the core has not already taken, so the two
            // add to a coverage that cannot exceed one.
            let gw = glow * p.glow_opacity * (1.0 - core);
            let cov = (core + gw).clamp(0.0, 1.0);
            let keep = if p.composite { 1.0 - cov } else { 0.0 };
            for i in 0..3 {
                let c = p.core_colour[i] * core + p.glow_colour[i] * gw;
                let lit = rgba[d + i] * keep + c;
                rgba[d + i] = rgba[d + i] * (1.0 - p.mix) + lit * p.mix;
            }
            let lit = rgba[d + 3] * keep + cov;
            rgba[d + 3] = rgba[d + 3] * (1.0 - p.mix) + lit * p.mix;
        }
    }
}

/// The most waves alive at once (docs/08 §3.75's fourth note): a budget, and one
/// that cannot be typed past.
pub const RADIO_WAVES_MAX: i32 = 32;

/// One resolved Radio waves (docs/08 §3.75), reduced to what both paths read.
/// The polygon is already solved into one sector, for a **unit** radius, because
/// every wave is that shape scaled (§3.75's second note).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioWavesParams {
    /// Where the waves are emitted, raster pixels.
    pub centre: [f32; 2],
    /// The unit shape's first vertex, in the sector's own frame.
    pub vertex: [f32; 2],
    /// The outward unit normal of the edge leaving it.
    pub normal: [f32; 2],
    /// One sector, radians: `2π ÷ Sides`.
    pub period: f32,
    /// Rotation in radians, from straight up, clockwise.
    pub rotation: f32,
    /// Spin in radians per second.
    pub spin: f32,
    /// The newest wave's index, `floor(Time × Frequency)` — taken host-side
    /// (K-399).
    pub newest: i32,
    /// How many waves to walk back from it.
    pub count: i32,
    /// The Time control, seconds.
    pub time: f32,
    /// `1 ÷ Frequency`, seconds between waves.
    pub period_s: f32,
    /// Expansion in raster pixels per second.
    pub expansion: f32,
    /// Lifespan in seconds, floored above zero.
    pub lifespan: f32,
    /// The stroke's half-width in raster pixels.
    pub half_width: f32,
    /// Fade in as a share of the lifespan, floored above zero.
    pub fade_in: f32,
    /// Fade out as a share of the lifespan, floored above zero.
    pub fade_out: f32,
    /// The stroke's colour, scene-linear RGB.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// Whether the layer that arrived is kept under the waves.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One pixel's coverage under the live waves — the expression both paths
/// evaluate. §3.71's sector fold, once per wave, against a shape scaled to that
/// wave's radius.
#[must_use]
pub fn radio_waves_sample(px: f32, py: f32, p: &RadioWavesParams) -> f32 {
    let rx = px - p.centre[0];
    let ry = py - p.centre[1];
    let r = (rx * rx + ry * ry).sqrt();
    let phi = ry.atan2(rx) + std::f32::consts::FRAC_PI_2;
    let mut acc = 0.0f32;
    for j in 0..p.count {
        let k = p.newest - j;
        if k < 0 {
            continue;
        }
        let age = p.time - k as f32 * p.period_s;
        if age < 0.0 || age > p.lifespan {
            continue;
        }
        let rad = age * p.expansion;
        // The sector fold: `floor(x + ½)` and never `round` (§3.47's reason).
        let turned = phi - p.rotation - p.spin * age;
        let a = (turned - p.period * (turned / p.period + 0.5).floor()).abs();
        let (sin, cos) = a.sin_cos();
        let dx = r * cos - rad * p.vertex[0];
        let dy = r * sin - rad * p.vertex[1];
        let dist = (dx * p.normal[0] + dy * p.normal[1]).abs();
        let cov = ((p.half_width + 0.5 - dist) / p.half_width.max(0.5)).clamp(0.0, 1.0);
        let u = age / p.lifespan;
        let fade = (u / p.fade_in)
            .clamp(0.0, 1.0)
            .min(((1.0 - u) / p.fade_out).clamp(0.0, 1.0));
        acc = acc.max(cov * fade);
    }
    acc
}

/// Radio waves (docs/08 §3.75) — the CPU reference and §1.6 oracle.
pub fn radio_waves(rgba: &mut [f32], w: u32, h: u32, p: &RadioWavesParams) {
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let cov = radio_waves_sample(x as f32 + 0.5, y as f32 + 0.5, p) * p.opacity;
            let keep = if p.composite { 1.0 - cov } else { 0.0 };
            for i in 0..3 {
                let lit = rgba[d + i] * keep + p.colour[i] * cov;
                rgba[d + i] = rgba[d + i] * (1.0 - p.mix) + lit * p.mix;
            }
            let lit = rgba[d + 3] * keep + cov;
            rgba[d + 3] = rgba[d + 3] * (1.0 - p.mix) + lit * p.mix;
        }
    }
}

/// One resolved Vegas (docs/08 §3.76), reduced to what both paths read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegasParams {
    /// True to read the alpha rather than the perceptual luma.
    pub from_alpha: bool,
    /// The contour's level, 0..1 in the read value.
    pub level: f32,
    /// The stroke's half-width in raster pixels.
    pub half_width: f32,
    /// The soft band either side of it, raster pixels, floored above zero.
    pub band: f32,
    /// `1 ÷ Segment length`, raster pixels.
    pub inv_segment: f32,
    /// The lit share of one segment, 0..1.
    pub duty: f32,
    /// Rotation in turns — one full turn marches the dashes on by a segment.
    pub phase: f32,
    /// The stroke's colour, scene-linear RGB.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// Whether the layer that arrived is kept under the stroke.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The value Vegas takes its contour from at one pixel: the perceptual luma of
/// the unpremultiplied colour, or the alpha (§3.76 decision 5).
#[must_use]
fn vegas_value(px: &[f32], from_alpha: bool) -> f32 {
    if from_alpha {
        return px[3];
    }
    let u = unpremult(px);
    perceptual(u[0]) * LUMA[0] + perceptual(u[1]) * LUMA[1] + perceptual(u[2]) * LUMA[2]
}

/// One pixel's stroke coverage — the expression both paths evaluate. `l` is the
/// smoothed value and `gx`/`gy` the Sobel pair, already normalised, so this
/// function is identical either side of the fetch.
///
/// `px`/`py` are measured **from the middle of the frame**, not from its corner,
/// and that is not cosmetic: the dash's phase is the pixel's position projected
/// on the contour's direction, so an error of ε in that direction moves the
/// phase by `|p|·ε`. Halving the arm halves the wobble for nothing.
#[must_use]
pub fn vegas_stroke(px: f32, py: f32, l: f32, gx: f32, gy: f32, p: &VegasParams) -> f32 {
    let g = (gx * gx + gy * gy).sqrt();
    // The signed distance to the level set, in pixels (§3.76 decision 1). A flat
    // neighbourhood sends this to infinity, which switches the stroke off rather
    // than lighting it.
    let sd = (l - p.level) / g.max(1e-6);
    let across = ((p.half_width - sd.abs()) / p.band + 0.5).clamp(0.0, 1.0);
    // The contour's own direction, and the dash laid along it.
    let inv = 1.0 / g.max(1e-6);
    let tx = -gy * inv;
    let ty = gx * inv;
    let phase = (px * tx + py * ty) * p.inv_segment + p.phase;
    let frac = phase - phase.floor();
    let soft = (p.band * p.inv_segment).max(1e-4);
    let along = ((p.duty - frac) / soft + 0.5).clamp(0.0, 1.0);
    across * along
}

/// The smoothing and derivative halves of the 5×5 Sobel (docs/08 §3.76 decision
/// 1). Five taps of smoothing either way is what makes the contour's *direction*
/// steady enough to lay a dash along: a 3×3 gradient on a compressed gradient
/// points a different way in every pixel, and the dashes come out as speckle.
pub const VEGAS_SMOOTH: [f32; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];
/// The derivative half; see [`VEGAS_SMOOTH`].
pub const VEGAS_DERIV: [f32; 5] = [-1.0, -2.0, 0.0, 2.0, 1.0];

/// Vegas (docs/08 §3.76) — the CPU reference and §1.6 oracle. One separable 5×5
/// Sobel over the neighbourhood, clamped at the frame's edge, then the stroke.
pub fn vegas(rgba: &mut [f32], w: u32, h: u32, p: &VegasParams) {
    let src = rgba.to_vec();
    let at = |x: i32, y: i32| -> f32 {
        let cx = x.clamp(0, w as i32 - 1) as usize;
        let cy = y.clamp(0, h as i32 - 1) as usize;
        let d = (cy * w as usize + cx) * 4;
        vegas_value(&src[d..d + 4], p.from_alpha)
    };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let d = (y as usize * w as usize + x as usize) * 4;
            let mut l = 0.0f32;
            let mut gx = 0.0f32;
            let mut gy = 0.0f32;
            for j in 0..5 {
                for i in 0..5 {
                    let v = at(x + i as i32 - 2, y + j as i32 - 2);
                    l += VEGAS_SMOOTH[i] * VEGAS_SMOOTH[j] * v;
                    gx += VEGAS_DERIV[i] * VEGAS_SMOOTH[j] * v;
                    gy += VEGAS_SMOOTH[i] * VEGAS_DERIV[j] * v;
                }
            }
            // 16 for each smoothing pass, 8 for the derivative's own scale.
            let l = l * (1.0 / 256.0);
            let gx = gx * (1.0 / 128.0);
            let gy = gy * (1.0 / 128.0);
            let cov = vegas_stroke(
                x as f32 + 0.5 - w as f32 * 0.5,
                y as f32 + 0.5 - h as f32 * 0.5,
                l,
                gx,
                gy,
                p,
            ) * p.opacity;
            let keep = if p.composite { 1.0 - cov } else { 0.0 };
            for ch in 0..3 {
                let lit = rgba[d + ch] * keep + p.colour[ch] * cov;
                rgba[d + ch] = rgba[d + ch] * (1.0 - p.mix) + lit * p.mix;
            }
            let lit = rgba[d + 3] * keep + cov;
            rgba[d + 3] = rgba[d + 3] * (1.0 - p.mix) + lit * p.mix;
        }
    }
}

/// One resolved Add grain (docs/08 §3.77), reduced to what both paths read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddGrainParams {
    /// The grain's amplitude in perceptual units, per channel — Intensity, the
    /// channel gain and the fixed 0.1 scale already multiplied together.
    pub amplitude: [f32; 3],
    /// `1 ÷ Size`, raster pixels.
    pub inv_size: f32,
    /// Softness ÷ 100.
    pub softness: f32,
    /// The three tonal weights, each already divided by 100.
    pub tonal: [f32; 3],
    /// True to read one lane for all three channels.
    pub monochrome: bool,
    /// Which draw the grain follows (§2.4).
    pub seed: u32,
    /// The frame's draw, zero when Animate is off.
    pub tick: i32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One channel's grain at one cell coordinate — the expression both paths
/// evaluate. The hard reading is one flat cell, the soft one the same lattice
/// interpolated, and Softness crossfades them (§3.77's second note).
#[must_use]
pub fn grain_at(qx: f32, qy: f32, lane: u32, p: &AddGrainParams) -> f32 {
    let hard = hash01(p.seed, lane, qx.floor() as i32, qy.floor() as i32, p.tick) * 2.0 - 1.0;
    let soft = super::noise::value3(p.seed, lane, qx, qy, p.tick as f32, 0);
    hard + (soft - hard) * p.softness
}

/// Add grain (docs/08 §3.77) — the CPU reference and §1.6 oracle. Unpremultiplied
/// (§2.2); the wobble is added on the perceptual value and squared back.
pub fn add_grain(rgba: &mut [f32], w: u32, h: u32, p: &AddGrainParams) {
    // Intensity 0: the bit-exact identity, and it needs saying rather than
    // arriving — `perceptual(v)²` is not `v` in the last bit (the WGSL twin
    // short-circuits identically).
    if p.amplitude[0] == 0.0 && p.amplitude[1] == 0.0 && p.amplitude[2] == 0.0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let a = rgba[d + 3];
            let u = unpremult(&rgba[d..d + 4]);
            let qx = (x as f32 + 0.5) * p.inv_size;
            let qy = (y as f32 + 0.5) * p.inv_size;
            for c in 0..3 {
                let v = perceptual(u[c]);
                // Three hats summing to one, so 100/100/100 is provably neutral.
                let h0 = (1.0 - 2.0 * v).clamp(0.0, 1.0);
                let h2 = (2.0 * v - 1.0).clamp(0.0, 1.0);
                let weight = p.tonal[0] * h0 + p.tonal[1] * (1.0 - h0 - h2) + p.tonal[2] * h2;
                let lane = if p.monochrome { 0 } else { c as u32 };
                let g = grain_at(qx, qy, lane, p);
                let out = (v + g * p.amplitude[c] * weight).max(0.0);
                rgba[d + c] = rgba[d + c] * (1.0 - p.mix) + out * out * a * p.mix;
            }
        }
    }
}

/// The most straight pieces one path-drawn effect may occupy — the budget
/// Scribble's hatch, Stroke's brush trail and Vegas' mask stroke all draw from
/// (docs/08 §3.78, §3.79, §3.76).
///
/// A cap for §3.74's reason: the geometry rides in a uniform, and a uniform is a
/// fixed size. 512 is generous — a full-frame 1080p ellipse flattens to a few
/// hundred points at the K-408 tolerance — and what happens past it is a
/// coarsening, never a fault: [`path_chain`] keeps every part of the shape and
/// merges pieces to fit, Scribble widens its own spacing before it gets here,
/// and Stroke spaces its dots out.
pub const PATH_PRIMITIVES: usize = 512;

/// [`PathDrawParams::style`]: the drawing sits on the picture that arrived.
pub const PAINT_ON_ORIGINAL: u32 = 0;
/// [`PathDrawParams::style`]: the drawing is all there is.
pub const PAINT_ON_TRANSPARENT: u32 = 1;
/// [`PathDrawParams::style`]: the drawing is a *hole* — it reveals the picture
/// that arrived and hides the rest of it. AE's Reveal Original Image.
pub const PAINT_REVEAL_ORIGINAL: u32 = 2;

/// A point that lifts the pen: the edges into and out of it are not drawn, and
/// neither adds to the distance along (docs/08 §3.78).
///
/// One continuous line is what Scribble wants — the pen travels from the end of
/// each stroke to the start of the next, which is what makes it read as a
/// scribble rather than as a comb — but a mask with a notch in it has *two*
/// strokes on one line, and joining those would draw straight through the hole.
/// So the chain carries a lift, spelled as a non-finite point because no real
/// vertex can ever collide with one.
pub const PEN_UP: [f32; 2] = [f32::NAN, f32::NAN];

/// One resolved path drawing (docs/08 §3.78, §3.79 and §3.76's Mask/Path
/// source), reduced to what both paths read.
///
/// # In plain terms
///
/// Three effects draw a *line* rather than a picture: Scribble fills a mask with
/// pencil strokes, Stroke walks a brush round one, and Vegas can march its
/// dashes along one. They differ entirely in **where the line goes** and hardly
/// at all in **how it is drawn**, so the line is worked out first, on the CPU,
/// and all three hand the same small description to the same kernel.
///
/// **The geometry is already built.** It arrives here as a list of straight
/// pieces in **raster pixels**, each carrying how far along the drawing its
/// start sits. That is §3.74's first decision applied to a whole family:
/// neither render path generates geometry, so neither can generate it
/// differently, and §1.6's comparison is handed identical numbers rather than
/// two hashes that must agree.
///
/// A `count` of zero is the documented no-op — an unset mask row, a deleted
/// mask, a mask with no area — and renders the input unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathDrawParams {
    /// The drawing, as `(ax, ay, bx, by)` in raster pixels.
    pub segments: [[f32; 4]; PATH_PRIMITIVES],
    /// How far along the whole drawing each piece's `a` end sits, raster pixels
    /// — what lets a dash be evenly spaced *round* a path rather than projected
    /// across it.
    pub arcs: [f32; PATH_PRIMITIVES],
    /// How many of the above are real.
    pub count: u32,
    /// Half the drawn width, raster pixels.
    pub half_width: f32,
    /// The soft band either side of it, raster pixels, floored above zero.
    pub band: f32,
    /// `1 ÷ dash length`, raster pixels — 0 for an undashed drawing.
    pub inv_segment: f32,
    /// The lit share of one dash; **2 for a continuous line**, which is how
    /// Scribble and Stroke switch the dash off without a branch (§3.76's own
    /// convention, and part of why there is one kernel and not three).
    pub duty: f32,
    /// The dash's phase in turns.
    pub phase: f32,
    /// How far the drawing is wobbled, raster pixels. 0 is no wobble at all and
    /// skips the noise entirely.
    pub wiggle_amp: f32,
    /// The wobble's frequency, cells per raster pixel.
    pub wiggle_freq: f32,
    /// Where in the wobble's evolution this frame sits — taken from layer time
    /// host-side, so the kernel never sees a clock (§2.4).
    pub wiggle_tick: f32,
    /// The wobble's seed.
    pub seed: u32,
    /// The drawing's colour, scene-linear RGB.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// One of [`PAINT_ON_ORIGINAL`], [`PAINT_ON_TRANSPARENT`],
    /// [`PAINT_REVEAL_ORIGINAL`].
    pub style: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl PathDrawParams {
    /// An empty drawing — the no-op every path effect degrades to, and the base
    /// each of them fills in with `..PathDrawParams::blank()`.
    ///
    /// Not [`Default`], because an array of 512 does not derive one.
    #[must_use]
    pub fn blank() -> Self {
        Self {
            segments: [[0.0; 4]; PATH_PRIMITIVES],
            arcs: [0.0; PATH_PRIMITIVES],
            count: 0,
            half_width: 0.0,
            band: 0.5,
            inv_segment: 0.0,
            duty: 2.0,
            phase: 0.0,
            wiggle_amp: 0.0,
            wiggle_freq: 0.0,
            wiggle_tick: 0.0,
            seed: 0,
            colour: [0.0; 3],
            opacity: 0.0,
            style: PAINT_ON_ORIGINAL,
            mix: 0.0,
        }
    }
}

/// Fill `p`'s geometry from a chain of points in raster pixels, trimmed to the
/// `start_pct`..`end_pct` share of its own drawn length (docs/08 §3.78).
///
/// The chain is one continuous line with [`PEN_UP`] lifts in it, and the trim is
/// **by distance along the line**, which is what makes Start and End behave like
/// a pen drawing the thing rather than like a pair of clipping planes.
///
/// Past [`PATH_PRIMITIVES`] the chain is **coarsened, not cut**: every n'th
/// vertex is kept and the chords between them drawn, so the whole shape still
/// appears, slightly straighter. A coarsened chain can also swallow a pen-up,
/// joining two strokes that should be apart — only reachable past the cap, and
/// the one producer of lifts widens its spacing to stay under it.
pub fn path_chain(pts: &[[f32; 2]], start_pct: f32, end_pct: f32, p: &mut PathDrawParams) {
    p.count = 0;
    if pts.len() < 2 {
        return;
    }
    let edges = pts.len() - 1;
    let stride = edges.div_ceil(PATH_PRIMITIVES).max(1);
    let kept = edges.div_ceil(stride);
    let at = |k: usize| pts[(k * stride).min(pts.len() - 1)];
    // A chord is drawn only when both its ends are real; a lift is skipped and
    // does not advance the distance along, because the pen is off the paper.
    let chord = |k: usize| -> Option<([f32; 4], f32)> {
        let (a, b) = (at(k), at(k + 1));
        if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) {
            return None;
        }
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        (len > 0.0).then_some(([a[0], a[1], b[0], b[1]], len))
    };
    let mut total = 0.0f32;
    for k in 0..kept {
        if let Some((_, len)) = chord(k) {
            total += len;
        }
    }
    let lo = (start_pct.min(end_pct) / 100.0).clamp(0.0, 1.0);
    let hi = (start_pct.max(end_pct) / 100.0).clamp(0.0, 1.0);
    let (s0, s1) = (total * lo, total * hi);
    let mut arc = 0.0f32;
    let mut n = 0usize;
    for k in 0..kept {
        let Some((c, len)) = chord(k) else {
            continue;
        };
        let (e0, e1) = (arc, arc + len);
        arc = e1;
        if e1 <= s0 || e0 >= s1 {
            continue;
        }
        let t0 = ((s0 - e0) / len).clamp(0.0, 1.0);
        let t1 = ((s1 - e0) / len).clamp(0.0, 1.0);
        if t1 <= t0 {
            continue;
        }
        let (dx, dy) = (c[2] - c[0], c[3] - c[1]);
        p.segments[n] = [
            c[0] + dx * t0,
            c[1] + dy * t0,
            c[0] + dx * t1,
            c[1] + dy * t1,
        ];
        p.arcs[n] = e0 + len * t0;
        n += 1;
        if n == PATH_PRIMITIVES {
            break;
        }
    }
    p.count = n as u32;
}

/// The wobble: where this pixel *really* samples the drawing from (docs/08
/// §3.78's second decision).
///
/// The scribble is not wobbled — the paper is. Displacing the sample point by a
/// smooth noise field costs one lookup a pixel and gives every stroke the same
/// hand-drawn waver, where wobbling the geometry would have cost eight times the
/// pieces to say the same thing. `wiggle_amp` 0 returns the point untouched, bit
/// for bit, which is what the other two consumers pass.
#[must_use]
pub fn path_draw_warp(px: f32, py: f32, p: &PathDrawParams) -> (f32, f32) {
    if p.wiggle_amp <= 0.0 {
        return (px, py);
    }
    let (qx, qy) = (px * p.wiggle_freq, py * p.wiggle_freq);
    let wx = value3(p.seed, 0, qx, qy, p.wiggle_tick, 0) * 2.0 - 1.0;
    let wy = value3(p.seed, 1, qx, qy, p.wiggle_tick, 0) * 2.0 - 1.0;
    (px + wx * p.wiggle_amp, py + wy * p.wiggle_amp)
}

/// One pixel's coverage — the expression both paths evaluate. A **maximum** over
/// the pieces, never a sum, for §3.74's reason: every joint is shared by two of
/// them and a sum would bead at each one.
#[must_use]
pub fn path_draw_sample(px: f32, py: f32, p: &PathDrawParams) -> f32 {
    let (qx, qy) = path_draw_warp(px, py, p);
    let mut cov = 0.0f32;
    for i in 0..p.count as usize {
        let s = &p.segments[i];
        let (dx, dy) = (s[2] - s[0], s[3] - s[1]);
        let (rx, ry) = (qx - s[0], qy - s[1]);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let t = ((rx * dx + ry * dy) / len2).clamp(0.0, 1.0);
        let (ox, oy) = (rx - t * dx, ry - t * dy);
        let d = (ox * ox + oy * oy).sqrt();
        let across = ((p.half_width - d) / p.band + 0.5).clamp(0.0, 1.0);
        if across <= 0.0 {
            continue;
        }
        // How far along the whole drawing this pixel's nearest point sits —
        // measured, not projected, which is the thing §3.76's third decision
        // could not do without a path to trace.
        let phase = (p.arcs[i] + t * len2.sqrt()) * p.inv_segment + p.phase;
        let frac = phase - phase.floor();
        let soft = (p.band * p.inv_segment).max(1e-4);
        let along = ((p.duty - frac) / soft + 0.5).clamp(0.0, 1.0);
        cov = cov.max(across * along);
    }
    cov * p.opacity
}

/// The shared path drawing (docs/08 §3.78, §3.79, §3.76) — the CPU reference and
/// §1.6 oracle for all three of its consumers. One coverage a pixel, then the
/// paint style.
pub fn path_draw(rgba: &mut [f32], w: u32, h: u32, p: &PathDrawParams) {
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            let cov = path_draw_sample(x as f32 + 0.5, y as f32 + 0.5, p);
            if p.style == PAINT_REVEAL_ORIGINAL {
                // The drawing is the matte: what it covers survives — colour and
                // coverage alike, which is what premultiplied means — and the
                // rest of the picture goes.
                for i in 0..4 {
                    let lit = rgba[d + i] * cov;
                    rgba[d + i] = rgba[d + i] * (1.0 - p.mix) + lit * p.mix;
                }
                continue;
            }
            let keep = if p.style == PAINT_ON_ORIGINAL {
                1.0 - cov
            } else {
                0.0
            };
            for i in 0..3 {
                let lit = rgba[d + i] * keep + p.colour[i] * cov;
                rgba[d + i] = rgba[d + i] * (1.0 - p.mix) + lit * p.mix;
            }
            let lit = rgba[d + 3] * keep + cov;
            rgba[d + 3] = rgba[d + 3] * (1.0 - p.mix) + lit * p.mix;
        }
    }
}

/// A mask polyline in **raster** pixels — the px@comp vertices K-408 hands over,
/// taken to the raster the frame is actually being drawn at (docs/08 §2.3).
///
/// Every consumer of the seam does this and none of them should do it
/// differently, which is the whole reason it is a function.
#[must_use]
pub fn path_points(poly: &crate::mask::MaskPolyline, px_scale: f32) -> Vec<[f32; 2]> {
    poly.points
        .iter()
        .map(|q| [q[0] * px_scale, q[1] * px_scale])
        .collect()
}

/// The most hatch strokes a Scribble may lay down: two chain points each, so
/// half the budget (docs/08 §3.78). Past it the **spacing widens** rather than
/// the fill stopping half way down the shape, which is the degradation that
/// keeps the picture whole (docs/14 §4).
pub const SCRIBBLE_MAX_STROKES: usize = PATH_PRIMITIVES / 2;

/// Build a scribble's chain: parallel strokes at `angle_deg` across the mask
/// `poly` (raster pixels), `spacing` apart, each run on past the edge by
/// `overlap`, joined end to end into one continuous line (docs/08 §3.78).
///
/// The line is where the pen goes, so its direction alternates — left to right,
/// then right to left — so the join between one stroke and the next is a short
/// hop along the edge rather than a flight back across the shape.
///
/// Points come back in raster pixels, with [`PEN_UP`] between strokes the pen
/// must not join: the two halves of one line across a mask with a notch in it.
#[must_use]
pub fn scribble_chain(
    poly: &[[f32; 2]],
    angle_deg: f32,
    spacing: f32,
    overlap: f32,
) -> Vec<[f32; 2]> {
    let mut chain: Vec<[f32; 2]> = Vec::new();
    if poly.len() < 3 {
        return chain;
    }
    let th = angle_deg.to_radians();
    // `u` runs along a stroke and `v` across them; orthonormal, so the point at
    // (across, along) is just `across·v + along·u`.
    let (ux, uy) = (th.cos(), th.sin());
    let (vx, vy) = (-uy, ux);
    let mut vmin = f32::INFINITY;
    let mut vmax = f32::NEG_INFINITY;
    for q in poly {
        let d = q[0] * vx + q[1] * vy;
        vmin = vmin.min(d);
        vmax = vmax.max(d);
    }
    if !vmin.is_finite() || !vmax.is_finite() || vmax <= vmin {
        return chain;
    }
    // Widen the spacing rather than run out of budget half way down the shape.
    let span = vmax - vmin;
    let step = spacing.max(0.5).max(span / SCRIBBLE_MAX_STROKES as f32);
    let lines = (span / step).floor().max(0.0) as i32;
    let mut hits: Vec<f32> = Vec::new();
    for k in 0..=lines {
        // Half a step in, so the outermost stroke is not laid exactly on the
        // shape's tangent line, where it would flicker as the shape animates.
        let off = vmin + step * 0.5 + step * k as f32;
        hits.clear();
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            let (da, db) = (a[0] * vx + a[1] * vy - off, b[0] * vx + b[1] * vy - off);
            // Half-open: an edge counts on one side only, so a vertex sitting
            // exactly on the line is crossed once rather than twice or never.
            if (da <= 0.0) == (db <= 0.0) {
                continue;
            }
            let t = da / (da - db);
            let (px, py) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            hits.push(px * ux + py * uy);
        }
        hits.sort_by(|x, y| x.partial_cmp(y).unwrap_or(core::cmp::Ordering::Equal));
        // Even-odd: inside the shape between the first crossing and the second,
        // the third and the fourth, and so on.
        //
        // On a reversed line the strokes are taken in reverse **order** as well
        // as reversed in themselves. Reversing only the strokes would leave the
        // pen at the far side of the shape and send it back across the whole
        // width — through the notch it just lifted over — to start the next line.
        let spans = hits.len() / 2;
        for j in 0..spans {
            let pair = if k % 2 == 0 { j } else { spans - 1 - j };
            if chain.len() + 3 > PATH_PRIMITIVES {
                return chain;
            }
            let (a, b) = (hits[pair * 2] - overlap, hits[pair * 2 + 1] + overlap);
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            let (a, b) = if k % 2 == 0 { (a, b) } else { (b, a) };
            // The pen joins only strokes that follow each other down the shape;
            // a second stroke on the same line is across a hole.
            if j > 0 {
                chain.push(PEN_UP);
            }
            chain.push([off * vx + a * ux, off * vy + a * uy]);
            chain.push([off * vx + b * ux, off * vy + b * uy]);
        }
    }
    chain
}

/// Fill `p`'s geometry with a brush stroke along `poly`, between the `start_pct`
/// and `end_pct` marks (docs/08 §3.79). `diameter` and `spacing` are raster
/// pixels; `px_scale` takes the seam's px@comp vertices to that raster.
///
/// **Why two shapes and not one.** A brush stroke is a row of round stamps, and
/// while they overlap their union *is* the path swept by the brush — drawing the
/// path directly is the same picture for a fraction of the pieces, and it is the
/// only form that stays inside the budget on a long path with a fine brush. Once
/// the stamps are further apart than the brush is wide they stop being a stroke
/// and become a dotted line, which is what the control is for, and then they are
/// drawn as what they are. The changeover is at **half the brush width**, where
/// the deepest scallop between two stamps is an eighth of the radius — under a
/// pixel for any brush you would notice it on.
pub fn stroke_geometry(
    poly: &crate::mask::MaskPolyline,
    px_scale: f32,
    diameter: f32,
    spacing: f32,
    start_pct: f32,
    end_pct: f32,
    p: &mut PathDrawParams,
) {
    p.count = 0;
    if poly.is_empty() {
        return;
    }
    let scale = px_scale.max(1e-6);
    if spacing <= diameter * 0.5 {
        path_chain(&path_points(poly, scale), start_pct, end_pct, p);
        return;
    }
    let total = poly.length() * scale;
    let lo = (start_pct.min(end_pct) / 100.0).clamp(0.0, 1.0) * total;
    let hi = (start_pct.max(end_pct) / 100.0).clamp(0.0, 1.0) * total;
    let span = hi - lo;
    let mut step = spacing.max(1e-3);
    // Past the budget the dots space out; the trail still reaches the End mark,
    // which is the failure nobody notices over one that stops half way.
    if span / step + 1.0 > PATH_PRIMITIVES as f32 {
        step = span / (PATH_PRIMITIVES - 1) as f32;
    }
    let n = ((span / step.max(1e-3)).floor() as usize + 1).min(PATH_PRIMITIVES);
    for i in 0..n {
        let arc = lo + step * i as f32;
        let q = poly.point_at(arc / scale);
        let (x, y) = (q[0] * scale, q[1] * scale);
        // A stamp is a piece with no length: the capsule's round cap is the
        // brush, so one expression draws both shapes.
        p.segments[i] = [x, y, x, y];
        p.arcs[i] = arc;
    }
    p.count = n as u32;
}
