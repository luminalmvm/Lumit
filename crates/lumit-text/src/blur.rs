//! A letter's own blur, for an animator's Blur: the letter is drawn alone
//! into a scratch raster, blurred there and laid over the others.

/// How far the blur of `radius` px reaches, in px: three box passes of half
/// width `b` reach `3 b`.
fn half_width(radius: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // px
    let b = (radius * 0.5).round().max(1.0) as usize;
    b.min(256)
}


/// One running-sum box pass of half width `b` over `n` samples `stride`
/// apart, four channels each, outside the run counting as nothing.
fn box_pass(buf: &mut [f32], tmp: &mut Vec<f32>, start: usize, n: usize, stride: usize, b: usize) {
    tmp.clear();
    tmp.extend((0..n).flat_map(|i| {
        let at = (start + i * stride) * 4;
        [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]
    }));
    #[allow(clippy::cast_precision_loss)] // a short window
    let scale = 1.0 / (2 * b + 1) as f32;
    let mut sum = [0.0f32; 4];
    // The window round sample 0: samples 0..=b.
    for i in 0..=b.min(n.saturating_sub(1)) {
        for c in 0..4 {
            sum[c] += tmp[i * 4 + c];
        }
    }
    for i in 0..n {
        let at = (start + i * stride) * 4;
        for c in 0..4 {
            buf[at + c] = sum[c] * scale;
        }
        // Slide: add i + b + 1, take i − b away.
        let add = i + b + 1;
        if add < n {
            for c in 0..4 {
                sum[c] += tmp[add * 4 + c];
            }
        }
        if i >= b {
            let sub = i - b;
            for c in 0..4 {
                sum[c] -= tmp[sub * 4 + c];
            }
        }
    }
}

/// Blur what `scratch` holds inside `rect` (left, top, right, bottom, the
/// ink before blurring) by `radius` px, lay it over `onto`, and leave
/// `scratch` clear again. Both are straight-alpha RGBA, `w` × `h`.
pub(crate) fn blur_over(
    scratch: &mut [u8],
    onto: &mut [u8],
    (w, h): (u32, u32),
    rect: [u32; 4],
    radius: f32,
) {
    let b = half_width(radius);
    let r = 3 * b;
    let (w, h) = (w as usize, h as usize);
    let x0 = (rect[0] as usize).saturating_sub(r);
    let y0 = (rect[1] as usize).saturating_sub(r);
    let x1 = (rect[2] as usize + r).min(w);
    let y1 = (rect[3] as usize + r).min(h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (rw, rh) = (x1 - x0, y1 - y0);
    // Premultiplied, so a see-through edge does not darken.
    let mut buf = vec![0.0f32; rw * rh * 4];
    for y in 0..rh {
        for x in 0..rw {
            let i = ((y0 + y) * w + x0 + x) * 4;
            let a = f32::from(scratch[i + 3]) / 255.0;
            let o = (y * rw + x) * 4;
            buf[o] = f32::from(scratch[i]) * a;
            buf[o + 1] = f32::from(scratch[i + 1]) * a;
            buf[o + 2] = f32::from(scratch[i + 2]) * a;
            buf[o + 3] = a;
            scratch[i..i + 4].fill(0);
        }
    }
    let mut tmp = Vec::new();
    for _ in 0..3 {
        for y in 0..rh {
            box_pass(&mut buf, &mut tmp, y * rw, rw, 1, b);
        }
        for x in 0..rw {
            box_pass(&mut buf, &mut tmp, x, rh, rw, b);
        }
    }
    for y in 0..rh {
        for x in 0..rw {
            let o = (y * rw + x) * 4;
            let a = buf[o + 3].clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let i = ((y0 + y) * w + x0 + x) * 4;
            let under = f32::from(onto[i + 3]) / 255.0;
            let out = a + under * (1.0 - a);
            for c in 0..3 {
                let mixed = (buf[o + c] + f32::from(onto[i + c]) * under * (1.0 - a)) / out;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    onto[i + c] = mixed.round().clamp(0.0, 255.0) as u8;
                }
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                onto[i + 3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// The 2 × 2 that slants a letter by `skew` degrees along a line `axis`
/// degrees round from level, in y-down pixels, as `[xx, yx, xy, yy]`: x' =
/// xx x + xy y, y' = yx x + yy y. A positive skew leans the letter's top to
/// the right. `None` for no slant.
#[must_use]
pub(crate) fn skew_matrix(skew: f32, axis: f32) -> Option<[f32; 4]> {
    if skew == 0.0 || !skew.is_finite() || !axis.is_finite() {
        return None;
    }
    let k = -skew.clamp(-85.0, 85.0).to_radians().tan();
    let (s, c) = axis.to_radians().sin_cos();
    Some([1.0 - c * s * k, -s * s * k, c * c * k, 1.0 + s * c * k])
}
