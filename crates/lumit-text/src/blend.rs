//! Two rasters of words faded into one, for a fade between keyed words.

use crate::RasterText;

/// `a` and `b` mixed as `a * (1 - w) + b * w`, premultiplied, then handed back
/// straight as every raster here is. Each is placed at the top left of the
/// union of the two sizes, which is where a plain raster of either word sits,
/// so a fade starts and ends exactly where the words are drawn before and
/// after it.
#[must_use]
pub fn crossfade(a: &RasterText, b: &RasterText, w: f32) -> RasterText {
    let width = a.width.max(b.width);
    let height = a.height.max(b.height);
    let (wide, high) = (width as usize, height as usize);
    // Premultiplied colour in byte units, and coverage 0 to 1.
    let mut sum = vec![0.0f32; wide * high * 4];
    for (r, k) in [(a, 1.0 - w), (b, w)] {
        let row = r.width as usize * 4;
        if row == 0 {
            continue;
        }
        for (y, line) in r.rgba.chunks_exact(row).enumerate() {
            for (x, px) in line.chunks_exact(4).enumerate() {
                let at = ((y * wide) + x) * 4;
                let Some(out) = sum.get_mut(at..at + 4) else {
                    continue;
                };
                let alpha = f32::from(px[3]) / 255.0 * k;
                for c in 0..3 {
                    out[c] += f32::from(px[c]) * alpha;
                }
                out[3] += alpha;
            }
        }
    }
    let rgba = sum
        .chunks_exact(4)
        .flat_map(|p| {
            if p[3] <= 0.0 {
                return [0; 4];
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
            let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            [
                byte(p[0] / p[3]),
                byte(p[1] / p[3]),
                byte(p[2] / p[3]),
                byte(p[3] * 255.0),
            ]
        })
        .collect();
    RasterText {
        width,
        height,
        rgba,
    }
}
