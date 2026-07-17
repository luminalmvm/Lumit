//! Byte-level colour helpers shared by every path that hands sRGB pixels to
//! the GPU or a colour picker — deliberately ungated (the Project panel needs
//! a solid's swatch even in a media-free build).

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

pub fn solid_rgba(c: lumit_core::model::LinearColour) -> [u8; 4] {
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

/// Bilinearly sample RGBA8 `src` (`w × h`) at continuous `(x, y)`, clamping to
/// the edges. Returns the four channels.
fn sample_bilinear(src: &[u8], w: u32, h: u32, x: f64, y: f64) -> [u8; 4] {
    let x = x.clamp(0.0, f64::from(w - 1));
    let y = y.clamp(0.0, f64::from(h - 1));
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - f64::from(x0);
    let fy = y - f64::from(y0);
    let at = |px: u32, py: u32, c: usize| f64::from(src[((py * w + px) * 4) as usize + c]);
    let mut out = [0u8; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let bot = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *o = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Resize RGBA8 `src` (`src_w × src_h`) into a fresh `dst_w × dst_h` RGBA8
/// frame, contain-fitted and centred on opaque black (letterbox). Used by the
/// export resolution presets; bilinear sampling, so it up- and down-scales.
/// Returns opaque black if `src` is too short for its stated size.
pub fn letterbox_resize(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for px in out.chunks_exact_mut(4) {
        px[3] = 255; // opaque black background
    }
    let (w, h, ox, oy) = fit_contain(src_w, src_h, dst_w, dst_h);
    if w == 0 || h == 0 || src.len() < (src_w as usize) * (src_h as usize) * 4 {
        return out;
    }
    for y in 0..h {
        let sy = (f64::from(y) + 0.5) * f64::from(src_h) / f64::from(h) - 0.5;
        for x in 0..w {
            let sx = (f64::from(x) + 0.5) * f64::from(src_w) / f64::from(w) - 0.5;
            let px = sample_bilinear(src, src_w, src_h, sx, sy);
            let di = (((oy + y) * dst_w + (ox + x)) * 4) as usize;
            out[di..di + 4].copy_from_slice(&px);
        }
    }
    out
}

/// Per-channel linear crossfade of two equal-length RGBA8 buffers:
/// `a·(1−t) + b·t`. `t` is clamped to 0..1 (0 = all `a`). The shared frame-blend
/// used by both preview and export so a blended slow-mo frame is identical in
/// each (K-031). Blends in sRGB bytes — standard NLE frame blending.
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
    // The sampling rate: a conform rate below the native one (K-095) makes
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
        // Conform (K-095): a 60fps clip conformed to 15fps brackets frames
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
        let out = letterbox_resize(&src, 4, 2, 2, 2);
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
        let out = letterbox_resize(&src, 2, 2, 8, 8);
        for px in out.chunks_exact(4) {
            assert_eq!(px, &blue);
        }
    }
}
