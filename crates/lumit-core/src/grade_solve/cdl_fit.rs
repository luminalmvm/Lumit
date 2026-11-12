//! A Correction's grade as ASC CDL numbers, for handing to another tool.
//!
//! **In plain terms.** Offset, lift, gamma, gain and saturation on their own
//! are exactly a CDL: lift and offset fold into the slope and offset, gain
//! into both through the power, and saturation is the same sum. When other
//! rows move too, no CDL can say exactly what the node does, so the numbers
//! are a least-squares fit: each channel's slope, offset and power to a grey
//! ramp through the node, then the saturation to a ramp of colours. Either
//! way the numbers are in the node's grading space, and the node is read as
//! if its key took the whole picture.

use crate::fx::effects::correction::{maths, packed::Packed, Correction};

/// CDL numbers, and whether they say exactly what the node does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CdlFit {
    pub slope: [f32; 3],
    pub offset: [f32; 3],
    pub power: [f32; 3],
    pub saturation: f32,
    pub exact: bool,
}

/// Points on the grey ramp the fit reads.
const RAMP: usize = 33;

/// Whether only the rows a CDL can hold have moved.
fn primaries_only(pk: &Packed) -> bool {
    pk.pre_gain == [1.0; 3]
        && pk.shadow == [0.0; 3]
        && pk.midtone == [0.0; 3]
        && pk.highlight == [0.0; 3]
        && pk.lum_mix == 1.0
        && pk.contrast == 1.0
        && pk.hue == [1.0, 0.0]
        && pk.boost == 0.0
        && pk.tonal == [0.0; 2]
        && !pk.mixer_on
        && !pk.curves_on
        && pk.knees == [0.0, 1.0]
        && !pk.hue_curves_on
        && !pk.hdr.on
}

/// The node's grade of `v`, in its grading space, keyed whole.
fn graded(pk: &Packed, v: [f32; 3]) -> [f32; 3] {
    let u = v.map(|c| maths::decode(c, pk.space));
    let out = maths::grade([u[0], u[1], u[2], 1.0], 1.0, maths::luma(v), pk);
    [0, 1, 2].map(|c| maths::encode(out[c], pk.space))
}

/// One channel's slope and offset by least squares of `y ≈ s·x + o`.
fn line(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let (sx, sy) = (xs.iter().sum::<f64>(), ys.iter().sum::<f64>());
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let d = n * sxx - sx * sx;
    if d.abs() < 1e-12 {
        return (1.0, 0.0);
    }
    let s = (n * sxy - sx * sy) / d;
    (s, (sy - s * sx) / n)
}

/// The SOP of one channel, clamped as the list clamps.
fn sop(x: f64, s: f64, o: f64, p: f64) -> f64 {
    let v = (x * s + o).clamp(0.0, 1.0);
    if v > 0.0 {
        v.powf(p)
    } else {
        0.0
    }
}

/// For power `p`, the slope and offset that fit `t` best, and the squared
/// error they leave.
fn at_power(xs: &[f64], t: &[f64], p: f64) -> (f64, f64, f64) {
    let pre: Vec<f64> = t.iter().map(|v| v.powf(1.0 / p)).collect();
    let (s, o) = line(xs, &pre);
    let err = xs
        .iter()
        .zip(t)
        .map(|(x, y)| (sop(*x, s, o, p) - y).powi(2))
        .sum();
    (s, o, err)
}

/// One channel's slope, offset and power for the targets `t` over `xs`: the
/// best of a sweep of powers, then narrowed by golden section.
fn fit_channel(xs: &[f64], t: &[f64]) -> (f64, f64, f64) {
    let powers: Vec<f64> = (0..=60)
        .map(|i| 0.2 * 25f64.powf(i as f64 / 60.0))
        .collect();
    let errs: Vec<f64> = powers.iter().map(|p| at_power(xs, t, *p).2).collect();
    let best = errs
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i);
    let (mut lo, mut hi) = (
        powers[best.saturating_sub(1)],
        powers[(best + 1).min(powers.len() - 1)],
    );
    let g = 0.5 * (5f64.sqrt() - 1.0);
    for _ in 0..40 {
        let a = hi - g * (hi - lo);
        let b = lo + g * (hi - lo);
        if at_power(xs, t, a).2 <= at_power(xs, t, b).2 {
            hi = b;
        } else {
            lo = a;
        }
    }
    let p = 0.5 * (lo + hi);
    let (s, o, _) = at_power(xs, t, p);
    (s, o, p)
}

/// The CDL that does what `node` does, exactly when it can.
#[must_use]
pub fn cdl_of(node: &Correction) -> CdlFit {
    let mut pk = node.packed(1.0);
    pk.colour_only();
    pk.mix = 1.0;
    pk.detail = 0.0;
    if primaries_only(&pk) {
        // G·((1 − L)(x + O) + L)^e = (G^(1/e)(1 − L)·x + G^(1/e)((1 − L)O + L))^e.
        let k = [0, 1, 2].map(|c| pk.gain[c].max(0.0).powf(1.0 / pk.gamma[c]));
        return CdlFit {
            slope: [0, 1, 2].map(|c| k[c] * (1.0 - pk.lift[c])),
            offset: [0, 1, 2].map(|c| k[c] * ((1.0 - pk.lift[c]) * pk.offset[c] + pk.lift[c])),
            power: pk.gamma,
            saturation: pk.sat,
            exact: true,
        };
    }
    // A fit: each channel to the grey ramp, then saturation to colours.
    let xs: Vec<f64> = (0..RAMP).map(|i| i as f64 / (RAMP - 1) as f64).collect();
    let greys: Vec<[f32; 3]> = xs.iter().map(|x| graded(&pk, [*x as f32; 3])).collect();
    let mut slope = [1.0f32; 3];
    let mut offset = [0.0f32; 3];
    let mut power = [1.0f32; 3];
    for c in 0..3 {
        let t: Vec<f64> = greys
            .iter()
            .map(|g| f64::from(g[c]).clamp(0.0, 1.0))
            .collect();
        let (s, o, p) = fit_channel(&xs, &t);
        (slope[c], offset[c], power[c]) = (s as f32, o as f32, p as f32);
    }
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    let hues = [
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
    ];
    for level in [0.3f32, 0.55, 0.8] {
        for h in hues {
            let v = h.map(|c| level * (0.4 + 0.6 * c));
            let t = graded(&pk, v);
            let x = [0, 1, 2].map(|c| {
                sop(
                    f64::from(v[c]),
                    f64::from(slope[c]),
                    f64::from(offset[c]),
                    f64::from(power[c]),
                )
            });
            let y = 0.2126 * x[0] + 0.7152 * x[1] + 0.0722 * x[2];
            for c in 0..3 {
                num += (x[c] - y) * (f64::from(t[c]) - y);
                den += (x[c] - y) * (x[c] - y);
            }
        }
    }
    CdlFit {
        slope,
        offset,
        power,
        saturation: if den > 1e-12 {
            (num / den).max(0.0) as f32
        } else {
            1.0
        },
        exact: false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::cdl::Cdl;
    use crate::fx::effects::correction::packed::HUE_FLAT;
    use crate::fx::params::Params;
    use crate::fx::registry::EffectMetadata;
    use crate::grade_solve::test_support::with_rows;
    use crate::grade_solve::RowValue;

    /// Wheels and saturation come out as the CDL that grades the same: a
    /// CDL node with the numbers matches the Correction pixel for pixel,
    /// inside the 0..1 the list clamps to.
    #[test]
    fn primaries_export_as_the_same_grade() {
        let mut node = with_rows(
            &[],
            &vec![
                ("lift", RowValue::Colour([0.05, 0.02, 0.0])),
                ("gamma", RowValue::Colour([0.1, 0.0, -0.15])),
                ("gain", RowValue::Colour([1.1, 1.0, 0.9])),
                ("offset", RowValue::Colour([0.01, 0.0, 0.02])),
                ("saturation", RowValue::Float(90.0)),
            ],
        );
        // Rows read from nothing leave the hue curves on the diagonal.
        let flat = crate::fx::params::CurvePoints::sanitised(&HUE_FLAT);
        for c in [
            &mut node.hue_hue,
            &mut node.hue_sat,
            &mut node.hue_lum,
            &mut node.lum_sat,
            &mut node.sat_sat,
            &mut node.sat_lum,
        ] {
            *c = flat;
        }
        let fit = cdl_of(&node);
        assert!(fit.exact);
        let mut cdl = Cdl::read(Params::EMPTY);
        cdl.slope = [fit.slope[0], fit.slope[1], fit.slope[2], 1.0];
        cdl.offset = [fit.offset[0], fit.offset[1], fit.offset[2], 1.0];
        cdl.power = [fit.power[0], fit.power[1], fit.power[2], 1.0];
        cdl.saturation = fit.saturation;
        let cdl = cdl.packed();
        let pk = node.packed(1.0);
        for i in 0..64 {
            let t = i as f32 / 63.0;
            let u = [0.6 * t, 0.4 * t * t, 0.25 + 0.5 * t].map(|c| c * 0.8);
            let want = maths::grade([u[0], u[1], u[2], 1.0], 1.0, 0.0, &pk);
            let got = cdl.pixel([u[0], u[1], u[2], 1.0]);
            for c in 0..3 {
                assert!((want[c] - got[c]).abs() < 1e-4, "{i}: {want:?} {got:?}");
            }
        }
    }
}
