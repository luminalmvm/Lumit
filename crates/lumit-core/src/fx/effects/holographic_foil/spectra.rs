//! Holographic foil's colour, worked out without a lookup texture.
//!
//! The CIE 1931 observer is a sum of one-sided Gaussian lobes (Wyman, Sloan
//! and Shirley, JCGT 2013), so a band of light of any width blurs each lobe in
//! closed form. Each light spectrum becomes a short list of such terms, which
//! the kernel sums per pixel. The slowly varying parts (the aluminium, a
//! daylight or tungsten curve, the dye's edge) ride on each term as a factor.
//! Runs wherever the effect's parameters are packed.

use std::sync::OnceLock;

/// The observer's lobes: axis (x, y, z), height, centre in nm, and one over
/// the width below and above the centre.
const LOBES: [(usize, f64, f64, f64, f64); 7] = [
    (0, 0.362, 442.0, 0.0624, 0.0374),
    (0, 1.056, 599.8, 0.0264, 0.0323),
    (0, -0.065, 501.1, 0.049, 0.0382),
    (1, 0.821, 568.8, 0.0213, 0.0247),
    (1, 0.286, 530.9, 0.0613, 0.0322),
    (2, 1.217, 437.0, 0.0845, 0.0278),
    (2, 0.681, 459.0, 0.0385, 0.0725),
];

/// XYZ under an equal-energy white to linear sRGB, adapted to D65, by rows.
const E_TO_SRGB: [[f64; 3]; 3] = [
    [3.14666, -1.66646, -0.48019],
    [-0.99552, 1.95576, 0.03976],
    [0.06359, -0.21456, 1.15097],
];

/// The quadrature: 360 to 830 nm in half nanometres.
const Q_N: usize = 941;
const Q_START: f64 = 360.0;
const Q_STEP: f64 = 0.5;

/// Planck's second radiation constant, nm kelvin.
const C2: f64 = 1.4388e7;

/// The fluorescent tube's lines: centre nm, height, width nm.
const LINES: [(f64, f64, f64); 7] = [
    (405.0, 0.12, 2.5),
    (436.0, 0.55, 2.5),
    (487.0, 0.12, 3.0),
    (546.0, 1.0, 3.0),
    (577.0, 0.15, 3.0),
    (611.0, 0.95, 4.0),
    (631.0, 0.12, 4.0),
];

/// The LED's blue die and phosphor: centre nm, width nm, height.
const LED: [(f64, f64, f64); 2] = [(452.0, 10.0, 1.0), (565.0, 52.0, 0.62)];

/// The four nodes a slowly varying spectrum is sampled at, nm.
pub(super) const NODES: [f64; 4] = [430.0, 510.0, 590.0, 670.0];

/// The environment's wavelength bins over 400 to 700 nm.
pub(super) const BINS: usize = 6;

/// The five light spectra, in the order of the Spectrum choices.
pub(super) const SPECTRA: usize = 5;
pub(super) const DAYLIGHT: usize = 1;
pub(super) const TUNGSTEN: usize = 2;
pub(super) const LED_SPECTRUM: usize = 3;
pub(super) const FLUORESCENT: usize = 4;

/// The dye in the lacquer: none, passing long waves, or passing short ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Tint {
    /// 0 clear, 1 long waves pass, 2 short waves pass.
    pub kind: u32,
    pub edge: f64,
    pub width: f64,
    pub floor: f64,
}

impl Tint {
    /// The dye's two-way transmission at `l` nm.
    fn t2(&self, l: f64) -> f64 {
        let s = smoothstep(self.edge - self.width, self.edge + self.width, l);
        match self.kind {
            1 => self.floor + (1.0 - self.floor) * s,
            2 => self.floor + (1.0 - self.floor) * (1.0 - s),
            _ => 1.0,
        }
    }

    /// The aluminium under the dye, both ways through it, at `l` nm.
    pub(super) fn metal(&self, l: f64) -> f64 {
        r_al(l) * self.t2(l)
    }
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a).max(1e-9)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Aluminium's reflectance at `l` nm.
fn r_al(l: f64) -> f64 {
    0.889 - 1.25e-4 * (l.clamp(380.0, 780.0) - 450.0)
}

/// Planck's law without its constants, at `l` nm and `t` kelvin.
fn planck(l: f64, t: f64) -> f64 {
    l.powi(-5) / (C2 / (l * t)).exp_m1()
}

fn gauss(l: f64, c: f64, s: f64) -> f64 {
    let d = (l - c) / s;
    (-0.5 * d * d).exp()
}

/// The observer at `l` nm, XYZ.
fn cmf(l: f64) -> [f64; 3] {
    let mut xyz = [0.0; 3];
    for (axis, a, b, g, d) in LOBES {
        let k = if l < b { g } else { d };
        let t = (l - b) * k;
        xyz[axis] += a * (-0.5 * t * t).exp();
    }
    xyz
}

/// One raw spectrum at `l` nm, before its scale.
fn raw(s: usize, l: f64) -> f64 {
    match s {
        DAYLIGHT => planck(l, 5600.0),
        TUNGSTEN => planck(l, 3200.0),
        LED_SPECTRUM => LED.iter().map(|&(c, w, a)| a * gauss(l, c, w)).sum(),
        FLUORESCENT => {
            0.03 + LINES
                .iter()
                .map(|&(c, a, w)| a * gauss(l, c, w))
                .sum::<f64>()
        }
        _ => 1.0,
    }
}

fn q_lambda(i: usize) -> f64 {
    Q_START + Q_STEP * i as f64
}

/// Trapezoid weight of sample `i`.
fn q_weight(i: usize) -> f64 {
    if i == 0 || i == Q_N - 1 {
        0.5 * Q_STEP
    } else {
        Q_STEP
    }
}

/// What never changes: the observer in linear sRGB per sample, normalised so a
/// flat spectrum of unit radiance is white, and each spectrum's scale to equal
/// luminance.
struct Tables {
    rgb: [[f64; 3]; Q_N],
    /// One over the integral of each observer axis.
    inv_i: [f64; 3],
    scale: [f64; SPECTRA],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut sum = [0.0; 3];
        for i in 0..Q_N {
            let xyz = cmf(q_lambda(i));
            for (s, v) in sum.iter_mut().zip(xyz) {
                *s += q_weight(i) * v;
            }
        }
        let inv_i = sum.map(|s| 1.0 / s);
        let mut rgb = [[0.0; 3]; Q_N];
        for (i, out) in rgb.iter_mut().enumerate() {
            *out = to_rgb(cmf(q_lambda(i)), inv_i);
        }
        let mut scale = [1.0; SPECTRA];
        for (s, out) in scale.iter_mut().enumerate() {
            let lum: f64 = (0..Q_N)
                .map(|i| q_weight(i) * raw(s, q_lambda(i)) * cmf(q_lambda(i))[1])
                .sum();
            *out = sum[1] / lum.max(1e-30);
        }
        Tables { rgb, inv_i, scale }
    })
}

fn to_rgb(xyz: [f64; 3], inv_i: [f64; 3]) -> [f64; 3] {
    let n = [xyz[0] * inv_i[0], xyz[1] * inv_i[1], xyz[2] * inv_i[2]];
    E_TO_SRGB.map(|row| row[0] * n[0] + row[1] * n[1] + row[2] * n[2])
}

/// One closed-form term: a Gaussian of centre `c` and width `s` kept on one
/// side of `cut` (`side` -1 below, +1 above, 0 both and no slow factor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Term {
    pub amp: [f64; 3],
    pub c: f64,
    pub s: f64,
    pub cut: f64,
    pub side: f64,
}

/// The observer's fourteen half-lobes in linear sRGB, for a flat spectrum.
fn lobe_terms(k: f64) -> impl Iterator<Item = Term> {
    let t = tables();
    LOBES.into_iter().flat_map(move |(axis, a, b, g, d)| {
        let h = a * t.inv_i[axis] * k;
        let amp = [
            E_TO_SRGB[0][axis] * h,
            E_TO_SRGB[1][axis] * h,
            E_TO_SRGB[2][axis] * h,
        ];
        [(1.0 / g, -1.0), (1.0 / d, 1.0)].map(|(s, side)| Term {
            amp,
            c: b,
            s,
            cut: b,
            side,
        })
    })
}

/// The flat, daylight and tungsten list: the half-lobes alone. Daylight and
/// tungsten carry their curve as the slow factor.
pub(super) fn equal_terms() -> impl Iterator<Item = Term> {
    lobe_terms(1.0)
}

/// The LED: each half-lobe times each of its two Gaussians, again a Gaussian
/// with the same cut. Exact.
pub(super) fn led_terms() -> impl Iterator<Item = Term> {
    let k = tables().scale[LED_SPECTRUM];
    lobe_terms(1.0).flat_map(move |t| {
        LED.map(|(l0, sl, a)| {
            let v = t.s * t.s + sl * sl;
            let f = a * k * (-(t.c - l0) * (t.c - l0) / (2.0 * v)).exp();
            Term {
                amp: t.amp.map(|x| x * f),
                c: (t.c * sl * sl + l0 * t.s * t.s) / v,
                s: t.s * sl / v.sqrt(),
                cut: t.cut,
                side: t.side,
            }
        })
    })
}

/// The fluorescent tube: its seven narrow lines, the metal and dye baked in,
/// then its faint continuum as the half-lobes.
pub(super) fn fluorescent_terms(tint: Tint) -> impl Iterator<Item = Term> {
    let t = tables();
    let k = t.scale[FLUORESCENT];
    let lines = LINES.into_iter().map(move |(c, a, w)| {
        let rho = r_al(c) * tint.t2(c);
        let rgb = to_rgb(cmf(c), t.inv_i);
        Term {
            amp: rgb.map(|x| x * k * a * rho),
            c,
            s: w,
            cut: c,
            side: 0.0,
        }
    });
    lines.chain(lobe_terms(0.03 * k))
}

/// A Planck spectrum's slow factor as the kernel computes it:
/// `k * (560 / l)^5 / expm1(c2 / (l * t))`. Returns `(t, k)`, or zeros for the
/// other spectra.
pub(super) fn planck_factor(s: usize) -> [f64; 2] {
    let t = match s {
        DAYLIGHT => 5600.0,
        TUNGSTEN => 3200.0,
        _ => return [0.0, 0.0],
    };
    [t, tables().scale[s] * 560f64.powi(-5)]
}

/// Cubic Lagrange basis on the four nodes, read at `l` clamped to 400..700.
fn lagrange(l: f64) -> [f64; 4] {
    let x = l.clamp(400.0, 700.0);
    let mut out = [1.0; 4];
    for (k, o) in out.iter_mut().enumerate() {
        for (j, xj) in NODES.iter().enumerate() {
            if j != k {
                *o *= (x - xj) / (NODES[k] - xj);
            }
        }
    }
    out
}

/// What the packing needs of one spectrum under one dye: the light's own
/// colour, the 3 by 4 matrix that turns a slow spectrum sampled at the nodes
/// into the colour the foil reflects, by rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Projection {
    pub light: [f64; 3],
    pub project: [[f64; 4]; 3],
}

/// The light colour and the node projection of spectrum `s` over `tint`.
pub(super) fn projection(s: usize, tint: Tint) -> Projection {
    let t = tables();
    let mut light = [0.0; 3];
    let mut project = [[0.0; 4]; 3];
    for i in 0..Q_N {
        let l = q_lambda(i);
        let spd = q_weight(i) * t.scale[s] * raw(s, l);
        let rho = r_al(l) * tint.t2(l);
        let basis = lagrange(l);
        for c in 0..3 {
            let v = spd * t.rgb[i][c];
            light[c] += v;
            for (k, b) in basis.iter().enumerate() {
                project[c][k] += v * rho * b;
            }
        }
    }
    Projection { light, project }
}

/// The environment's bins: the colour of each 50 nm slice of the foil under a
/// flat light. They add up to the foil's colour over 400 to 700 nm.
pub(super) fn bins(tint: Tint) -> [[f64; 3]; BINS] {
    let t = tables();
    let mut out = [[0.0; 3]; BINS];
    for i in 0..Q_N {
        let l = q_lambda(i);
        if !(400.0..700.0).contains(&l) {
            continue;
        }
        let j = (((l - 400.0) / 50.0) as usize).min(BINS - 1);
        let w = q_weight(i) * r_al(l) * tint.t2(l);
        for (o, v) in out[j].iter_mut().zip(t.rgb[i]) {
            *o += w * v;
        }
    }
    out
}

/// What a speck's own dye does to the foil's colour under a flat light, as a
/// ratio per channel: the dye passes `floor` away from `peak` and all of it
/// at `peak`, `width` nm wide.
pub(super) fn speck_tint(tint: Tint, peak: f64, width: f64, floor: f64) -> [f64; 3] {
    let t = tables();
    let mut plain = [0.0; 3];
    let mut dyed = [0.0; 3];
    for i in 0..Q_N {
        let l = q_lambda(i);
        let w = q_weight(i) * r_al(l) * tint.t2(l);
        let k = floor + (1.0 - floor) * gauss(l, peak, width);
        for c in 0..3 {
            plain[c] += w * t.rgb[i][c];
            dyed[c] += w * k * t.rgb[i][c];
        }
    }
    [0, 1, 2].map(|c| (dyed[c].max(0.0) / plain[c].max(1e-3)).clamp(0.0, 4.0))
}

#[cfg(test)]
mod tests;
