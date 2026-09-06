//! Byte-level colour helpers shared by every path that hands sRGB pixels to
//! the GPU or a colour picker.
//!
//! # In plain terms
//!
//! These are the small, boring conversions everything else leans on: turning a
//! scene-linear colour into the sRGB bytes a screen or a texture upload wants,
//! crossfading two frames, fitting one image inside another, and working out
//! *which* source frame of a clip a given moment lands on. None of it needs
//! FFmpeg or a graphics card, so it lives in the engine root where every crate
//! — the render pipeline, both frontends, the bridge — can reach it, including
//! in a build with no media support at all (the Project panel still has to draw
//! a solid's colour swatch).

use crate::model::LinearColour;

pub fn srgb_encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let e = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (e * 255.0).round() as u8
}

/// Inverse of [`srgb_encode`] (colour pickers hand back sRGB bytes).
pub fn srgb_decode(v: u8) -> f32 {
    let e = f32::from(v) / 255.0;
    if e <= 0.040_45 {
        e / 12.92
    } else {
        ((e + 0.055) / 1.055).powf(2.4)
    }
}

pub fn solid_rgba(c: LinearColour) -> [u8; 4] {
    [
        srgb_encode(c.0[0]),
        srgb_encode(c.0[1]),
        srgb_encode(c.0[2]),
        (c.0[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub fn px_tile(px: &[u8; 4], w: u32, h: u32) -> Vec<u8> {
    std::iter::repeat_n(*px, (w * h) as usize)
        .flatten()
        .collect()
}

/// Contain-fit a `src_w × src_h` image inside `dst_w × dst_h`, keeping aspect
/// ratio: returns `(w, h, off_x, off_y)` — the scaled size and the top-left
/// offset that centres it (the black bars of a letterbox fill the rest).
pub fn fit_contain(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, 0, 0);
    }
    let scale = (f64::from(dst_w) / f64::from(src_w)).min(f64::from(dst_h) / f64::from(src_h));
    let w = ((f64::from(src_w) * scale).round() as u32).clamp(1, dst_w);
    let h = ((f64::from(src_h) * scale).round() as u32).clamp(1, dst_h);
    ((w), (h), (dst_w - w) / 2, (dst_h - h) / 2)
}

/// One colour channel as the processor carries it on the way to a file: eight
/// bits or sixteen.
///
/// # In plain terms
///
/// A frame is a pile of numbers, and the only difference between an eight-bit
/// frame and a sixteen-bit one is how big each number is allowed to be. Rather
/// than write every pixel routine twice — once counting to 255 and once to
/// 65535, with two chances to round differently — the routines are written
/// once and this says what "full" means and how a channel is written into a
/// file. Nothing here decides colour; it is arithmetic about width.
pub trait Channel: Copy {
    /// Full scale: white, opaque, all the way up.
    const FULL: Self;
    /// Full scale as a float, for the arithmetic that has to normalise.
    const SCALE: f64;
    /// How many bytes one channel occupies in a file.
    const BYTES: usize;
    fn to_f64(self) -> f64;
    /// Rounded to the nearest code and clamped into range — one rule, so no
    /// two callers can round a channel differently.
    fn from_f64(v: f64) -> Self;
    /// Append this channel to a file buffer, little-endian (the byte order the
    /// encoder seam expects; each format's own order is its business).
    fn write_le(self, out: &mut Vec<u8>);
}

impl Channel for u8 {
    const FULL: Self = u8::MAX;
    const SCALE: f64 = 255.0;
    const BYTES: usize = 1;
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
    fn from_f64(v: f64) -> Self {
        v.round().clamp(0.0, Self::SCALE) as Self
    }
    fn write_le(self, out: &mut Vec<u8>) {
        out.push(self);
    }
}

impl Channel for u16 {
    const FULL: Self = u16::MAX;
    const SCALE: f64 = 65_535.0;
    const BYTES: usize = 2;
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
    fn from_f64(v: f64) -> Self {
        v.round().clamp(0.0, Self::SCALE) as Self
    }
    fn write_le(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

/// Bilinearly sample RGBA `src` (`w × h`) at continuous `(x, y)`, clamping to
/// the edges. Returns the four channels.
fn sample_bilinear<C: Channel>(src: &[C], w: u32, h: u32, x: f64, y: f64) -> [C; 4] {
    let x = x.clamp(0.0, f64::from(w - 1));
    let y = y.clamp(0.0, f64::from(h - 1));
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - f64::from(x0);
    let fy = y - f64::from(y0);
    let at = |px: u32, py: u32, c: usize| src[((py * w + px) * 4) as usize + c].to_f64();
    let mut out = [C::FULL; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let bot = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *o = C::from_f64(top * (1.0 - fy) + bot * fy);
    }
    out
}

/// Which filter the export's resize samples with.
///
/// In plain terms: shrinking a picture has to decide what happens to the
/// detail that no longer fits. *Fast* reads the four nearest pixels and mixes
/// them, which is quick and is what Lumit has always done. *High* uses a
/// Lanczos-3 window whose reach grows with the shrink, so every source pixel
/// that lands inside an output pixel actually contributes to it — the
/// difference between a shrunken fine check pattern turning into an even grey
/// (correct) and into a moiré (what point sampling gives you).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Resample {
    /// Bilinear: the four nearest source pixels, weighted by distance. The
    /// behaviour every Lumit export has had, kept as the default so a file
    /// exported today is byte-identical to the same export yesterday.
    #[default]
    Fast,
    /// Lanczos-3 (`sinc(x)·sinc(x/3)`, |x| < 3), separable, with the window
    /// widened by the shrink factor on a downscale so the filter covers the
    /// whole source footprint of each output pixel.
    High,
}

/// One output sample's source taps: the first source index (which may be
/// negative or run past the edge — reads clamp) and the normalised weights.
struct Taps {
    first: i64,
    weights: Vec<f64>,
}

/// The Lanczos-3 kernel. `sinc(x)·sinc(x/3)` for |x| < 3, zero outside.
fn lanczos3(x: f64) -> f64 {
    const A: f64 = 3.0;
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() >= A {
        return 0.0;
    }
    let px = std::f64::consts::PI * x;
    (px.sin() / px) * ((px / A).sin() / (px / A))
}

/// Tap tables for one axis: `src` source samples resampled to `dst` outputs.
///
/// The window is widened by the shrink factor (`src/dst` when that exceeds 1),
/// which is what makes a downscale average rather than point-sample. Weights
/// are normalised, so a flat input stays exactly flat and total energy is
/// preserved. Taps that fall outside the source clamp to the edge sample —
/// the same edge rule [`sample_bilinear`] uses — so the border does not darken.
fn lanczos_taps(src: u32, dst: u32) -> Vec<Taps> {
    let ratio = f64::from(src) / f64::from(dst);
    let filter_scale = ratio.max(1.0);
    let support = 3.0 * filter_scale;
    (0..dst)
        .map(|i| {
            let centre = (f64::from(i) + 0.5) * ratio - 0.5;
            let first = (centre - support).ceil() as i64;
            let last = (centre + support).floor() as i64;
            let mut weights: Vec<f64> = (first..=last)
                .map(|s| lanczos3((s as f64 - centre) / filter_scale))
                .collect();
            let sum: f64 = weights.iter().sum();
            if sum.abs() > f64::EPSILON {
                for w in &mut weights {
                    *w /= sum;
                }
            } else {
                // Degenerate (cannot happen for a > 0 support, but a zero sum
                // would divide by nothing): fall back to the nearest sample.
                weights.clear();
                weights.push(1.0);
                return Taps {
                    first: centre.round() as i64,
                    weights,
                };
            }
            Taps { first, weights }
        })
        .collect()
}

/// Separable Lanczos-3 resample of RGBA `src` into a fresh `dst_w × dst_h`
/// buffer of raw f64 channel values (not yet quantised).
fn lanczos_resample<C: Channel>(
    src: &[C],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<f64> {
    let xt = lanczos_taps(src_w, dst_w);
    let yt = lanczos_taps(src_h, dst_h);
    // Horizontal pass: dst_w x src_h, still in source rows.
    let mut mid = vec![0.0f64; (dst_w as usize) * (src_h as usize) * 4];
    for y in 0..src_h as usize {
        let row = y * (src_w as usize) * 4;
        for (x, t) in xt.iter().enumerate() {
            let out = (y * (dst_w as usize) + x) * 4;
            for (k, w) in t.weights.iter().enumerate() {
                let sx = (t.first + k as i64).clamp(0, i64::from(src_w) - 1) as usize;
                let si = row + sx * 4;
                for c in 0..4 {
                    mid[out + c] += src[si + c].to_f64() * w;
                }
            }
        }
    }
    // Vertical pass.
    let mut out = vec![0.0f64; (dst_w as usize) * (dst_h as usize) * 4];
    for (y, t) in yt.iter().enumerate() {
        for x in 0..dst_w as usize {
            let oi = (y * (dst_w as usize) + x) * 4;
            for (k, w) in t.weights.iter().enumerate() {
                let sy = (t.first + k as i64).clamp(0, i64::from(src_h) - 1) as usize;
                let si = (sy * (dst_w as usize) + x) * 4;
                for c in 0..4 {
                    out[oi + c] += mid[si + c] * w;
                }
            }
        }
    }
    out
}

/// Resize RGBA `src` (`src_w × src_h`) into a fresh `dst_w × dst_h` frame,
/// contain-fitted and centred on opaque black (letterbox). Used by the export
/// resolution presets; `how` picks the filter, and both up- and downscale.
/// Returns opaque black if `src` is too short for its stated size.
///
/// Generic over the channel width, so an eight-bit and a sixteen-bit export
/// letterbox through the identical arithmetic (docs/06 §7.4). Both filters run
/// in `f64` in a fixed order, so the result is deterministic (docs/06 §7.3).
pub fn letterbox_resize<C: Channel>(
    src: &[C],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    how: Resample,
) -> Vec<C> {
    let mut out = vec![C::from_f64(0.0); (dst_w as usize) * (dst_h as usize) * 4];
    for px in out.chunks_exact_mut(4) {
        px[3] = C::FULL; // opaque black background
    }
    let (w, h, ox, oy) = fit_contain(src_w, src_h, dst_w, dst_h);
    if w == 0 || h == 0 || src.len() < (src_w as usize) * (src_h as usize) * 4 {
        return out;
    }
    match how {
        Resample::Fast => {
            for y in 0..h {
                let sy = (f64::from(y) + 0.5) * f64::from(src_h) / f64::from(h) - 0.5;
                for x in 0..w {
                    let sx = (f64::from(x) + 0.5) * f64::from(src_w) / f64::from(w) - 0.5;
                    let px = sample_bilinear(src, src_w, src_h, sx, sy);
                    let di = (((oy + y) * dst_w + (ox + x)) * 4) as usize;
                    out[di..di + 4].copy_from_slice(&px);
                }
            }
        }
        Resample::High => {
            let fit = lanczos_resample(src, src_w, src_h, w, h);
            for y in 0..h {
                for x in 0..w {
                    let si = (((y * w) + x) * 4) as usize;
                    let di = (((oy + y) * dst_w + (ox + x)) * 4) as usize;
                    for c in 0..4 {
                        // Lanczos rings past the ends of the range; `from_f64`
                        // is the one place a channel is clamped and rounded.
                        out[di + c] = C::from_f64(fit[si + c]);
                    }
                }
            }
        }
    }
    out
}

/// Source-over one colour onto a pixel, in the straight-alpha bytes the CPU
/// path carries everywhere else. Shared by the shape and paint rasterisers so
/// the two composite identically.
pub(crate) fn over(px: &mut [u8], rgb: [u8; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let dst_a = f32::from(px[3]) / 255.0;
    let out_a = a + dst_a * (1.0 - a);
    if out_a <= 0.0 {
        px.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for c in 0..3 {
        let src = f32::from(rgb[c]) / 255.0;
        let dst = f32::from(px[c]) / 255.0;
        let out = (src * a + dst * dst_a * (1.0 - a)) / out_a;
        px[c] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    px[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// [`over`] for a scene-linear float pixel.
///
/// The same source-over, minus the two conversions: the colour is already
/// linear and the destination already float, so nothing is decoded, encoded or
/// rounded on the way through. Only alpha is clamped — a colour here may sit
/// above white and must stay there.
pub(crate) fn over_f32(px: &mut [f32; 4], rgb: [f32; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let dst_a = px[3];
    let out_a = a + dst_a * (1.0 - a);
    if out_a <= 0.0 {
        *px = [0.0; 4];
        return;
    }
    for c in 0..3 {
        px[c] = (rgb[c] * a + px[c] * dst_a * (1.0 - a)) / out_a;
    }
    px[3] = out_a;
}

/// Per-channel linear crossfade of two equal-length RGBA8 buffers:
/// `a·(1−t) + b·t`. `t` is clamped to 0..1 (0 = all `a`). The shared frame-blend
/// used by both preview and export so a blended slow-mo frame is identical in
/// each. Blends in sRGB bytes — standard NLE frame blending.
pub fn blend_rgba(a: &[u8], b: &[u8], t: f32) -> Vec<u8> {
    let t = t.clamp(0.0, 1.0);
    let n = a.len().min(b.len());
    (0..n)
        .map(|i| {
            (f32::from(a[i]) * (1.0 - t) + f32::from(b[i]) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// One float sample out of a byte buffer, or nought where the buffer stops
/// short. Every float helper reads through this, so a mismatched length is a
/// calm black pixel rather than a panic (docs/14 §4).
#[must_use]
pub fn f32_at(buf: &[u8], i: usize) -> f32 {
    let at = i.saturating_mul(4);
    buf.get(at..at + 4)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map_or(0.0, f32::from_le_bytes)
}

/// Pixel `n`'s four channels out of a `LinearF32` buffer.
#[must_use]
pub fn f32_px(buf: &[u8], n: usize) -> [f32; 4] {
    let base = n.saturating_mul(4);
    [
        f32_at(buf, base),
        f32_at(buf, base + 1),
        f32_at(buf, base + 2),
        f32_at(buf, base + 3),
    ]
}

/// Write pixel `n`'s four channels back into a `LinearF32` buffer. A write past
/// the end does nothing, for [`f32_at`]'s reason.
pub fn set_f32_px(buf: &mut [u8], n: usize, v: [f32; 4]) {
    let at = n.saturating_mul(16);
    let Some(px) = buf.get_mut(at..at + 16) else {
        return;
    };
    for (slot, value) in px.chunks_exact_mut(4).zip(v) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

/// [`blend_rgba`] for the float frames a float source decodes to
/// (`lumit_media::PixelFormat::LinearF32`).
///
/// The same crossfade, done on the numbers rather than on bytes. Blending
/// floats *as* bytes would not dim a highlight, it would shred it: the four
/// bytes of a float are a sign, an exponent and a mantissa, and averaging those
/// separately is not averaging anything.
///
/// No clamp at the top. Both inputs are scene-linear and may sit well above
/// white, and a crossfade between two bright frames has no business darkening
/// either of them to 1.0.
#[must_use]
pub fn blend_f32(a: &[u8], b: &[u8], t: f32) -> Vec<u8> {
    let t = t.clamp(0.0, 1.0);
    let n = a.len().min(b.len()) / 4;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let v = f32_at(a, i) * (1.0 - t) + f32_at(b, i) * t;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Which source frame(s) show `source_time` seconds of footage at `fps` over
/// `frames` frames. Nearest → `(frame, None)`. Blend → `(floor, Some((ceil,
/// weight)))` where `weight` is how far past `floor` the moment sits (0 at the
/// floor). Exact frames and the last frame collapse to a single frame (no
/// blend). Everything is clamped into `0..frames`. Shared by preview + export.
pub fn frame_pick(
    source_time: f64,
    fps: f64,
    frames: usize,
    blend: bool,
    sample_fps: Option<f64>,
) -> (usize, Option<(usize, f32)>) {
    if frames == 0 {
        return (0, None);
    }
    let last = frames - 1;
    if !blend {
        // Nearest shows the native frame at the source time — conform is a
        // blend/flow concept and never applies here.
        let pos = (source_time * fps).max(0.0);
        return ((pos.round() as usize).min(last), None);
    }
    // The sampling rate: a conform rate below the native one makes
    // flow bracket source frames spaced further apart — real motion for
    // high-fps footage. None, or a rate at/above native, samples adjacent
    // native frames exactly as before.
    let r = match sample_fps {
        Some(r) if r > 0.0 && r < fps => r,
        _ => fps,
    };
    let v = (source_time * r).max(0.0);
    let floor_v = v.floor();
    let w = (v - floor_v) as f32;
    // Map a virtual (conform-rate) frame index back to the nearest native
    // frame to decode.
    let to_native = |vi: f64| (((vi / r) * fps).round().max(0.0) as usize).min(last);
    let a = to_native(floor_v);
    let b = to_native(floor_v + 1.0);
    if a == b || w <= 0.0 {
        (a, None)
    } else {
        (a, Some((b, w)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The conform rate, from the other end: animation drawn on 2s.
    ///
    /// A 24 fps anime cut animated on 2s holds each drawing for two frames —
    /// A A B B C C. Interpolating natively, half the pairs bracket a frame and
    /// its own duplicate (no motion at all) and the rest carry the whole step,
    /// which reads as judder rather than slow motion. Conforming to 12 — the
    /// rate it was *drawn* at — makes every bracket span two different
    /// drawings.
    #[test]
    fn a_conform_rate_skips_the_duplicates_of_animation_on_twos() {
        let fps = 24.0;
        let frames = 48;
        // Native: at an eighth of a second in, the bracket is frames 3 and 4 —
        // and on 2s, frames 2 and 3 are the same drawing, so the pair 3-4
        // straddles a change while 2-3 would not move at all.
        let (a, b) = frame_pick(0.14, fps, frames, true, None);
        assert_eq!(a, 3);
        assert_eq!(b.map(|(f, _)| f), Some(4));

        // Conformed to 12: brackets are always an even frame and the next even
        // frame — one drawing to the next, never a drawing to itself.
        for step in 0..8 {
            let t = f64::from(step) * 0.09;
            let (a, b) = frame_pick(t, fps, frames, true, Some(12.0));
            assert!(
                a.is_multiple_of(2),
                "conformed bracket starts on a drawing: {a}"
            );
            if let Some((f, _)) = b {
                assert_eq!(f, a + 2, "and ends on the next one, never a duplicate");
            }
        }
    }

    #[test]
    fn blend_and_frame_pick() {
        // Half-blend of black and mid-grey is mid-value.
        assert_eq!(
            blend_rgba(&[0, 0, 0, 0], &[100, 100, 100, 100], 0.5),
            vec![50, 50, 50, 50]
        );
        assert_eq!(blend_rgba(&[0; 4], &[80; 4], 0.0), vec![0; 4]); // t=0 → a
        assert_eq!(blend_rgba(&[0; 4], &[80; 4], 1.0), vec![80; 4]); // t=1 → b
                                                                     // Nearest rounds to a single frame.
        assert_eq!(frame_pick(1.017, 30.0, 100, false, None), (31, None));
        // Blend straddles two frames with the fractional weight.
        let (f, b) = frame_pick(1.017, 30.0, 100, true, None);
        assert_eq!(f, 30);
        let (c, w) = b.unwrap();
        assert_eq!(c, 31);
        assert!((w - 0.51).abs() < 0.01);
        // An exact frame doesn't blend; past the end clamps to the last frame.
        assert_eq!(frame_pick(1.0, 30.0, 100, true, None), (30, None));
        assert_eq!(frame_pick(100.0, 30.0, 100, true, None), (99, None));
        // Conform: a 60fps clip conformed to 15fps brackets frames
        // spaced 4 native frames apart. At source_time 0.05s the 15fps
        // virtual index is 0.75, so it blends native frames 0 and 4 at 0.75.
        let (f, b) = frame_pick(0.05, 60.0, 100, true, Some(15.0));
        assert_eq!(f, 0);
        let (c, w) = b.unwrap();
        assert_eq!(c, 4);
        assert!((w - 0.75).abs() < 0.01);
        // A conform rate at or above native is a no-op (adjacent frames).
        let (f, b) = frame_pick(1.017, 30.0, 100, true, Some(60.0));
        assert_eq!(f, 30);
        let (c, w) = b.unwrap();
        assert_eq!(c, 31);
        assert!((w - 0.51).abs() < 0.01);
    }

    #[test]
    fn fit_contain_letterboxes_and_pillarboxes() {
        // 16:9 into a tall 1080×1920 frame: full width, bars top and bottom.
        let (w, h, ox, oy) = fit_contain(1920, 1080, 1080, 1920);
        assert_eq!((w, ox), (1080, 0));
        assert_eq!(h, 608); // 1080 * 9/16 rounded
        assert_eq!(oy, (1920 - 608) / 2);
        // Exact multiple upscales cleanly, centred.
        assert_eq!(fit_contain(2, 2, 4, 4), (4, 4, 0, 0));
        // Degenerate inputs don't panic.
        assert_eq!(fit_contain(0, 0, 4, 4), (0, 0, 0, 0));
    }

    #[test]
    fn letterbox_puts_the_image_in_a_black_frame() {
        // A solid red 4×2 into a 2×2 target: contain scale 0.5 ⇒ 2×1, so the
        // top row is red and the bottom row is the black bar.
        let red = [255u8, 0, 0, 255];
        let src: Vec<u8> = red.iter().copied().cycle().take(4 * 2 * 4).collect();
        let out = letterbox_resize(&src, 4, 2, 2, 2, Resample::Fast);
        assert_eq!(&out[0..4], &red); // (0,0) red
        assert_eq!(&out[4..8], &red); // (1,0) red
        assert_eq!(&out[8..12], &[0, 0, 0, 255]); // (0,1) black bar
        assert_eq!(&out[12..16], &[0, 0, 0, 255]); // (1,1) black bar
    }

    #[test]
    fn letterbox_preserves_a_solid_colour() {
        let blue = [0u8, 0, 255, 255];
        let src: Vec<u8> = blue.iter().copied().cycle().take(2 * 2 * 4).collect();
        // Same aspect (square → square) fills the whole target with blue.
        let out = letterbox_resize(&src, 2, 2, 8, 8, Resample::Fast);
        for px in out.chunks_exact(4) {
            assert_eq!(px, &blue);
        }
    }

    /// An 8×8 black/white checker box-downscaled to 2×2 has one analytic
    /// answer: every output pixel covers exactly sixteen source pixels, eight
    /// of each, so the mean is half scale. Lanczos-3 with the window widened
    /// by the shrink factor lands on it, because every source pixel inside an
    /// output pixel's footprint actually gets a weight.
    #[test]
    fn high_downscales_a_checker_to_its_mean() {
        let mut src = Vec::with_capacity(8 * 8 * 4);
        for y in 0..8u32 {
            for x in 0..8u32 {
                let v = if (x + y) % 2 == 0 { 255u8 } else { 0 };
                src.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let hi = letterbox_resize(&src, 8, 8, 2, 2, Resample::High);
        for px in hi.chunks_exact(4) {
            for c in &px[..3] {
                assert!(
                    (i32::from(*c) - 128).abs() <= 12,
                    "Lanczos downscale of a checker should be near mid grey, got {c}"
                );
            }
            assert_eq!(px[3], 255);
        }
    }

    /// Energy in, energy out: a ramp downscaled 4:1 keeps its mean, because
    /// every tap row is normalised to sum to one.
    #[test]
    fn high_preserves_total_energy() {
        let (w, h) = (32u32, 32u32);
        let mut src = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 3) % 256) as u8;
                src.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let mean = |px: &[u8]| -> f64 {
            let n = px.len() / 4;
            px.chunks_exact(4).map(|p| f64::from(p[0])).sum::<f64>() / n as f64
        };
        let out = letterbox_resize(&src, w, h, 8, 8, Resample::High);
        assert!(
            (mean(&src) - mean(&out)).abs() < 2.0,
            "mean drifted: {} -> {}",
            mean(&src),
            mean(&out)
        );
    }

    /// A flat field stays exactly flat through the high filter too — the
    /// normalisation makes ringing impossible where there is nothing to ring.
    #[test]
    fn high_preserves_a_solid_colour_and_repeats() {
        let blue = [0u8, 0, 255, 255];
        let src: Vec<u8> = blue.iter().copied().cycle().take(2 * 2 * 4).collect();
        let a = letterbox_resize(&src, 2, 2, 8, 8, Resample::High);
        for px in a.chunks_exact(4) {
            assert_eq!(px, &blue);
        }
        // Deterministic: the same input gives the same bytes, every time.
        let b = letterbox_resize(&src, 2, 2, 8, 8, Resample::High);
        assert_eq!(a, b);
    }
}
