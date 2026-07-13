//! Text rasterisation (docs/03-DATA-MODEL.md §9.1, Phase 1 v1 scope).
//!
//! In plain terms: turning a line of text into pixels. v1 is one embedded
//! font (Inter, OFL-licensed, vendored in assets/fonts per the household's
//! self-hosted rule), single-style runs, simple advance-based layout. Styled
//! runs, font selection, shaping/kerning via a full text stack (cosmic-text)
//! and per-character animators follow the data model doc.

use std::sync::OnceLock;

/// Inter Regular, embedded at compile time — deterministic across machines.
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.otf");

fn font() -> &'static fontdue::Font {
    static FONT: OnceLock<fontdue::Font> = OnceLock::new();
    FONT.get_or_init(|| {
        #[allow(clippy::expect_used)] // compile-time asset; failure = broken build
        fontdue::Font::from_bytes(INTER, fontdue::FontSettings::default())
            .expect("embedded Inter font parses")
    })
}

/// A rasterised line: straight-alpha RGBA8, tightly cropped.
pub struct RasterText {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Measure and rasterise a single line at `size` px with the given linear
/// colour (encoded to sRGB bytes here; alpha carries the glyph coverage).
pub fn rasterise_line(text: &str, size: f32, rgb8: [u8; 3]) -> RasterText {
    let font = font();
    let size = size.clamp(4.0, 512.0);

    // First pass: measure.
    let mut pen_x = 0.0f32;
    let mut glyphs = Vec::new();
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, size);
        let top = -(metrics.ymin as f32) - metrics.height as f32;
        min_y = min_y.min(top);
        max_y = max_y.max(top + metrics.height as f32);
        glyphs.push((ch, pen_x, metrics));
        pen_x += metrics.advance_width;
    }
    if glyphs.is_empty() || min_y > max_y {
        return RasterText {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
    }
    let width = pen_x.ceil().max(1.0) as u32;
    let height = (max_y - min_y).ceil().max(1.0) as u32;

    // Second pass: blit coverage into the buffer.
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (ch, x0, metrics) in glyphs {
        let (_, bitmap) = font.rasterize(ch, size);
        let gx = x0.round() as i64 + i64::from(metrics.xmin);
        let gy = (-(metrics.ymin as f32) - metrics.height as f32 - min_y).round() as i64;
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let px = gx + col as i64;
                let py = gy + row as i64;
                if px < 0 || py < 0 || px >= i64::from(width) || py >= i64::from(height) {
                    continue;
                }
                let cov = bitmap[row * metrics.width + col];
                let idx = ((py as u32 * width + px as u32) * 4) as usize;
                rgba[idx] = rgb8[0];
                rgba[idx + 1] = rgb8[1];
                rgba[idx + 2] = rgb8[2];
                rgba[idx + 3] = rgba[idx + 3].max(cov);
            }
        }
    }
    RasterText {
        width,
        height,
        rgba,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn renders_visible_deterministic_text() {
        let a = rasterise_line("Kiriko", 48.0, [255, 255, 255]);
        assert!(a.width > 60 && a.height > 20, "{}x{}", a.width, a.height);
        let ink: u64 = a.rgba.chunks_exact(4).map(|p| u64::from(p[3])).sum();
        assert!(ink > 10_000, "ink {ink}");
        // Deterministic: identical run, identical bytes.
        let b = rasterise_line("Kiriko", 48.0, [255, 255, 255]);
        assert_eq!(a.rgba, b.rgba);
    }

    #[test]
    fn empty_text_yields_a_transparent_pixel() {
        let r = rasterise_line("", 48.0, [255, 0, 0]);
        assert_eq!((r.width, r.height), (1, 1));
        assert_eq!(r.rgba[3], 0);
    }

    #[test]
    fn size_scales_the_raster() {
        let small = rasterise_line("Aa", 16.0, [255, 255, 255]);
        let large = rasterise_line("Aa", 64.0, [255, 255, 255]);
        assert!(large.width > small.width * 3);
        assert!(large.height > small.height * 3);
    }
}
