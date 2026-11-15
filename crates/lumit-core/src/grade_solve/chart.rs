//! The colour chart: an X-Rite ColorChecker Classic (24 patches) in the
//! picture, matched to its published values with a 3x3 matrix and an
//! exposure, worked in scene linear.
//!
//! Each patch's middle half is read (the median of each channel), a matrix is
//! fitted by weighted least squares with the six greys counting more, and
//! Exposure then puts Neutral 5 where the chart has it. The error of each
//! patch before and after is given in delta E 2000.

use super::{percentile, RowValue, Rows, SolveError};
use crate::fx::cpu;
use crate::fx::effects::correction::{maths, Correction};
use crate::linalg::{cholesky, cholesky_solve, Dense};

/// Patches across and down.
pub const COLUMNS: usize = 6;
pub const ROWS: usize = 4;
pub const PATCHES: usize = COLUMNS * ROWS;

/// The patch whose level Exposure matches: Neutral 5, the fourth grey.
pub const NEUTRAL_5: usize = 21;

/// How much more a grey counts in the fit.
const GREY_WEIGHT: f64 = 4.0;

/// The chart's patches in CIE Lab under D50, row by row from dark skin at
/// the top left to black at the bottom right. X-Rite's values for charts made
/// from November 2014 on.
pub const CHART_LAB: [[f32; 3]; PATCHES] = [
    [37.54, 14.37, 14.92],
    [64.66, 19.27, 17.5],
    [49.32, -3.82, -22.54],
    [43.46, -12.74, 22.72],
    [54.94, 9.61, -24.79],
    [70.48, -32.26, -0.37],
    [62.73, 35.83, 56.5],
    [39.43, 10.75, -45.17],
    [50.57, 48.64, 16.67],
    [30.1, 22.54, -20.87],
    [71.77, -24.13, 58.19],
    [71.51, 18.24, 67.37],
    [28.37, 15.42, -49.8],
    [54.38, -39.72, 32.27],
    [42.43, 51.05, 28.62],
    [81.8, 2.67, 80.41],
    [50.63, 51.28, -14.12],
    [49.57, -29.71, -28.32],
    [95.19, -1.03, 2.93],
    [81.29, -0.57, 0.44],
    [66.89, -0.75, -0.06],
    [50.76, -0.13, 0.14],
    [35.63, -0.46, -0.48],
    [20.64, 0.07, -0.46],
];

/// D50's white, the white the chart's Lab is measured against.
const D50: [f64; 3] = [0.964_22, 1.0, 0.825_21];

/// Bradford's adaptation from D50 to D65, and back.
const BRADFORD_D50_D65: [[f64; 3]; 3] = [
    [0.955_576_6, -0.023_039_3, 0.063_163_6],
    [-0.028_289_5, 1.009_941_6, 0.021_007_7],
    [0.012_298_2, -0.020_483_0, 1.329_909_8],
];
const BRADFORD_D65_D50: [[f64; 3]; 3] = [
    [1.047_811_2, 0.022_886_6, -0.050_127_0],
    [0.029_542_4, 0.990_484_4, -0.017_049_1],
    [-0.009_234_5, 0.015_043_6, 0.752_131_6],
];

/// Linear Rec. 709 to XYZ (D65), and back.
const RGB_XYZ: [[f64; 3]; 3] = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175_0],
    [0.019_333_9, 0.119_192_0, 0.950_304_1],
];
const XYZ_RGB: [[f64; 3]; 3] = [
    [3.240_454_2, -1.537_138_5, -0.498_531_4],
    [-0.969_266_0, 1.876_010_8, 0.041_556_0],
    [0.055_643_4, -0.204_025_9, 1.057_225_2],
];

/// The working space's primaries against linear Rec. 709, both ways. A
/// project working in an OCIO config's `scene_linear` (ACEScg, say) has its
/// own; Lumit's own working space is Rec. 709 itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Primaries {
    pub from_rec709: [[f64; 3]; 3],
    pub to_rec709: [[f64; 3]; 3],
}

impl Primaries {
    pub const REC709: Self = Self {
        from_rec709: IDENTITY,
        to_rec709: IDENTITY,
    };
}

const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// What a chart match writes and how far each patch was out.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartMatch {
    pub rows: Rows,
    /// Delta E 2000 of each patch, in chart order, as the picture came and
    /// after the rows.
    pub before: [f32; PATCHES],
    pub after: [f32; PATCHES],
}

/// Each patch's colour (scene linear, unpremultiplied), the median of each
/// channel over the patch's middle half. `corners` are the chart's top left,
/// top right, bottom right and bottom left in raster pixels, dark skin at
/// the top left. A patch with nothing readable under it is `None`.
pub fn chart_samples(
    rgba: &[f32],
    width: u32,
    height: u32,
    corners: [[f32; 2]; 4],
) -> [Option<[f32; 3]>; PATCHES] {
    let at = face(corners);
    const GRID: u32 = 12;
    std::array::from_fn(|i| {
        let (col, row) = ((i % COLUMNS) as f32, (i / COLUMNS) as f32);
        let mut ch: [Vec<f32>; 3] = Default::default();
        for gy in 0..GRID {
            for gx in 0..GRID {
                let fu = 0.25 + 0.5 * (gx as f32 + 0.5) / GRID as f32;
                let fv = 0.25 + 0.5 * (gy as f32 + 0.5) / GRID as f32;
                let p = at((col + fu) / COLUMNS as f32, (row + fv) / ROWS as f32);
                if !(p[0] >= 0.0 && p[1] >= 0.0) {
                    continue;
                }
                let (x, y) = (p[0] as u32, p[1] as u32);
                if x >= width || y >= height {
                    continue;
                }
                let k = ((y * width + x) * 4) as usize;
                let Some(px) = rgba.get(k..k + 4) else {
                    continue;
                };
                let a = px[3];
                if a < 0.5 || !px.iter().all(|c| c.is_finite()) {
                    continue;
                }
                for c in 0..3 {
                    ch[c].push(px[c] / a);
                }
            }
        }
        (!ch[0].is_empty()).then(|| {
            let [r, g, b] = &mut ch;
            [percentile(r, 0.5), percentile(g, 0.5), percentile(b, 0.5)]
        })
    })
}

/// The point at (`u`, `v`) across the chart's face, 0..1 each way, seen in
/// perspective: the projective map that takes the unit square's corners to
/// `corners` (Heckbert's square to quad). A chart turned away from the camera
/// shrinks into the distance, which a straight blend of the corners misses.
fn face(corners: [[f32; 2]; 4]) -> impl Fn(f32, f32) -> [f32; 2] {
    let [p0, p1, p2, p3] = corners.map(|p| p.map(f64::from));
    let s = [0, 1].map(|k| p0[k] - p1[k] + p2[k] - p3[k]);
    let d1 = [0, 1].map(|k| p1[k] - p2[k]);
    let d2 = [0, 1].map(|k| p3[k] - p2[k]);
    let den = d1[0] * d2[1] - d2[0] * d1[1];
    // Four corners in a line have no perspective to find; the affine part
    // still places them.
    let (g, h) = if den.abs() > 1e-12 {
        (
            (s[0] * d2[1] - d2[0] * s[1]) / den,
            (d1[0] * s[1] - s[0] * d1[1]) / den,
        )
    } else {
        (0.0, 0.0)
    };
    move |u, v| {
        let (u, v) = (f64::from(u), f64::from(v));
        let w = g * u + h * v + 1.0;
        [0, 1].map(|k| {
            let x = (p1[k] - p0[k] + g * p1[k]) * u + (p3[k] - p0[k] + h * p3[k]) * v + p0[k];
            // Behind the camera: nowhere on the picture.
            if w > 1e-9 {
                (x / w) as f32
            } else {
                f32::NAN
            }
        })
    }
}

/// The rows that take the chart in `samples` (from [`chart_samples`], read
/// off the picture coming into `node`) to its published colours: Space
/// Linear, the three mixer rows and Exposure. The node's Temperature and
/// Tint act before the mixer and are allowed for.
///
/// The published Lab is under D50; it goes to XYZ, through Bradford to D65,
/// into linear Rec. 709, and then into the project's working primaries,
/// which `samples` are in too.
pub fn chart_match(
    samples: &[Option<[f32; 3]>; PATCHES],
    node: &Correction,
    working: &Primaries,
) -> Result<ChartMatch, SolveError> {
    let reference: [[f64; 3]; PATCHES] =
        std::array::from_fn(|i| mul64(&working.from_rec709, reference_rgb(i)));
    let (gain_r, gain_b) = cpu::temperature_gains((node.temperature / 100.0).clamp(-2.0, 2.0));
    let gain_g = cpu::tint_gain((node.tint / 100.0).clamp(-2.0, 2.0));
    let pre = [gain_r, gain_g, gain_b].map(f64::from);

    // Normal equations, one output row at a time, all sharing one matrix.
    let mut ata = Dense::zero(3);
    let mut atb = [[0.0f64; 3]; 3];
    let mut read = 0;
    for (i, s) in samples.iter().enumerate() {
        let Some(s) = s else { continue };
        read += 1;
        let s = [0, 1, 2].map(|c| f64::from(s[c]) * pre[c]);
        let w = if i >= PATCHES - COLUMNS {
            GREY_WEIGHT
        } else {
            1.0
        };
        for r in 0..3 {
            for c in 0..3 {
                ata.add(r, c, w * s[r] * s[c]);
            }
            for (k, row) in atb.iter_mut().enumerate() {
                row[r] += w * reference[i][k] * s[r];
            }
        }
    }
    if read < 4 {
        return Err(SolveError::Empty);
    }
    // A touch of damping, so a chart with too little colour still solves.
    for d in 0..3 {
        ata.add(d, d, 1e-9);
    }
    let l = cholesky(&ata).ok_or(SolveError::Empty)?;
    let fitted: [[f64; 3]; 3] = std::array::from_fn(|k| {
        let row = cholesky_solve(&l, &atb[k]);
        [0, 1, 2].map(|c| row.get(c).copied().unwrap_or(0.0))
    });

    // The mixer keeps white's luma; Exposure carries the level.
    let white = (0..3)
        .map(|r| f64::from(maths::LUMA[r]) * fitted[r].iter().sum::<f64>())
        .sum::<f64>();
    let scale = if white.abs() > 1e-9 { white } else { 1.0 };
    let mixer: [[f32; 3]; 3] = fitted.map(|row| row.map(|v| ((v / scale) as f32).clamp(-2.0, 2.0)));
    let exposure = match samples[NEUTRAL_5] {
        Some(n5) => {
            let s = [0, 1, 2].map(|c| n5[c] * pre[c] as f32);
            let got = maths::luma(mul(&mixer, s));
            let want = maths::luma(reference[NEUTRAL_5].map(|v| v as f32));
            if got > 1e-6 && want > 1e-6 {
                (want / got).log2().clamp(-20.0, 20.0)
            } else {
                0.0
            }
        }
        None => (scale.max(1e-9).log2() as f32).clamp(-20.0, 20.0),
    };

    let lab_ref: [[f64; 3]; PATCHES] = CHART_LAB.map(|l| l.map(f64::from));
    let delta = |i: usize, rgb: [f32; 3]| {
        let rec709 = mul64(&working.to_rec709, rgb.map(f64::from));
        de2000(lab_of(rec709), lab_ref[i]) as f32
    };
    let gain = exposure.exp2();
    let mut before = [0.0f32; PATCHES];
    let mut after = [0.0f32; PATCHES];
    for (i, s) in samples.iter().enumerate() {
        let Some(s) = s else { continue };
        before[i] = delta(i, *s);
        let s = [0, 1, 2].map(|c| s[c] * pre[c] as f32 * gain);
        after[i] = delta(i, mul(&mixer, s));
    }

    Ok(ChartMatch {
        rows: vec![
            ("space", RowValue::Choice(2)),
            ("mixer_red", RowValue::Colour(mixer[0])),
            ("mixer_green", RowValue::Colour(mixer[1])),
            ("mixer_blue", RowValue::Colour(mixer[2])),
            ("exposure", RowValue::Float(exposure)),
        ],
        before,
        after,
    })
}

fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

fn mul64(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

/// Patch `i`'s published colour in linear Rec. 709 (D65).
pub fn reference_rgb(i: usize) -> [f64; 3] {
    let lab = CHART_LAB.get(i).copied().unwrap_or_default().map(f64::from);
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let inv = |f: f64| {
        if f > 6.0 / 29.0 {
            f * f * f
        } else {
            3.0 * (6.0f64 / 29.0).powi(2) * (f - 4.0 / 29.0)
        }
    };
    let xyz50 = [inv(fx) * D50[0], inv(fy) * D50[1], inv(fz) * D50[2]];
    mul64(&XYZ_RGB, mul64(&BRADFORD_D50_D65, xyz50))
}

/// A linear Rec. 709 colour in Lab under D50, Bradford-adapted.
fn lab_of(rgb: [f64; 3]) -> [f64; 3] {
    let xyz = mul64(&BRADFORD_D65_D50, mul64(&RGB_XYZ, rgb));
    let f = |t: f64| {
        if t > (6.0f64 / 29.0).powi(3) {
            t.cbrt()
        } else {
            t / (3.0 * (6.0f64 / 29.0).powi(2)) + 4.0 / 29.0
        }
    };
    let [fx, fy, fz] = [0, 1, 2].map(|k| f(xyz[k] / D50[k]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 between two Lab colours (Sharma, Wu and Dalal's formulation).
fn de2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let deg = |r: f64| r * 180.0 / PI;
    let rad = |d: f64| d * PI / 180.0;
    let c1 = a[1].hypot(a[2]);
    let c2 = b[1].hypot(b[2]);
    let cm = 0.5 * (c1 + c2);
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1, a2) = ((1.0 + g) * a[1], (1.0 + g) * b[1]);
    let (c1p, c2p) = (a1.hypot(a[2]), a2.hypot(b[2]));
    let hue = |y: f64, x: f64| {
        if x == 0.0 && y == 0.0 {
            0.0
        } else {
            let h = deg(y.atan2(x));
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let (h1, h2) = (hue(a[2], a1), hue(b[2], a2));
    let dl = b[0] - a[0];
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (rad(dh) / 2.0).sin();
    let lm = 0.5 * (a[0] + b[0]);
    let cmp = 0.5 * (c1p + c2p);
    let hm = if c1p * c2p == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        0.5 * (h1 + h2)
    } else if h1 + h2 < 360.0 {
        0.5 * (h1 + h2 + 360.0)
    } else {
        0.5 * (h1 + h2 - 360.0)
    };
    let t = 1.0 - 0.17 * rad(hm - 30.0).cos()
        + 0.24 * rad(2.0 * hm).cos()
        + 0.32 * rad(3.0 * hm + 6.0).cos()
        - 0.20 * rad(4.0 * hm - 63.0).cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -(rad(2.0 * dtheta)).sin() * rc;
    let (l, c, h) = (dl / sl, dc / sc, dhh / sh);
    (l * l + c * c + h * h + rt * c * h).max(0.0).sqrt()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::grade_solve::test_support::{declared, with_rows};

    /// Sharma's published pairs land on their published differences.
    #[test]
    fn delta_e_2000_matches_the_published_pairs() {
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([2.0776, 0.0795, -1.135], [0.9033, -0.0636, -0.5514], 0.9082),
        ];
        for (a, b, want) in pairs {
            assert!((de2000(a, b) - want).abs() < 1e-3, "{a:?} {b:?}");
        }
    }

    /// A chart drawn through a known cast and level, turned away from the
    /// camera, is read, and the fit takes it back to the published colours.
    /// A straight blend of the corners would read the far patches off their
    /// borders.
    #[test]
    fn a_chart_through_a_known_matrix_is_recovered() {
        let skew = [[0.9, 0.12, 0.0], [0.05, 0.85, 0.1], [0.0, 0.15, 1.1]];
        let level = 0.6;
        // Chart units (0..6 across, 0..4 down) to pixels, its right side
        // further away, and back by the adjugate. A dark border runs between
        // patches.
        let q: [[f64; 3]; 3] = [[90.0, 8.0, 40.0], [2.0, 95.0, 30.0], [0.08, 0.004, 1.0]];
        let to_px = |u: f64, v: f64| {
            let p = mul64(&q, [u, v, 1.0]);
            [(p[0] / p[2]) as f32, (p[1] / p[2]) as f32]
        };
        let back = [
            [
                q[1][1] * q[2][2] - q[1][2] * q[2][1],
                q[0][2] * q[2][1] - q[0][1] * q[2][2],
                q[0][1] * q[1][2] - q[0][2] * q[1][1],
            ],
            [
                q[1][2] * q[2][0] - q[1][0] * q[2][2],
                q[0][0] * q[2][2] - q[0][2] * q[2][0],
                q[0][2] * q[1][0] - q[0][0] * q[1][2],
            ],
            [
                q[1][0] * q[2][1] - q[1][1] * q[2][0],
                q[0][1] * q[2][0] - q[0][0] * q[2][1],
                q[0][0] * q[1][1] - q[0][1] * q[1][0],
            ],
        ];
        let (w, h) = (640u32, 460u32);
        let mut rgba = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let k = ((y * w + x) * 4) as usize;
                rgba[k + 3] = 1.0;
                let c = mul64(&back, [f64::from(x), f64::from(y), 1.0]);
                let (u, v) = ((c[0] / c[2]) as f32, (c[1] / c[2]) as f32);
                if !(0.0..6.0).contains(&u) || !(0.0..4.0).contains(&v) {
                    continue;
                }
                if u.fract() < 0.08 || v.fract() < 0.08 {
                    continue;
                }
                let i = v as usize * COLUMNS + u as usize;
                let r = reference_rgb(i).map(|c| c as f32);
                let shown = mul(&skew.map(|row| row.map(|c: f32| c * level)), r);
                rgba[k..k + 3].copy_from_slice(&shown);
            }
        }
        let corners = [
            to_px(0.0, 0.0),
            to_px(6.0, 0.0),
            to_px(6.0, 4.0),
            to_px(0.0, 4.0),
        ];
        let samples = chart_samples(&rgba, w, h, corners);
        let node = with_rows(&[], &Vec::new());
        let solved = chart_match(&samples, &node, &Primaries::REC709).unwrap();
        assert!(declared(&solved.rows));

        let mean = |e: &[f32; PATCHES]| e.iter().sum::<f32>() / PATCHES as f32;
        assert!(mean(&solved.before) > 5.0, "{:?}", solved.before);
        assert!(mean(&solved.after) < 0.5, "{:?}", solved.after);
        assert!(solved.after[NEUTRAL_5] < 0.5);
    }
}
