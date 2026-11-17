//! The space Denoise works in: sRGB-encoded BT.709 Y'CbCr, straight alpha.
//!
//! Noise is measured and removed on encoded values because that is where a
//! camera's noise is close to even from shadow to highlight. The curve is
//! carried on past 0 and 1 (mirrored below, the power curve above), so a float
//! plate comes back exactly as it went in wherever nothing was removed.

/// Linear to sRGB-encoded, carried past 0 and 1.
#[must_use]
pub fn oetf(x: f32) -> f32 {
    if x < 0.0 {
        return -oetf(-x);
    }
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB-encoded back to linear, the inverse of [`oetf`].
#[must_use]
pub fn eotf(v: f32) -> f32 {
    if v < 0.0 {
        return -eotf(-v);
    }
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Encoded R'G'B' to Y'CbCr.
#[must_use]
pub fn to_ycc([r, g, b]: [f32; 3]) -> [f32; 3] {
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    [y, (b - y) / 1.8556, (r - y) / 1.5748]
}

/// Y'CbCr back to encoded R'G'B'.
#[must_use]
pub fn from_ycc([y, cb, cr]: [f32; 3]) -> [f32; 3] {
    let r = y + 1.5748 * cr;
    let b = y + 1.8556 * cb;
    let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
    [r, g, b]
}

/// Below this alpha a pixel is left as it is rather than divided.
pub const CLEAR: f32 = 1e-6;

/// One premultiplied linear pixel in Y'CbCr, its alpha beside it.
#[must_use]
pub fn pixel_to_ycc(px: [f32; 4]) -> [f32; 4] {
    let a = px[3];
    let k = if a > CLEAR { 1.0 / a } else { 1.0 };
    let [y, cb, cr] = to_ycc([oetf(px[0] * k), oetf(px[1] * k), oetf(px[2] * k)]);
    [y, cb, cr, a]
}

/// A Y'CbCr colour back to straight linear RGB.
#[must_use]
pub fn ycc_to_linear(ycc: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = from_ycc(ycc);
    [eotf(r), eotf(g), eotf(b)]
}

/// A whole premultiplied picture in Y'CbCr, alpha in the fourth lane.
#[must_use]
pub fn picture_to_ycc(rgba: &[f32]) -> Vec<f32> {
    rgba.chunks_exact(4)
        .flat_map(|p| pixel_to_ycc([p[0], p[1], p[2], p[3]]))
        .collect()
}

/// How the luma noise scales with brightness: Shadows at Y 0.1, Midtones at
/// 0.45, Highlights at 0.85, straight lines between and held flat outside.
#[must_use]
pub fn tone(y: f32, [shadows, midtones, highlights]: [f32; 3]) -> f32 {
    if y <= 0.1 {
        shadows
    } else if y <= 0.45 {
        shadows + (midtones - shadows) * (y - 0.1) / 0.35
    } else if y <= 0.85 {
        midtones + (highlights - midtones) * (y - 0.45) / 0.4
    } else {
        highlights
    }
}
