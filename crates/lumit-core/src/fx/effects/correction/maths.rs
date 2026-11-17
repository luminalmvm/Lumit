//! The Correction's per-pixel maths, the CPU twin of `fx_correction.wgsl`.
//!
//! **In plain terms.** One pixel at a time: lift it into the grading space,
//! work out how much of the grade it gets (the key), grade it, and mix the
//! two by the key. Every function here has a WGSL twin of the same name that
//! does the same arithmetic in the same order, and both read the numbers
//! [`Packed`](super::packed::Packed) made, so the two paths cannot drift.
//!
//! # The formulas
//!
//! Notation: `o` is the premultiplied pixel, `a` its alpha, `u = o.rgb / a`
//! (black where `a = 0`). `L = (0.2126, 0.7152, 0.0722)` and `luma(v) = L·v`.
//! `sgnpow(x, e) = sign(x)·|x|^e`, and nought where `x = 0`.
//! `ss(e0, e1, x)` is smoothstep, a hard step at `e0` when `e1 <= e0`.
//! `C = 65504` bounds every graded value, so nothing becomes infinite.
//!
//! **1. Space.** `E` encodes scene linear into the grading space, `D` decodes:
//! - Video: `E(x) = sgnpow(x, 1/2.4)`, `D(y) = sgnpow(y, 2.4)`.
//! - Log (ACEScct): `E(x) = 10.5402377416545·x + 0.0729055341958355` for
//!   `x <= 0.0078125`, else `(log2(x) + 9.72) / 17.52`; `D` is its inverse,
//!   with `y` held at or below 1.4679963 so `D(y) <= C`.
//! - Linear: both are the identity.
//!
//! `v_in = E(u)`: the picture as it came, what the key is judged on and what
//! the grade is mixed back towards.
//!
//! **2. Key.** `key = matte · qualifier · windows`, each 1 when off.
//! - Matte: `clamp(luma(m), 0, 1)` of the prepared matte.
//! - YCbCr of a value `v`: `Y = luma(v)`, `Cb = (v.b − Y)/1.8556`,
//!   `Cr = (v.r − Y)/1.5748`. Hue `h = fract((atan2(Cr, Cb) − 1.7960518)/2π)`,
//!   red 0 and rising red, yellow, green, cyan, blue, magenta (the
//!   vectorscope's angle), and 0 where `Cb² + Cr² < 1e-12`. Saturation
//!   `s = max(v) − min(v)`, so each primary and secondary of a 0..1 value
//!   reads 1.
//! - `band(x, lo, hi, soft) = low · high`, `low = 1` when `lo <= 0` else
//!   `ss(lo − soft, lo, x)`, `high = 1` when `hi >= 1` else
//!   `1 − ss(hi, hi + soft, x)`.
//! - HSL, on `v_in` (or on `v_in` blurred by Denoise): hue distance
//!   `d = fract(h − centre + 0.5) − 0.5` (turns); half widths
//!   `lo = W/2·(1 − sym)`, `hi = W/2·(1 + sym)`; `k_h = 1 − ss(hi, hi + S, d)`
//!   for `d >= 0` and `1 − ss(lo, lo + S, −d)` below. `k_s = band(s, …)`,
//!   `k_l = band(Y, …)`. `k = k_h·k_s·k_l`, a factor left out when its Use
//!   switch is off. RGB: `band` on each channel, multiplied. Luma: `band(Y)`.
//! - 3D, on `v_in`: near a sample `n = 1 − ss(size, size + soft, d)`, `d` the
//!   distance in `(Y/2, Cb, Cr)`, size and soft over 100 times 0.25.
//!   `k = keep · (1 − drop)`, each the most `n` over its samples; with no kept
//!   sample `keep = 1`, with no dropped one `drop = 0`. The samples are the
//!   rows `qual_3d_keep` and `qual_3d_drop` (`2Cb, 2Cr` a point) and
//!   `qual_3d_keep_luma` and `qual_3d_drop_luma` (`Y` the first number).
//! - Clean: `k = clamp((k − 0.49·black)/(1 − 0.49·white − 0.49·black), 0, 1)`
//!   (black, white as 0..1). Clip: `k = min(max(k, black clip), white clip)`.
//!   Then Grow (a min or max over a square, as the Matte key's), Blur (the
//!   Gaussian blur), and Invert `k = 1 − k`.
//! - Windows, in the window's box: `q = R(−angle)(p − centre)`, `p` the pixel
//!   centre in raster pixels, `(w/2, h/2)` the half size. Signed distance `d`
//!   (negative inside): ellipse `k0(k0 − 1)/k1`, `k0 = |q/r|`, `k1 = |q/r²|`;
//!   rectangle the rounded box with radius `round·min(w, h)/2`; polygon the
//!   distance to the outline through the points `(x·w/2, y·h/2)` (straight, or
//!   a closed Catmull-Rom curve cut into four lines a span), signed by the
//!   even-odd rule; gradient `d = q.y`, so at angle 0 the part above the
//!   centre line is inside. A polygon of fewer than three points is off.
//!   Weight `w = 1 − ss(−soft in, soft out, d)`, then `1 − w` when inverted,
//!   then times opacity. Slots combine in order from 0 (from 1 when the first
//!   window on subtracts): `acc = max(acc, w)`, or `acc·(1 − w)` to subtract.
//!
//! **3. Grade**, on `v = E(Z(u · 2^exposure · g))`, `g` Temperature's gains
//! `(max(1 + 0.75t, 0), max(1 − 0.75k, 0), max(1 − 0.75t, 0))`, `t` and `k`
//! the Temperature and Tint over 100, and `Z` the HDR zones
//! ([`hdr`](super::hdr)), still in scene linear. Then in this order:
//! 1. `y0 = luma(v)`; `v += offset`; `v += lift·(1 − v)`;
//!    `v = sgnpow(v, 2^(−2·gamma))`; `v *= gain`.
//! 2. Log wheels: `y = luma(v)`, `s = ss(low − 0.1, low + 0.1, y)`,
//!    `h = ss(high − 0.1, high + 0.1, y)`;
//!    `v += (1 − s)·shadow + (s − h)·midtone + h·highlight`; held to ±C.
//! 3. Lum mix: `v += (1 − lum mix)·(y0 − luma(v))`.
//! 4. Contrast: `v = (v − pivot)·contrast + pivot`.
//! 5. Shadows and highlights: `y = luma(v)`;
//!    `v += 0.25·shadows·(1 − ss(0, 0.5, y)) + 0.25·highlights·ss(0.5, 1, y)`
//!    (both over 100).
//! 6. Mixer: `v = M·v`, rows Red, Green, Blue output; with Keep luma
//!    `v += luma(before) − luma(v)`.
//! 7. Hue, saturation, boost: in YCbCr, `(Cb, Cr)` turned by Hue, then scaled
//!    by `saturation·(1 + boost·(1 − clamp(s, 0, 1)))`, `s` read before the
//!    turn and boost over 100. Y is untouched.
//! 8. Midtone detail: `v += detail·(luma(v_in) − luma(blur(v_in)))·
//!    clamp(4·y·(1 − y), 0, 1)`, `y = luma(v)`, detail over 100, the blur
//!    20 px at comp size.
//! 9. Curves: `v_c = Y(C_c(v_c))`, each a 257-entry table read with linear
//!    interpolation, extrapolating past either end.
//! 10. Soft clip: above `kh = 1 − 0.5·high`, `t = (v − kh)/(1 − kh)`,
//!     `v = kh + (1 − kh)·t/(1 + t)`; below `kl = 0.5·low`,
//!     `t = (kl − v)/kl`, `v = kl − kl·t/(1 + t)` (high, low over 100).
//! 11. Hue curves, all read at the YCbCr of `v` before any of them moves it
//!     (`h`, `s` held to 0..1, `Y` held to 0..1 for the lookup):
//!     turn `(Cb, Cr)` by `(HvH(h) − 0.5)` turns; scale it by
//!     `2HvS(h) · 2LvS(Y) · 2SvS(s)`; `Y += 0.5·s·(HvL(h) − 0.5 + SvL(s) − 0.5)`.
//!     The hue curves are periodic: their tables are baked wrapped.
//! 12. Held to ±C.
//!
//! **4. Out.** `k = key·mix`; `w = v_in·(1 − k) + v·k`; `out = D(w)` held to
//! ±C, times `a`; alpha unchanged. A Correction whose every grade row is at
//! rest, or whose Mix is 0, hands the picture on untouched.

use super::packed::{Packed, Qualifier, Window, POLY_AT, TABLE};

/// Rec. 709 luma weights, as the rest of the colour family reads them.
pub const LUMA: [f32; 3] = crate::fx::cpu::LUMA;

/// The biggest value a graded channel may hold: fp16's largest, so a stored
/// picture never meets infinity.
pub const BOUND: f32 = 65504.0;

/// The Cb/Cr angle of pure red, so red reads hue 0.
pub const RED_ANGLE: f32 = 1.796_051_8;

/// ACEScct's top, the encoded value of [`BOUND`].
const LOG_TOP: f32 = 1.467_996_3;

/// `sign(x)·|x|^e`, and nought at nought.
#[must_use]
pub fn sgnpow(x: f32, e: f32) -> f32 {
    if x == 0.0 {
        0.0
    } else if x > 0.0 {
        x.powf(e)
    } else {
        -(-x).powf(e)
    }
}

/// Smoothstep, a hard step at `e0` when the edges meet or cross.
#[must_use]
pub fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 > e0 {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    } else if x < e0 {
        0.0
    } else {
        1.0
    }
}

/// Held to ±[`BOUND`].
#[must_use]
pub fn held(v: [f32; 3]) -> [f32; 3] {
    v.map(|c| c.clamp(-BOUND, BOUND))
}

/// Scene linear into the grading space.
#[must_use]
pub fn encode(x: f32, space: u32) -> f32 {
    match space {
        0 => sgnpow(x, 1.0 / 2.4),
        1 => {
            if x <= 0.007_812_5 {
                10.540_237 * x + 0.072_905_53
            } else {
                (x.log2() + 9.72) / 17.52
            }
        }
        _ => x,
    }
}

/// The grading space back to scene linear.
#[must_use]
pub fn decode(y: f32, space: u32) -> f32 {
    match space {
        0 => sgnpow(y, 2.4),
        1 => {
            if y <= 0.155_251_14 {
                (y - 0.072_905_53) / 10.540_237
            } else {
                (y.min(LOG_TOP) * 17.52 - 9.72).exp2()
            }
        }
        _ => y,
    }
}

/// Rec. 709 luma.
#[must_use]
pub fn luma(v: [f32; 3]) -> f32 {
    v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2]
}

/// `Y, Cb, Cr` of a value.
#[must_use]
pub fn ycc(v: [f32; 3]) -> [f32; 3] {
    let y = luma(v);
    [y, (v[2] - y) / 1.8556, (v[0] - y) / 1.5748]
}

/// The value with this `Y, Cb, Cr`.
#[must_use]
pub fn rgb_of(c: [f32; 3]) -> [f32; 3] {
    let r = c[0] + 1.5748 * c[2];
    let b = c[0] + 1.8556 * c[1];
    let g = (c[0] - LUMA[0] * r - LUMA[2] * b) / LUMA[1];
    [r, g, b]
}

/// The vectorscope hue of `Cb, Cr`, 0..1 with red at 0.
#[must_use]
pub fn hue_of(cb: f32, cr: f32) -> f32 {
    if cb * cb + cr * cr < 1e-12 {
        return 0.0;
    }
    let t = (cr.atan2(cb) - RED_ANGLE) / std::f32::consts::TAU;
    t - t.floor()
}

/// Saturation as the qualifier and the curves read it: `max − min`.
#[must_use]
pub fn chroma(v: [f32; 3]) -> f32 {
    v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2])
}

/// `(Cb, Cr)` turned by the angle whose cosine and sine are `rot`.
#[must_use]
pub fn turn(cb: f32, cr: f32, rot: [f32; 2]) -> [f32; 2] {
    [cb * rot[0] - cr * rot[1], cb * rot[1] + cr * rot[0]]
}

/// A soft range: 1 inside `lo..hi`, falling to 0 over `soft` outside it. An
/// end at or past 0 or 1 is open.
#[must_use]
pub fn band(x: f32, lo: f32, hi: f32, soft: f32) -> f32 {
    let low = if lo <= 0.0 { 1.0 } else { ss(lo - soft, lo, x) };
    let high = if hi >= 1.0 {
        1.0
    } else {
        1.0 - ss(hi, hi + soft, x)
    };
    low * high
}

/// Entry `i` of table `t` in `data`.
fn entry(data: &[f32], t: usize, i: usize) -> f32 {
    data.get(t * TABLE + i).copied().unwrap_or(0.0)
}

/// Table `t` at `x`, read as `cpu::curve_at` reads a curve: linear between
/// entries, carrying on along the end segments outside 0..1.
#[must_use]
pub fn table_at(data: &[f32], t: usize, x: f32) -> f32 {
    let last = (TABLE - 1) as f32;
    let s = x * last;
    let fi = s.clamp(0.0, last - 1.0).floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = (fi as usize).min(TABLE - 2);
    let f = s - fi;
    let a = entry(data, t, i);
    a + (entry(data, t, i + 1) - a) * f
}

/// The qualifier's key for `v` (already in the grading space), before Grow,
/// Blur and Invert.
#[must_use]
pub fn qualify(v: [f32; 3], q: &Qualifier) -> f32 {
    let k = match q.mode {
        1 => {
            let c = ycc(v);
            let mut k = 1.0;
            if q.use_hue {
                let h = hue_of(c[1], c[2]);
                let t = h - q.hue + 0.5;
                let d = t - t.floor() - 0.5;
                k *= if d >= 0.0 {
                    1.0 - ss(q.hue_hi, q.hue_hi + q.hue_soft, d)
                } else {
                    1.0 - ss(q.hue_lo, q.hue_lo + q.hue_soft, -d)
                };
            }
            if q.use_sat {
                k *= band(chroma(v), q.sat[0], q.sat[1], q.sat[2]);
            }
            if q.use_lum {
                k *= band(c[0], q.lum[0], q.lum[1], q.lum[2]);
            }
            k
        }
        2 => (0..3)
            .map(|c| band(v[c], q.rgb_lo[c], q.rgb_hi[c], q.rgb_soft))
            .product(),
        3 => band(luma(v), q.lum[0], q.lum[1], q.lum[2]),
        4 => {
            let c = ycc(v);
            let near = |s: &[f32; 3]| {
                let d = [0.5 * (c[0] - s[0]), c[1] - s[1], c[2] - s[2]];
                let d = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                1.0 - ss(q.reach[0], q.reach[0] + q.reach[1], d)
            };
            let most = |at: &[[f32; 3]], n: u32| {
                at.iter().take(n as usize).map(near).fold(0.0f32, f32::max)
            };
            let keep = if q.samples[0] == 0 {
                1.0
            } else {
                most(&q.keep_at, q.samples[0])
            };
            keep * (1.0 - most(&q.drop_at, q.samples[1]))
        }
        _ => 1.0,
    };
    let k = ((k - q.clean[0]) / (q.clean[1] - q.clean[0])).clamp(0.0, 1.0);
    k.max(q.clip[0]).min(q.clip[1])
}

/// The signed distance from `p` (local, raster pixels) to the polygon whose
/// vertices sit in `data`, negative inside.
fn polygon(p: [f32; 2], w: &Window, data: &[f32]) -> f32 {
    let at = |i: u32| {
        let k = POLY_AT + 2 * (w.poly[0] + i) as usize;
        [
            data.get(k).copied().unwrap_or(0.0),
            data.get(k + 1).copied().unwrap_or(0.0),
        ]
    };
    let n = w.poly[1];
    let mut best = f32::MAX;
    let mut sign = 1.0f32;
    let mut j = n.saturating_sub(1);
    for i in 0..n {
        let (vi, vj) = (at(i), at(j));
        let e = [vj[0] - vi[0], vj[1] - vi[1]];
        let d = [p[0] - vi[0], p[1] - vi[1]];
        let ee = (e[0] * e[0] + e[1] * e[1]).max(1e-12);
        let t = ((d[0] * e[0] + d[1] * e[1]) / ee).clamp(0.0, 1.0);
        let b = [d[0] - e[0] * t, d[1] - e[1] * t];
        best = best.min(b[0] * b[0] + b[1] * b[1]);
        let c0 = p[1] >= vi[1];
        let c1 = p[1] < vj[1];
        let c2 = e[0] * d[1] > e[1] * d[0];
        if (c0 && c1 && c2) || (!c0 && !c1 && !c2) {
            sign = -sign;
        }
        j = i;
    }
    sign * best.sqrt()
}

/// The signed distance from `q` (local, raster pixels) to window `w`'s shape.
fn shape_distance(q: [f32; 2], w: &Window, data: &[f32]) -> f32 {
    match w.shape {
        1 => {
            let r = [w.half[0].max(1e-3), w.half[1].max(1e-3)];
            let a = [q[0] / r[0], q[1] / r[1]];
            let b = [a[0] / r[0], a[1] / r[1]];
            let k0 = (a[0] * a[0] + a[1] * a[1]).sqrt();
            let k1 = (b[0] * b[0] + b[1] * b[1]).sqrt();
            if k1 < 1e-12 {
                -r[0].min(r[1])
            } else {
                k0 * (k0 - 1.0) / k1
            }
        }
        2 => {
            let r = w.round;
            let d = [q[0].abs() - w.half[0] + r, q[1].abs() - w.half[1] + r];
            let out = [d[0].max(0.0), d[1].max(0.0)];
            (out[0] * out[0] + out[1] * out[1]).sqrt() + d[0].max(d[1]).min(0.0) - r
        }
        3 => {
            // Past the outline's bounds by more than the outer softness, the
            // weight is nought whatever the outline does, so skip the walk.
            let bx = [
                (w.bbox[0] - q[0]).max(q[0] - w.bbox[2]),
                (w.bbox[1] - q[1]).max(q[1] - w.bbox[3]),
            ];
            if bx[0].max(bx[1]) > w.soft_out {
                bx[0].max(bx[1])
            } else {
                polygon(q, w, data)
            }
        }
        _ => q[1],
    }
}

/// How much of window `w` the pixel centre `p` (raster pixels) is in, 0..1,
/// after Invert and Opacity.
#[must_use]
pub fn window_weight(p: [f32; 2], w: &Window, data: &[f32]) -> f32 {
    let dx = p[0] - w.centre[0];
    let dy = p[1] - w.centre[1];
    let q = [w.rot[0] * dx + w.rot[1] * dy, w.rot[0] * dy - w.rot[1] * dx];
    let d = shape_distance(q, w, data);
    let mut k = 1.0 - ss(-w.soft_in, w.soft_out, d);
    if w.invert {
        k = 1.0 - k;
    }
    k * w.opacity
}

/// The four windows together, in slot order; 1 when none is on.
#[must_use]
pub fn windows(p: [f32; 2], ws: &[Window; 4], data: &[f32]) -> f32 {
    let mut acc = -1.0f32;
    for w in ws.iter().filter(|w| w.shape != 0) {
        if acc < 0.0 {
            acc = if w.subtract { 1.0 } else { 0.0 };
        }
        let k = window_weight(p, w, data);
        acc = if w.subtract {
            acc * (1.0 - k)
        } else {
            acc.max(k)
        };
    }
    if acc < 0.0 {
        1.0
    } else {
        acc
    }
}

/// The unpremultiplied colour of a premultiplied pixel, black where it is
/// clear.
#[must_use]
pub fn unpremult(o: [f32; 4]) -> [f32; 3] {
    if o[3] > 0.0 {
        [o[0] / o[3], o[1] / o[3], o[2] / o[3]]
    } else {
        [0.0; 3]
    }
}

/// The picture as it came, in the grading space.
#[must_use]
pub fn v_in(o: [f32; 4], space: u32) -> [f32; 3] {
    unpremult(o).map(|c| encode(c, space))
}

/// One pixel graded and mixed back by `key`. `detail` is the luma of the
/// blurred `v_in` under it, read only when Midtone detail is on.
#[must_use]
pub fn grade(o: [f32; 4], key: f32, detail: f32, pk: &Packed) -> [f32; 4] {
    let space = pk.space;
    let u = unpremult(o);
    let vin = u.map(|c| encode(c, space));
    let mut ug = [0, 1, 2].map(|c| u[c] * pk.pre_gain[c]);
    if pk.hdr.on {
        ug = super::hdr::zones(ug, &pk.hdr);
    }
    let mut v = ug.map(|c| encode(c, space));

    // Wheels.
    let y0 = luma(v);
    v = [0, 1, 2].map(|c| {
        let x = v[c] + pk.offset[c];
        let x = x + pk.lift[c] * (1.0 - x);
        sgnpow(x, pk.gamma[c]) * pk.gain[c]
    });
    let y = luma(v);
    let s = ss(pk.range[0] - 0.1, pk.range[0] + 0.1, y);
    let h = ss(pk.range[1] - 0.1, pk.range[1] + 0.1, y);
    v = [0, 1, 2]
        .map(|c| v[c] + (1.0 - s) * pk.shadow[c] + (s - h) * pk.midtone[c] + h * pk.highlight[c]);
    v = held(v);
    let back = (1.0 - pk.lum_mix) * (y0 - luma(v));
    v = v.map(|c| c + back);

    // Adjust.
    v = v.map(|c| (c - pk.pivot) * pk.contrast + pk.pivot);
    let y = luma(v);
    let tonal = pk.tonal[0] * (1.0 - ss(0.0, 0.5, y)) + pk.tonal[1] * ss(0.5, 1.0, y);
    v = v.map(|c| c + tonal);
    if pk.mixer_on {
        let before = luma(v);
        let m = &pk.mixer;
        v = [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]);
        if pk.keep_luma {
            let back = before - luma(v);
            v = v.map(|c| c + back);
        }
    }
    let sat = chroma(v).clamp(0.0, 1.0);
    let c = ycc(v);
    let f = pk.sat * (1.0 + pk.boost * (1.0 - sat));
    let t = turn(c[1], c[2], pk.hue);
    v = rgb_of([c[0], t[0] * f, t[1] * f]);
    if pk.detail != 0.0 {
        let y = luma(v);
        let lift = pk.detail * (luma(vin) - detail) * (4.0 * y * (1.0 - y)).clamp(0.0, 1.0);
        v = v.map(|c| c + lift);
    }

    // Curves.
    if pk.curves_on {
        v = [0, 1, 2].map(|c| table_at(&pk.data, 0, table_at(&pk.data, c + 1, v[c])));
    }
    let (kl, kh) = (pk.knees[0], pk.knees[1]);
    for c in v.iter_mut() {
        if kh < 1.0 && *c > kh {
            let t = (*c - kh) / (1.0 - kh);
            *c = kh + (1.0 - kh) * t / (1.0 + t);
        }
        if kl > 0.0 && *c < kl {
            let t = (kl - *c) / kl;
            *c = kl - kl * t / (1.0 + t);
        }
    }
    if pk.hue_curves_on {
        let c = ycc(v);
        let hue = hue_of(c[1], c[2]);
        let s = chroma(v).clamp(0.0, 1.0);
        let yl = c[0].clamp(0.0, 1.0);
        let at = |t: usize, x: f32| table_at(&pk.data, t, x);
        let spin = (at(4, hue) - 0.5) * std::f32::consts::TAU;
        let f = 2.0 * at(5, hue) * 2.0 * at(7, yl) * 2.0 * at(8, s);
        let dy = 0.5 * s * (at(6, hue) - 0.5 + at(9, s) - 0.5);
        let t = turn(c[1], c[2], [spin.cos(), spin.sin()]);
        v = rgb_of([c[0] + dy, t[0] * f, t[1] * f]);
    }
    v = held(v);

    // Out.
    let k = key * pk.mix;
    let a = o[3];
    let out = [0, 1, 2].map(|c| {
        let w = vin[c] * (1.0 - k) + v[c] * k;
        decode(w, space).clamp(-BOUND, BOUND) * a
    });
    [out[0], out[1], out[2], a]
}
