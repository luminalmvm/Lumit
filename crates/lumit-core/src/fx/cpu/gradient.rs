//! Gradient's kernel (docs/08 §3.35), shared by the Gradient effect and the
//! Gradient overlay style.

use crate::fx::noise::hash01;

/// [`GradientParams::shape`]: the ramp projected onto the axis.
pub const LINEAR: u32 = 0;
/// [`GradientParams::shape`]: the ramp measured out from the start point.
pub const RADIAL: u32 = 1;
/// [`GradientParams::shape`]: the ramp swept once round the start point,
/// clockwise from the direction of the end point.
pub const ANGLE: u32 = 2;

/// One resolved Gradient (docs/08 §3.35), reduced to what both paths read.
/// Every reciprocal is computed host-side (`Gradient::packed`) and floored
/// there, so a zero-length axis collapses the ramp to one flat colour instead of
/// dividing by zero (docs/14 §4) and neither path divides per pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientParams {
    /// [`LINEAR`], [`RADIAL`] or [`ANGLE`].
    pub shape: u32,
    /// The start point in raster pixels (the §2.3 preview factor already applied).
    pub start: [f32; 2],
    /// `end − start`, raster pixels.
    pub axis: [f32; 2],
    /// `1 ÷ |axis|²` for the linear projection, floored.
    pub inv_len2: f32,
    /// `1 ÷ |axis|` for the radial distance, floored.
    pub inv_len: f32,
    /// The axis's direction, radians, where an angle ramp begins.
    pub turn: f32,
    /// A radial ramp's centre off the start point, raster pixels, and its
    /// squared distance from it less the squared radius: see [`focal`].
    /// Zero is centred.
    pub focal: [f32; 3],
    /// Scene-linear start colour, its alpha straight in the last lane.
    pub c0: [f32; 4],
    /// Scene-linear end colour.
    pub c1: [f32; 4],
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
    /// The alpha of each of `mid`.
    pub mid_alpha: [f32; 3],
    /// Mix the colours as display values, as a browser does, for a layer in a
    /// comp that blends in display.
    pub display: bool,
}

impl GradientParams {
    /// A black to white ramp from `start` along `axis`, in raster pixels, with
    /// every number the kernels read from the axis worked out here, once.
    /// Both reciprocals are floored against one epsilon on the squared
    /// length, so the linear and radial readings degenerate at exactly the
    /// same point, and Start and End at one place give one flat colour rather
    /// than a division by zero (docs/14 §4).
    #[must_use]
    pub fn aimed(start: [f32; 2], axis: [f32; 2]) -> Self {
        let len2 = (axis[0] * axis[0] + axis[1] * axis[1]).max(1e-6);
        Self {
            shape: LINEAR,
            start,
            axis,
            inv_len2: 1.0 / len2,
            inv_len: 1.0 / len2.sqrt(),
            turn: axis[1].atan2(axis[0]),
            focal: [0.0; 3],
            c0: [0.0, 0.0, 0.0, 1.0],
            c1: [1.0; 4],
            scatter: 0.0,
            seed: 0,
            mix: 1.0,
            clip_to_alpha: false,
            mids: 0,
            mid: [[0.0; 4]; 3],
            mid_alpha: [1.0; 3],
            display: false,
        }
    }

    /// The same ramp, mixing its colours as display values when `display` is
    /// set.
    #[must_use]
    pub fn in_display(self, display: bool) -> Self {
        Self { display, ..self }
    }
}

/// Where a radial ramp's bright centre sits for a highlight of `length` per
/// cent of the radius along `axis`, turned `angle` degrees clockwise from it,
/// as [`GradientParams::focal`] carries it. Never quite on the edge, where the
/// ramp would have no inside left.
#[must_use]
pub fn focal(axis: [f32; 2], length: f32, angle: f32) -> [f32; 3] {
    let off = (length / 100.0).clamp(-0.99, 0.99);
    if off == 0.0 {
        return [0.0; 3];
    }
    let len2 = (axis[0] * axis[0] + axis[1] * axis[1]).max(1e-6);
    let turn = axis[1].atan2(axis[0]) + angle.to_radians();
    let reach = len2.sqrt() * off;
    let (x, y) = (turn.cos() * reach, turn.sin() * reach);
    [x, y, x * x + y * y - len2]
}

/// The ramp's colour at `t` (0..1): `c0`, the colours in use from `mid`,
/// then `c1`, each mixed with the next by how far between them `t` is. The
/// colour and its alpha mix apart, straight, as a shape's gradient does. In
/// display the colours mix as display values and the alpha as it is.
#[must_use]
pub fn gradient_ramp(p: &GradientParams, t: f32) -> [f32; 4] {
    let each = |c: [f32; 4], f: fn(f32) -> f32| [f(c[0]), f(c[1]), f(c[2]), c[3]];
    let shown = |c: [f32; 4]| if p.display { each(c, display_value) } else { c };
    let back = |c: [f32; 4]| if p.display { each(c, linear_value) } else { c };
    let (mut at, mut from) = (0.0f32, shown(p.c0));
    for (m, a) in p.mid.iter().zip(p.mid_alpha).take(p.mids.min(3) as usize) {
        let to = shown([m[0], m[1], m[2], a]);
        if t <= m[3] {
            let k = if m[3] > at {
                (t - at) / (m[3] - at)
            } else {
                1.0
            };
            return back(std::array::from_fn(|c| from[c] + (to[c] - from[c]) * k));
        }
        (at, from) = (m[3], to);
    }
    let k = if at < 1.0 { (t - at) / (1.0 - at) } else { 1.0 };
    let to = shown(p.c1);
    back(std::array::from_fn(|c| from[c] + (to[c] - from[c]) * k))
}

/// A linear value as a display (sRGB) one, open above 1.
fn display_value(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// [`display_value`] undone.
fn linear_value(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Gradient (docs/08 §3.35): a linear, radial or angle ramp with optional
/// scatter — a **generator**, so it replaces the frame edge to edge and writes
/// the ramp's own alpha. Interpolation is in the working space (scene-linear, §2.1), so
/// the ramp is photometrically even and can drive another effect's matte. Mix 0
/// is the bit-exact identity (the WGSL twin matches).
pub fn gradient(rgba: &mut [f32], w: u32, h: u32, p: &GradientParams) {
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let dx = x as f32 + 0.5 - p.start[0];
            let dy = y as f32 + 0.5 - p.start[1];
            let mut t = match p.shape {
                RADIAL if p.focal[0] == 0.0 && p.focal[1] == 0.0 => {
                    (dx * dx + dy * dy).sqrt() * p.inv_len
                }
                // How far the pixel is from the centre, as a share of the way
                // from the centre through it to the outer edge.
                RADIAL => {
                    let (ex, ey) = (dx - p.focal[0], dy - p.focal[1]);
                    let a = ex * ex + ey * ey;
                    let b = p.focal[0] * ex + p.focal[1] * ey;
                    let reach = (b * b - a * p.focal[2]).sqrt() - b;
                    if reach > 0.0 {
                        a / reach
                    } else {
                        0.0
                    }
                }
                ANGLE => {
                    // y grows downwards, so a growing angle turns clockwise.
                    let turns = (dy.atan2(dx) - p.turn) / std::f32::consts::TAU;
                    turns - turns.floor()
                }
                _ => (dx * p.axis[0] + dy * p.axis[1]) * p.inv_len2,
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
            // A clear part of the ramp shows what it is laid over: the layer
            // inside it, nothing on the whole frame. Opaque, both terms are
            // the ones this kernel has always had, to the bit.
            let a = ramp[3];
            let under = if p.clip_to_alpha { 1.0 - a } else { 0.0 };
            for c in 0..3 {
                let mut g = ramp[c] * a * cover;
                if under > 0.0 {
                    g += rgba[i + c] * under;
                }
                rgba[i + c] = rgba[i + c] * (1.0 - p.mix) + g * p.mix;
            }
            if !p.clip_to_alpha {
                // The generator writes the ramp's alpha; the overlay leaves
                // the layer's own coverage exactly as it found it.
                rgba[i + 3] = rgba[i + 3] * (1.0 - p.mix) + a * p.mix;
            }
        }
    }
}
