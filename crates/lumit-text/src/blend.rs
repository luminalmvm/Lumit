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

#[cfg(test)]
mod tests {
    use super::*;

    fn word(width: u32, height: u32, alpha: u8) -> RasterText {
        RasterText {
            width,
            height,
            rgba: [200, 100, 50, alpha]
                .into_iter()
                .cycle()
                .take((width * height * 4) as usize)
                .collect(),
        }
    }

    #[test]
    fn a_fade_mixes_two_words_from_the_top_left_of_the_wider_one() {
        let (a, b) = (word(2, 1, 255), word(4, 2, 255));
        let mixed = crossfade(&a, &b, 0.5);
        assert_eq!((mixed.width, mixed.height), (4, 2));
        let at = |x: usize, y: usize| &mixed.rgba[(y * 4 + x) * 4..][..4];
        // Both words cover the first two pixels of the top row, so it is whole.
        assert_eq!(at(1, 0)[3], 255);
        // Only the wider word reaches the rest, so it is half as solid and
        // keeps its own colour.
        assert_eq!(at(3, 1), &[200, 100, 50, 128]);
        // A fade at nought is the first word alone.
        assert_eq!(crossfade(&a, &b, 0.0).rgba[..8], a.rgba[..8]);
    }
}
