//! Gradient's kernel (docs/08 §3.35), shared by the Gradient effect and the
//! Gradient overlay style.

use crate::fx::noise::hash01;

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
    /// **Paint the ramp only where the layer already is**: the Gradient
    /// *effect* is a generator and floods the frame opaque, while the Gradient
    /// overlay *style* dresses the layer's own coverage. One multiply by the
    /// alpha, and the alpha itself left alone — which is the whole difference
    /// between the two, and why there is one kernel rather than two.
    pub clip_to_alpha: bool,
    /// How many of `mid` are in use, 0..=3: colours between `c0` and `c1`,
    /// scene-linear rgb with the place along the ramp (0..1, ascending) in
    /// the last lane.
    pub mids: u32,
    pub mid: [[f32; 4]; 3],
}

/// The ramp's colour at `t` (0..1): `c0`, the colours in use from `mid`,
/// then `c1`, each mixed with the next by how far between them `t` is.
#[must_use]
pub fn gradient_ramp(p: &GradientParams, t: f32) -> [f32; 3] {
    let (mut at, mut from) = (0.0f32, p.c0);
    for m in p.mid.iter().take(p.mids.min(3) as usize) {
        let to = [m[0], m[1], m[2]];
        if t <= m[3] {
            let k = if m[3] > at {
                (t - at) / (m[3] - at)
            } else {
                1.0
            };
            return std::array::from_fn(|c| from[c] + (to[c] - from[c]) * k);
        }
        (at, from) = (m[3], to);
    }
    let k = if at < 1.0 { (t - at) / (1.0 - at) } else { 1.0 };
    std::array::from_fn(|c| from[c] + (p.c1[c] - from[c]) * k)
}

/// Gradient (docs/08 §3.35): a linear or radial ramp with optional
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
            // Clipped to alpha: `ramp · a` is the premultiplied form of
            // "this ramp at this coverage", so the overlay works on
            // premultiplied values with no round trip, exactly as `fill` does.
            let cover = if p.clip_to_alpha { rgba[i + 3] } else { 1.0 };
            let ramp = gradient_ramp(p, t);
            for c in 0..3 {
                let g = ramp[c] * cover;
                rgba[i + c] = rgba[i + c] * (1.0 - p.mix) + g * p.mix;
            }
            if !p.clip_to_alpha {
                // The generator writes opaque; the overlay leaves the layer's
                // own coverage exactly as it found it.
                rgba[i + 3] = rgba[i + 3] * (1.0 - p.mix) + p.mix;
            }
        }
    }
}
