//! Film's tables: the stock's curves and the two spectral steps (negative to
//! print, and film to the screen) baked into small cubes, laid end to end in
//! one buffer that both renderers read the same way.
//!
//! The cubes depend on the stock and the print stock only, so each pair is
//! worked out once a session and kept. The curves before the couplers depend on
//! Interlayer, and are cheap, so they are worked out per frame.

use std::sync::OnceLock;

use super::{data, Data, Stock, FILM_STOCKS, PRINT_STOCKS, WAVES};

/// Samples in a curve, from log exposure -3 to 4.
pub const CURVE: usize = 256;
/// The log exposure a curve starts at, and its span.
pub const LOG_E_MIN: f32 = -3.0;
pub const LOG_E_SPAN: f32 = 7.0;
/// Samples in a grain table, from no density to the layer's top.
pub const GRAIN: usize = 64;
/// A cube's side.
pub const LUT: usize = 17;
/// Where each table starts in the buffer, in floats. Each is laid out three
/// floats a sample, red first.
pub const AT_CURVE: usize = 0;
pub const AT_CURVE0: usize = AT_CURVE + CURVE * 3;
pub const AT_PRINT_CURVE: usize = AT_CURVE0 + CURVE * 3;
pub const AT_GRAIN: usize = AT_PRINT_CURVE + CURVE * 3;
pub const AT_LUT_PRINT: usize = AT_GRAIN + GRAIN * 3;
pub const AT_LUT_SCAN: usize = AT_LUT_PRINT + LUT * LUT * LUT * 3;
pub const LEN: usize = AT_LUT_SCAN + LUT * LUT * LUT * 3;
/// The lowest density the cubes cover, below a layer's own fog: grain can
/// take a pixel this far under it.
pub const DENSITY_FLOOR: f32 = -0.03;

/// What one stock and its print need beyond their curves, worked out once.
#[derive(Debug, Clone)]
pub struct Pair {
    /// Negative density to log print exposure, before the printer lights.
    pub lut_print: Vec<f32>,
    /// The density on the screen (the print's, or a slide's own) to log XYZ.
    pub lut_scan: Vec<f32>,
    /// The printer lights that print the negative's grey as a grey, log10.
    pub neutral: [f32; 3],
    /// The densities the cubes span: the negative's, then the screen's.
    pub print_top: [f32; 3],
    pub scan_low: [f32; 3],
    pub scan_top: [f32; 3],
    /// XYZ in the viewing light to linear Rec. 709.
    pub to_rec709: [[f32; 3]; 3],
}

/// Linear interpolation of `x` along increasing `xs`, held at the ends.
fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let (Some(&first), Some(&last)) = (xs.first(), xs.last()) else {
        return 0.0;
    };
    if x <= first {
        return ys.first().copied().unwrap_or(0.0);
    }
    if x >= last {
        return ys.last().copied().unwrap_or(0.0);
    }
    let i = xs.partition_point(|&v| v <= x).clamp(1, xs.len() - 1);
    let (x0, x1, y0, y1) = (xs[i - 1], xs[i], ys[i - 1], ys[i]);
    if x1 > x0 {
        y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    } else {
        y1
    }
}

fn log_e(i: usize) -> f64 {
    f64::from(LOG_E_MIN) + f64::from(LOG_E_SPAN) * i as f64 / (CURVE - 1) as f64
}

/// A curve read at log exposure `x`, `table` laid out as the buffer's.
pub fn curve_at(table: &[f32], c: usize, x: f32) -> f32 {
    let t = ((x - LOG_E_MIN) / LOG_E_SPAN * (CURVE - 1) as f32).clamp(0.0, (CURVE - 1) as f32);
    let i = (t.floor() as usize).min(CURVE - 2);
    let f = t - i as f32;
    let a = table[i * 3 + c];
    a + (table[(i + 1) * 3 + c] - a) * f
}

/// A grain table read at density `d` of a layer whose top is `top`.
pub fn grain_at(table: &[f32], c: usize, d: f32, top: f32) -> f32 {
    let t = (d / top.max(1e-6) * (GRAIN - 1) as f32).clamp(0.0, (GRAIN - 1) as f32);
    let i = (t.floor() as usize).min(GRAIN - 2);
    let f = t - i as f32;
    let a = table[i * 3 + c];
    a + (table[(i + 1) * 3 + c] - a) * f
}

/// A cube read at densities `d`, trilinearly, spanning `low..top` per axis.
pub fn cube_at(cube: &[f32], low: [f32; 3], top: [f32; 3], d: [f32; 3]) -> [f32; 3] {
    let n = (LUT - 1) as f32;
    let mut i = [0usize; 3];
    let mut f = [0.0f32; 3];
    for c in 0..3 {
        let t = ((d[c] - low[c]) / (top[c] - low[c]).max(1e-6) * n).clamp(0.0, n);
        i[c] = (t.floor() as usize).min(LUT - 2);
        f[c] = t - i[c] as f32;
    }
    let at = |r: usize, g: usize, b: usize, c: usize| cube[((b * LUT + g) * LUT + r) * 3 + c];
    let mut out = [0.0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = lerp(at(i[0], i[1], i[2], c), at(i[0] + 1, i[1], i[2], c), f[0]);
        let x10 = lerp(
            at(i[0], i[1] + 1, i[2], c),
            at(i[0] + 1, i[1] + 1, i[2], c),
            f[0],
        );
        let x01 = lerp(
            at(i[0], i[1], i[2] + 1, c),
            at(i[0] + 1, i[1], i[2] + 1, c),
            f[0],
        );
        let x11 = lerp(
            at(i[0], i[1] + 1, i[2] + 1, c),
            at(i[0] + 1, i[1] + 1, i[2] + 1, c),
            f[0],
        );
        *o = lerp(lerp(x00, x10, f[1]), lerp(x01, x11, f[1]), f[2]);
    }
    out
}

/// The light a stock lets through at dye densities `d`, per wavelength.
fn transmittance(s: &Stock, d: [f64; 3]) -> [f64; WAVES] {
    let mut out = [0.0; WAVES];
    for (l, o) in out.iter_mut().enumerate() {
        let dye = s.dye[l];
        let total = d[0] * f64::from(dye[0])
            + d[1] * f64::from(dye[1])
            + d[2] * f64::from(dye[2])
            + f64::from(s.base[l]);
        *o = 10f64.powf(-total);
    }
    out
}

/// How much each layer of `print` is exposed through the negative `neg` at
/// densities `d`, under the printer's lamp.
fn print_exposure(data: &Data, neg: &Stock, print: &Stock, d: [f64; 3]) -> [f64; 3] {
    let t = transmittance(neg, d);
    let mut raw = [0.0f64; 3];
    for (l, tl) in t.iter().enumerate() {
        let light = tl * f64::from(data.enlarger[l]);
        for (c, r) in raw.iter_mut().enumerate() {
            *r += light * f64::from(print.sens[l][c]);
        }
    }
    raw
}

/// The XYZ a stock at densities `d` shows in `light`, white being one.
fn scan_xyz(data: &Data, s: &Stock, light: &[f32], d: [f64; 3]) -> [f64; 3] {
    let t = transmittance(s, d);
    let (mut xyz, mut white) = ([0.0f64; 3], 0.0f64);
    for l in 0..WAVES {
        let lit = f64::from(light[l]);
        let cmf = data.cmf[l];
        for c in 0..3 {
            xyz[c] += t[l] * lit * f64::from(cmf[c]);
        }
        white += lit * f64::from(cmf[1]);
    }
    xyz.map(|v| v / white.max(1e-12))
}

fn log10_floor(v: f64) -> f32 {
    v.max(1e-10).log10() as f32
}

/// A cube over `low..top`, red fastest, of `f`'s three values.
fn cube(low: [f32; 3], top: [f32; 3], f: impl Fn([f64; 3]) -> [f32; 3]) -> Vec<f32> {
    let mut out = Vec::with_capacity(LUT * LUT * LUT * 3);
    let at = |c: usize, i: usize| {
        f64::from(low[c]) + f64::from(top[c] - low[c]) * i as f64 / (LUT - 1) as f64
    };
    for b in 0..LUT {
        for g in 0..LUT {
            for r in 0..LUT {
                out.extend_from_slice(&f([at(0, r), at(1, g), at(2, b)]));
            }
        }
    }
    out
}

/// Each layer's highest density.
fn tops(s: &Stock) -> [f32; 3] {
    let mut top = [0.0f32; 3];
    for v in &s.curves {
        for c in 0..3 {
            top[c] = top[c].max(v[c]);
        }
    }
    top
}

/// A curve's density at log exposure `x`.
fn density(s: &Stock, c: usize, x: f64) -> f64 {
    let t = ((x - f64::from(LOG_E_MIN)) / f64::from(LOG_E_SPAN) * (CURVE - 1) as f64)
        .clamp(0.0, (CURVE - 1) as f64);
    let i = (t.floor() as usize).min(CURVE - 2);
    let f = t - i as f64;
    let (a, b) = (f64::from(s.curves[i][c]), f64::from(s.curves[i + 1][c]));
    a + (b - a) * f
}

fn rec709(m: &[[f32; 3]; 3], xyz: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for (r, o) in out.iter_mut().enumerate() {
        *o = (0..3).map(|c| f64::from(m[r][c]) * xyz[c]).sum();
    }
    out
}

/// The printer lights that print `neg`'s grey as a grey on `print`, log10,
/// their mean held at the grey's own exposure, as a lab times a print.
fn neutral(data: &Data, neg: &Stock, print: &Stock, view: &super::View) -> [f32; 3] {
    let grey = [0, 1, 2].map(|c| density(neg, c, 0.0));
    let raw = print_exposure(data, neg, print, grey).map(|v| v.max(1e-12).log10());
    let shown = |u: [f64; 2]| {
        let mean = (u[0] + u[1]) / 3.0;
        let lights = [u[0] - mean - raw[0], -mean - raw[1], u[1] - mean - raw[2]];
        let dp = [0, 1, 2].map(|c| density(print, c, raw[c] + lights[c]));
        let rgb = rec709(&view.to_rec709, scan_xyz(data, print, &view.light, dp));
        let g = rgb[1].max(1e-9);
        (
            [
                (rgb[0].max(1e-9) / g).log10(),
                (rgb[2].max(1e-9) / g).log10(),
            ],
            lights,
        )
    };
    let mut u = [0.0f64; 2];
    for _ in 0..40 {
        let (e, _) = shown(u);
        if e[0].abs() < 1e-7 && e[1].abs() < 1e-7 {
            break;
        }
        let h = 1e-4;
        let (er, _) = shown([u[0] + h, u[1]]);
        let (eb, _) = shown([u[0], u[1] + h]);
        let j = [
            [(er[0] - e[0]) / h, (eb[0] - e[0]) / h],
            [(er[1] - e[1]) / h, (eb[1] - e[1]) / h],
        ];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if det.abs() < 1e-12 {
            break;
        }
        let step = [
            (e[0] * j[1][1] - e[1] * j[0][1]) / det,
            (e[1] * j[0][0] - e[0] * j[1][0]) / det,
        ];
        u = [
            u[0] - step[0].clamp(-0.5, 0.5),
            u[1] - step[1].clamp(-0.5, 0.5),
        ];
    }
    shown(u).1.map(|v| v as f32)
}

fn build(data: &Data, film: usize, print: Option<usize>) -> Option<Pair> {
    let neg = data.stock(FILM_STOCKS.get(film)?)?;
    let print_stock = match print {
        Some(p) => Some(data.stock(PRINT_STOCKS.get(p)?)?),
        None => None,
    };
    let screen = print_stock.unwrap_or(neg);
    let view = data.views.get(&screen.view)?;
    let neg_top = tops(neg);
    let scan_top = tops(screen);
    let floor = [DENSITY_FLOOR; 3];
    let scan_low = if print_stock.is_some() {
        [0.0; 3]
    } else {
        floor
    };
    let lut_print = match print_stock {
        Some(p) => cube(floor, neg_top, |d| {
            print_exposure(data, neg, p, d).map(log10_floor)
        }),
        None => vec![0.0; LUT * LUT * LUT * 3],
    };
    let lut_scan = cube(scan_low, scan_top, |d| {
        scan_xyz(data, screen, &view.light, d).map(log10_floor)
    });
    let neutral = match print_stock {
        Some(p) => neutral(data, neg, p, view),
        None => [0.0; 3],
    };
    Some(Pair {
        lut_print,
        lut_scan,
        neutral,
        print_top: neg_top,
        scan_low,
        scan_top,
        to_rec709: view.to_rec709,
    })
}

/// The tables for stock `film` printed on `print`, or shown as it is with
/// `None`. Worked out the first time a pair is asked for and kept.
pub fn pair(film: usize, print: Option<usize>) -> Option<&'static Pair> {
    const PRINTS: usize = PRINT_STOCKS.len() + 1;
    static CACHE: OnceLock<Vec<OnceLock<Option<Pair>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        (0..FILM_STOCKS.len() * PRINTS)
            .map(|_| OnceLock::new())
            .collect()
    });
    let slot = film * PRINTS + print.map_or(PRINT_STOCKS.len(), |p| p.min(PRINTS - 2));
    cache
        .get(slot)?
        .get_or_init(|| build(data()?, film, print))
        .as_ref()
}

/// The curve before the couplers act, such that a grey ramp still develops
/// to the stock's own curve once they have (spektrafilm's
/// `compute_density_curves_before_dir_couplers`).
fn curve_before_couplers(s: &Stock, m: &[[f32; 3]; 3], positive: bool) -> Vec<f32> {
    let top = tops(s);
    let grid: Vec<f64> = (0..CURVE).map(log_e).collect();
    let mut out = vec![0.0f32; CURVE * 3];
    for r in 0..3 {
        let shifted: Vec<f64> = (0..CURVE)
            .map(|j| {
                let held: f64 = (0..3)
                    .map(|k| {
                        let d = f64::from(s.curves[j][k]);
                        let silver = if positive { f64::from(top[k]) - d } else { d };
                        silver * f64::from(m[k][r])
                    })
                    .sum();
                grid[j] - held
            })
            .collect();
        let ys: Vec<f64> = (0..CURVE).map(|j| f64::from(s.curves[j][r])).collect();
        for (i, x) in grid.iter().enumerate() {
            out[i * 3 + r] = if positive {
                -interp(*x, &shifted, &ys.iter().map(|v| -v).collect::<Vec<_>>()) as f32
            } else {
                interp(*x, &shifted, &ys) as f32
            };
        }
    }
    out
}

/// Everything one frame's Film reads, laid out as [`AT_CURVE`] and the rest
/// say. `couplers` is the stock's matrix already scaled by Interlayer.
pub fn buffer(
    film: usize,
    pair: &Pair,
    print: Option<usize>,
    couplers: &[[f32; 3]; 3],
) -> Vec<f32> {
    let mut out = vec![0.0f32; LEN];
    let Some(d) = data() else {
        return out;
    };
    let Some(neg) = FILM_STOCKS.get(film).and_then(|id| d.stock(id)) else {
        return out;
    };
    let positive = neg.kind == "reversal";
    for (i, v) in neg.curves.iter().enumerate() {
        out[AT_CURVE + i * 3..AT_CURVE + i * 3 + 3].copy_from_slice(v);
    }
    out[AT_CURVE0..AT_CURVE0 + CURVE * 3]
        .copy_from_slice(&curve_before_couplers(neg, couplers, positive));
    if let Some(p) = print
        .and_then(|p| PRINT_STOCKS.get(p))
        .and_then(|id| d.stock(id))
    {
        for (i, v) in p.curves.iter().enumerate() {
            out[AT_PRINT_CURVE + i * 3..AT_PRINT_CURVE + i * 3 + 3].copy_from_slice(v);
        }
    }
    for (i, v) in neg.grain.iter().enumerate() {
        out[AT_GRAIN + i * 3..AT_GRAIN + i * 3 + 3].copy_from_slice(v);
    }
    out[AT_LUT_PRINT..AT_LUT_SCAN].copy_from_slice(&pair.lut_print);
    out[AT_LUT_SCAN..LEN].copy_from_slice(&pair.lut_scan);
    out
}

/// A stock's highest densities, its density at grey and the slope there:
/// what a scan of the negative is inverted around.
pub fn grey_and_slope(film: usize) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    let s = data()?.stock(FILM_STOCKS.get(film)?)?;
    let grey = [0, 1, 2].map(|c| density(s, c, 0.0) as f32);
    let slope = [0, 1, 2].map(|c| ((density(s, c, 0.1) - density(s, c, -0.1)) / 0.2) as f32);
    Some((tops(s), grey, slope))
}
