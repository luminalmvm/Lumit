//! The CPU reference: the core, the halo and the composite, in the order and
//! arithmetic `fx_energy_line.wgsl` runs them.

use super::geometry::{Bins, Pieces};
use crate::fx::cpu;
use crate::fx::noise::{fractal, FractalField};

/// One frame's light, every length in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Half the core's width at 100 % size.
    pub core_half: f32,
    /// The width at the stretch's start and end, as multiples of `core_half`.
    pub taper: [f32; 2],
    /// The share of the half-width the core's edge fades over.
    pub softness: f32,
    pub core_colour: [f32; 3],
    /// How bright the tight glow round the core is, 0 for none.
    pub halo: f32,
    pub glow_colour: [f32; 3],
    /// What the halo is multiplied by. 0 is no glow at all.
    pub glow_gain: f32,
    pub glow_radius: f32,
    /// Glow's own Falloff number.
    pub glow_falloff: f32,
    /// How far the noise pushes the core and the glow.
    pub core_warp: f32,
    pub glow_warp: f32,
    pub field: FractalField,
    /// Noise cells a raster pixel.
    pub inv_size: f32,
    pub evolution: f32,
    /// How far the wind has moved the field, in cells.
    pub drift: [f32; 2],
    /// The flicker's gain, 0..1.
    pub flicker: f32,
    pub composite: bool,
    pub mix: f32,
    /// How many shutter moments the pieces are spread over, at least 1.
    pub samples: u32,
}

/// How far the halo reaches past the core, in core half-widths.
pub const HALO_REACH: f32 = 8.0;

/// The halo's fall, in core half-widths: down to a thousandth at its reach.
const HALO_FALL: f32 = 1.2;

/// The halo `out` raster pixels past a core `r` wide, 1 at its edge and
/// exactly 0 at [`HALO_REACH`], so no tile edge ever shows where it stops.
#[must_use]
pub fn halo_at(out: f32, r: f32) -> f32 {
    let floor = (-HALO_REACH / HALO_FALL).exp();
    (((-out / (r * HALO_FALL)).exp() - floor) / (1.0 - floor)).clamp(0.0, 1.0)
}

/// Where each push reads the field, so the two axes and the two pushes don't
/// move together.
pub const WARP_TAPS: [[f32; 2]; 4] = [[0.0, 0.0], [17.31, -9.73], [41.2, 23.9], [-28.6, 57.1]];

/// The noise's push at `(x, y)`, `amount` raster pixels at most, read from
/// taps `pair` and `pair + 1`.
#[must_use]
pub fn warp(look: &Look, x: f32, y: f32, amount: f32, pair: usize) -> [f32; 2] {
    if amount <= 0.0 {
        return [0.0, 0.0];
    }
    let fx = x * look.inv_size + look.drift[0];
    let fy = y * look.inv_size + look.drift[1];
    let n = |tap: [f32; 2]| {
        fractal(&look.field, fx + tap[0], fy + tap[1], look.evolution).clamp(-1.0, 1.0) * amount
    };
    [n(WARP_TAPS[pair]), n(WARP_TAPS[pair + 1])]
}

/// The core's coverage, heat and halo at the pixel centred on
/// `(x + 0.5, y + 0.5)`. Heat is 1 on the line and 0 at the core's edge, which
/// is where the core colour gives way to the glow colour. Each is the most any
/// piece gives within a shutter moment, then the mean over the moments.
#[must_use]
pub fn core_at(x: u32, y: u32, look: &Look, p: &Pieces, b: &Bins) -> [f32; 3] {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let push = warp(look, px, py, look.core_warp, 0);
    let (qx, qy) = (px + push[0], py + push[1]);
    let mut moments = [[0.0f32; 3]; super::MAX_SAMPLES];
    for &k in b.at(x, y) {
        let (s, u) = (p.segs[k as usize], p.along[k as usize]);
        let m = &mut moments[(u[2] as usize).min(super::MAX_SAMPLES - 1)];
        let (dx, dy) = (s[2] - s[0], s[3] - s[1]);
        let (rx, ry) = (qx - s[0], qy - s[1]);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let t = ((rx * dx + ry * dy) / len2).clamp(0.0, 1.0);
        let (ox, oy) = (rx - t * dx, ry - t * dy);
        let dist = (ox * ox + oy * oy).sqrt();
        let along = u[0] + (u[1] - u[0]) * t;
        let r = look.core_half * (look.taper[0] + (look.taper[1] - look.taper[0]) * along);
        // A core of no width lights nothing.
        if r <= 0.0 {
            continue;
        }
        let band = (look.softness * r).max(0.5);
        let c = ((r - dist) / band + 0.5).clamp(0.0, 1.0) * u[3];
        m[0] = m[0].max(c);
        m[1] = m[1].max((1.0 - dist / r).clamp(0.0, 1.0) * c);
        m[2] = m[2].max(halo_at((dist - r).max(0.0), r) * u[3]);
    }
    let n = look.samples.clamp(1, super::MAX_SAMPLES as u32) as usize;
    let mut out = [0.0f32; 3];
    for m in &moments[..n] {
        for (o, v) in out.iter_mut().zip(m) {
            *o += v / n as f32;
        }
    }
    out
}

/// The halo at a fractional position, bilinear between pixel centres with the
/// edges held, from channel 0 of an RGBA picture.
#[must_use]
pub fn sample(halo: &[f32], w: u32, h: u32, x: f32, y: f32) -> f32 {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |xi: f32, yi: f32| {
        let xi = (xi as i32).clamp(0, w as i32 - 1) as usize;
        let yi = (yi as i32).clamp(0, h as i32 - 1) as usize;
        halo[(yi * w as usize + xi) * 4]
    };
    let top = at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * tx;
    top + (bottom - top) * ty
}

/// The light over the picture that arrived. Both are straight alpha, which is
/// how the compositor reads a layer's picture: the picture is weighted by its
/// alpha, the light added to it, and the sum handed back straight.
#[must_use]
pub fn combine(src: [f32; 4], core: [f32; 3], halo: f32, look: &Look) -> [f32; 4] {
    let [cov, heat, near] = core;
    // The middle runs white-hot quickly, so the colour lives in the rim.
    let t = (heat * 1.6).clamp(0.0, 1.0);
    let hot = t * t * (3.0 - 2.0 * t);
    let mut light = [0.0f32; 3];
    for (i, l) in light.iter_mut().enumerate() {
        let core = cov * (look.glow_colour[i] * (1.0 - hot) + look.core_colour[i] * hot);
        let glow = look.glow_colour[i] * (halo.max(0.0) * look.glow_gain + near * look.halo);
        *l = (core + glow) * look.flicker;
    }
    let alpha = light[0].max(light[1]).max(light[2]).clamp(0.0, 1.0);
    let before = [src[0] * src[3], src[1] * src[3], src[2] * src[3], src[3]];
    let under = if look.composite { before } else { [0.0; 4] };
    let lit = [
        under[0] + light[0],
        under[1] + light[1],
        under[2] + light[2],
        under[3] + alpha * (1.0 - under[3]),
    ];
    let m = look.mix;
    let mut out = [0.0f32; 4];
    for (i, o) in out.iter_mut().enumerate() {
        *o = before[i] * (1.0 - m) + lit[i] * m;
    }
    if out[3] <= 0.0 {
        return [0.0; 4];
    }
    [out[0] / out[3], out[1] / out[3], out[2] / out[3], out[3]]
}

/// The whole effect on a `w × h` premultiplied RGBA picture.
pub fn render(rgba: &mut [f32], w: u32, h: u32, look: &Look, p: &Pieces, b: &Bins) {
    let n = (w * h) as usize;
    if rgba.len() < n * 4 || (p.segs.is_empty() && look.composite) {
        return;
    }
    let mut core = vec![0.0f32; n * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize * 4;
            let [c, heat, near] = core_at(x, y, look, p, b);
            core[i] = c;
            core[i + 1] = heat;
            core[i + 2] = near;
        }
    }
    let halo = if look.glow_gain > 0.0 {
        cpu::glow_exponential(&core, w, h, look.glow_radius, look.glow_falloff)
    } else {
        vec![0.0; n * 4]
    };
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize * 4;
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let push = warp(look, px, py, look.glow_warp, 2);
            let glow = sample(&halo, w, h, px + push[0], py + push[1]);
            let src = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
            let out = combine(src, [core[i], core[i + 1], core[i + 2]], glow, look);
            rgba[i..i + 4].copy_from_slice(&out);
        }
    }
}
