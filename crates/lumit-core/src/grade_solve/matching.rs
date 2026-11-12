//! Shot match: make one shot's tones and colour sit where another's do.
//!
//! In the target node's grading space, each channel's 1st, 50th and 99th
//! percentiles of both pictures are found, and Lift, Gamma and Gain are
//! solved per channel, through the Correction's own formula, so the target's
//! three land on the reference's. Saturation then matches the mid-tone
//! chroma.

use super::SolveError;
use super::{colours, gamma_row, percentile, pre_gain, wheel_input, wheels, RowValue, Rows};
use crate::fx::effects::correction::{maths, Correction};

/// The percentiles matched.
const PERCENTILES: [f32; 3] = [0.01, 0.5, 0.99];

/// Lift is searched over this range, in this many steps, then refined.
const LIFT_RANGE: f32 = 0.9;
const LIFT_STEPS: u32 = 360;

/// The rows that make `target`, the picture coming into `node`, match
/// `reference`. Both are scene-linear and premultiplied; a still's display
/// picture goes through [`linear_from_srgb8`] first. Sets Lift, Gamma, Gain
/// and Saturation.
pub fn shot_match(
    target: &[f32],
    reference: &[f32],
    node: &Correction,
) -> Result<Rows, SolveError> {
    let (gain, space) = pre_gain(node);
    let offset = [node.offset[0], node.offset[1], node.offset[2]];
    let tv: Vec<[f32; 3]> = colours(target)
        .map(|u| wheel_input(u, gain, space, offset))
        .collect();
    let rv: Vec<[f32; 3]> = colours(reference)
        .map(|u| u.map(|c| maths::encode(c, space)))
        .collect();
    if tv.is_empty() || rv.is_empty() {
        return Err(SolveError::Empty);
    }

    let mut lift = [0.0f32; 3];
    let mut exponent = [1.0f32; 3];
    let mut gain_row = [1.0f32; 3];
    for c in 0..3 {
        let x = points(&tv, c);
        let y = points(&rv, c);
        let (l, e, g) = fit(x, y);
        lift[c] = l;
        exponent[c] = e;
        gain_row[c] = g;
    }

    // Saturation from the mid-tone chroma, the target read after the wheels.
    let graded: Vec<[f32; 3]> = tv
        .iter()
        .map(|v| [0, 1, 2].map(|c| wheels(v[c], lift[c], exponent[c], gain_row[c])))
        .collect();
    let saturation = match (mid_chroma(&graded), mid_chroma(&rv)) {
        (Some(t), Some(r)) if t > 1e-4 => (100.0 * r / t).clamp(0.0, 200.0),
        _ => 100.0,
    };

    Ok(vec![
        ("lift", RowValue::Colour(lift)),
        ("gamma", RowValue::Colour(exponent.map(gamma_row))),
        ("gain", RowValue::Colour(gain_row)),
        ("saturation", RowValue::Float(saturation)),
    ])
}

/// A display-referred sRGB picture (eight bits, four a pixel, straight
/// alpha) as scene-linear premultiplied floats, the sRGB curve undone.
pub fn linear_from_srgb8(rgba: &[u8]) -> Vec<f32> {
    let lin = |b: u8| {
        let v = f32::from(b) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let mut out = Vec::with_capacity(rgba.len());
    for p in rgba.chunks_exact(4) {
        let a = f32::from(p[3]) / 255.0;
        out.extend_from_slice(&[lin(p[0]) * a, lin(p[1]) * a, lin(p[2]) * a, a]);
    }
    out
}

/// Channel `c`'s three percentiles.
fn points(v: &[[f32; 3]], c: usize) -> [f32; 3] {
    let mut ch: Vec<f32> = v.iter().map(|v| v[c]).collect();
    PERCENTILES.map(|p| percentile(&mut ch, p))
}

/// The mean chroma (`|Cb, Cr|`) of the pixels whose luma is a mid-tone.
fn mid_chroma(v: &[[f32; 3]]) -> Option<f32> {
    let mut sum = 0.0f64;
    let mut n = 0u32;
    for p in v {
        let c = maths::ycc(*p);
        if (0.2..=0.8).contains(&c[0]) {
            sum += f64::from(c[1].hypot(c[2]));
            n += 1;
        }
    }
    (n >= 16).then(|| (sum / f64::from(n)) as f32)
}

/// Lift, the gamma exponent and Gain taking `x` to `y`, point for point.
///
/// For each Lift, the exponent and Gain come in closed form from the top two
/// points; Lift is the one that leaves the least error over all three: a
/// coarse sweep, then a golden-section search around the best step. A flat
/// picture, or one whose top two points meet, gets the exponent 1 and Gain
/// alone. Every value is held to its row's range.
fn fit(x: [f32; 3], y: [f32; 3]) -> (f32, f32, f32) {
    if x[2] - x[0] < 1e-4 {
        let g = if x[1].abs() > 1e-4 { y[1] / x[1] } else { 1.0 };
        return (0.0, 1.0, g.clamp(0.0, 4.0));
    }
    let solve = |lift: f32| -> (f32, f32, f32) {
        let up = |v: f32| v + lift * (1.0 - v);
        let (a, b) = (up(x[1]), up(x[2]));
        let e = if a > 1e-5 && b > a + 1e-6 && y[1] > 1e-5 && y[2] > y[1] + 1e-6 {
            ((y[2] / y[1]).ln() / (b / a).ln()).clamp(0.25, 4.0)
        } else {
            1.0
        };
        let top = maths::sgnpow(b, e);
        let g = if top.abs() > 1e-6 { y[2] / top } else { 1.0 };
        let g = g.clamp(0.0, 4.0);
        let err = (0..3)
            .map(|i| (wheels(x[i], lift, e, g) - y[i]).powi(2))
            .sum::<f32>();
        (err, e, g)
    };

    let step = 2.0 * LIFT_RANGE / LIFT_STEPS as f32;
    let mut best = (f32::MAX, 0.0f32);
    for i in 0..=LIFT_STEPS {
        let lift = -LIFT_RANGE + step * i as f32;
        let err = solve(lift).0;
        if err < best.0 {
            best = (err, lift);
        }
    }
    let ratio = 0.618_034f32;
    let (mut lo, mut hi) = (best.1 - step, best.1 + step);
    for _ in 0..40 {
        let a = hi - ratio * (hi - lo);
        let b = lo + ratio * (hi - lo);
        if solve(a).0 <= solve(b).0 {
            hi = b;
        } else {
            lo = a;
        }
    }
    let lift = (0.5 * (lo + hi)).clamp(-1.0, 1.0);
    let (_, e, g) = solve(lift);
    (lift, e, g)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::grade_solve::test_support::{declared, graded, with_rows};

    /// A picture graded away from its original by known wheels, matched back
    /// to the original, lands on it.
    #[test]
    fn a_shifted_copy_matches_its_original() {
        let (w, h) = (64u32, 32u32);
        let mut original = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let t = (x as f32 + 0.5) / w as f32;
                let s = (y as f32 + 0.5) / h as f32;
                let level = 0.01 + 0.8 * t * t;
                let hue = s * std::f32::consts::TAU;
                let tint = [0, 1, 2]
                    .map(|c| 1.0 + 0.3 * (hue + c as f32 * std::f32::consts::TAU / 3.0).cos());
                original.extend(tint.iter().map(|t| level * t));
                original.push(1.0);
            }
        }
        let shift = with_rows(
            &[],
            &vec![
                ("lift", RowValue::Colour([0.05, 0.02, -0.03])),
                ("gamma", RowValue::Colour([0.1, -0.05, 0.08])),
                ("gain", RowValue::Colour([0.9, 1.05, 1.1])),
            ],
        );
        let target = graded(&original, w, h, &shift);

        let rows = shot_match(&target, &original, &with_rows(&[], &Vec::new())).unwrap();
        assert!(declared(&rows));
        let matched = graded(&target, w, h, &with_rows(&[], &rows));

        let enc = |rgba: &[f32]| -> Vec<[f32; 3]> {
            colours(rgba)
                .map(|u| u.map(|c| maths::encode(c, 0)))
                .collect()
        };
        let (m, o) = (enc(&matched), enc(&original));
        for c in 0..3 {
            let (pm, po) = (points(&m, c), points(&o, c));
            for i in 0..3 {
                assert!(
                    (pm[i] - po[i]).abs() < 0.02,
                    "channel {c}: {pm:?} vs {po:?}"
                );
            }
        }
    }
}
