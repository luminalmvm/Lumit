//! The Correction over a whole picture on the CPU: the oracle the GPU passes
//! are held to, pass for pass.

use super::maths;
use super::packed::Packed;
use crate::fx::cpu;

/// The picture in the grading space, unpremultiplied, alpha kept: what the
/// Denoise and Midtone detail blurs read.
fn v_in_picture(rgba: &[f32], space: u32) -> Vec<f32> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        let v = maths::v_in([px[0], px[1], px[2], px[3]], space);
        px[..3].copy_from_slice(&v);
    }
    out
}

/// The node's key, one number a pixel, or `None` when nothing keys and
/// every pixel takes the whole grade. `matte` is the prepared matte, empty
/// when none is bound.
#[must_use]
pub fn correction_key(
    rgba: &[f32],
    w: u32,
    h: u32,
    pk: &Packed,
    matte: &[f32],
) -> Option<Vec<f32>> {
    let qual_on = pk.qualifier_on();
    if !qual_on && !pk.windows_on() && matte.is_empty() {
        return None;
    }
    let q = &pk.qual;
    // The qualifier's key, the same number in every channel, as the GPU's.
    let qual = qual_on.then(|| {
        let source = if q.denoise_px > 0.0 {
            let mut v = v_in_picture(rgba, pk.space);
            cpu::blur_gaussian(&mut v, w, h, q.denoise_px, 1, 1.0);
            v
        } else {
            v_in_picture(rgba, pk.space)
        };
        let mut k: Vec<f32> = Vec::with_capacity(source.len());
        for px in source.chunks_exact(4) {
            let v = maths::qualify([px[0], px[1], px[2]], q);
            k.extend_from_slice(&[v; 4]);
        }
        if q.grow_px != 0.0 {
            cpu::matte_morph(&mut k, w, h, q.grow_px);
        }
        if q.blur_px > 0.0 {
            cpu::blur_gaussian(&mut k, w, h, q.blur_px, 1, 1.0);
        }
        k
    });
    let mut key = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let mut k = maths::windows([x as f32 + 0.5, y as f32 + 0.5], &pk.windows, &pk.data);
            if let Some(qk) = &qual {
                let v = qk.get(i * 4).copied().unwrap_or(1.0);
                k *= if q.invert { 1.0 - v } else { v };
            }
            if let Some(m) = matte.get(i * 4..i * 4 + 3) {
                k *= maths::luma([m[0], m[1], m[2]]).clamp(0.0, 1.0);
            }
            key.push(k);
        }
    }
    Some(key)
}

/// What the Viewer's matte view draws in mid grey, scene linear.
pub const MATTE_VIEW_GREY: f32 = 0.18;

/// The node's matte view over `rgba`: `view` 2 is the key in black and
/// white, opaque, and the rest the graded picture where the key is over a
/// backdrop ([`matte_shown`]). The twin of the GPU's: the key pass, then the
/// backdrop and the matte dissolve, or the matte prepare for 2.
pub fn matte_view(rgba: &mut [f32], w: u32, h: u32, pk: &Packed, view: u32) {
    let key = correction_key(rgba, w, h, pk, &[]);
    if view != 2 {
        correction(rgba, w, h, pk, &[]);
    }
    matte_shown(rgba, key.as_deref(), view);
}

/// What Highlight shows where the key is not, drawn from the node's picture:
/// grey for 1, black for 3, white for 4, and for 5 the picture in black and
/// white (Tint at its rest, which maps each pixel to its luma).
pub fn matte_backdrop(rgba: &mut [f32], view: u32) {
    match view {
        3 => cpu::fill(rgba, [0.0; 3], 1.0),
        4 => cpu::fill(rgba, [1.0; 3], 1.0),
        5 => cpu::tint(rgba, [0.0; 3], [1.0; 3], 1.0),
        _ => cpu::fill(rgba, [MATTE_VIEW_GREY; 3], 1.0),
    }
}

/// The matte view drawn over `rgba`, the picture the node made, with its
/// `key` (`None` keys every pixel whole).
pub fn matte_shown(rgba: &mut [f32], key: Option<&[f32]>, view: u32) {
    let back = (view != 2).then(|| {
        let mut back = rgba.to_vec();
        matte_backdrop(&mut back, view);
        back
    });
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let k = key
            .map_or(1.0, |k| k.get(i).copied().unwrap_or(1.0))
            .clamp(0.0, 1.0);
        let Some(b) = back.as_ref().and_then(|b| b.get(i * 4..i * 4 + 4)) else {
            px.copy_from_slice(&[k, k, k, 1.0]);
            continue;
        };
        for c in 0..4 {
            px[c] = b[c] * (1.0 - k) + px[c] * k;
        }
    }
}

/// The Correction over `rgba` (premultiplied, four floats a pixel). `matte` is
/// the prepared matte, empty when none is bound.
pub fn correction(rgba: &mut [f32], w: u32, h: u32, pk: &Packed, matte: &[f32]) {
    if pk.neutral {
        return;
    }
    let key = correction_key(rgba, w, h, pk, matte);
    let detail = (pk.detail != 0.0).then(|| {
        let mut v = v_in_picture(rgba, pk.space);
        cpu::blur_gaussian(&mut v, w, h, pk.detail_px, 1, 1.0);
        v
    });
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let k = key
            .as_ref()
            .map_or(1.0, |k| k.get(i).copied().unwrap_or(1.0));
        let d = detail.as_ref().map_or(0.0, |d| {
            d.get(i * 4..i * 4 + 3)
                .map_or(0.0, |v| maths::luma([v[0], v[1], v[2]]))
        });
        let out = maths::grade([px[0], px[1], px[2], px[3]], k, d, pk);
        px.copy_from_slice(&out);
    }
}
