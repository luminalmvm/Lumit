//! Auto balance: take the cast out and set the black and white points.
//!
//! The cast is read grey-world over the quietest mid-tone pixels and solved
//! with Temperature's own maths, so the rows read in the units the sliders
//! use. Then, in the node's grading space, black is the 0.5th percentile of
//! luma and white the 99.5th, and Lift and Gain put each channel's black at 0
//! and its white at [`WHITE`].

use super::{colours, mean, percentile, wheel_input, RowValue, Rows, SolveError};
use crate::fx::cpu;
use crate::fx::effects::correction::{maths, Correction};
use crate::fx::effects::temperature::neutral_for;

/// Where white lands, in the grading space: a little under 1, so the
/// brightest detail keeps some room.
pub const WHITE: f32 = 0.95;

/// The rows that neutralise `rgba`, the picture coming into `node`. Sets
/// Temperature, Tint, Lift and Gain and leaves everything else alone.
pub fn auto_balance(rgba: &[f32], node: &Correction) -> Result<Rows, SolveError> {
    let picture: Vec<[f32; 3]> = colours(rgba).collect();
    if picture.is_empty() {
        return Err(SolveError::Empty);
    }

    // Mid-tones by their gamma 2.4 luma, whatever the node's space, then the
    // quietest quarter of them by saturation: the pixels most likely grey.
    let tone = |u: &[f32; 3]| maths::encode(maths::luma(*u), 0);
    let quiet = |u: &[f32; 3]| {
        let top = u[0].max(u[1]).max(u[2]);
        if top > 1e-6 {
            maths::chroma(*u) / top
        } else {
            0.0
        }
    };
    let mid: Vec<&[f32; 3]> = picture
        .iter()
        .filter(|u| (0.15..=0.85).contains(&tone(u)))
        .collect();
    let pool: Vec<&[f32; 3]> = if mid.is_empty() {
        picture.iter().collect()
    } else {
        mid
    };
    let mut sats: Vec<f32> = pool.iter().map(|u| quiet(u)).collect();
    let limit = percentile(&mut sats, 0.25).min(0.5);
    let grey = mean(pool.iter().filter(|u| quiet(u) <= limit).map(|u| **u))
        .or_else(|| mean(pool.iter().map(|u| **u)))
        .ok_or(SolveError::Empty)?;

    // The picture comes in before this node, so nothing of its own
    // Temperature is on it yet.
    let (temperature, tint) = neutral_for(grey, (0.0, 0.0));

    // The new gains, with the node's own Exposure kept.
    let expo = node.exposure.clamp(-20.0, 20.0).exp2();
    let (gain_r, gain_b) = cpu::temperature_gains((temperature / 100.0).clamp(-2.0, 2.0));
    let gain_g = cpu::tint_gain((tint / 100.0).clamp(-2.0, 2.0));
    let gain = [gain_r * expo, gain_g * expo, gain_b * expo];
    let space = node.packed(1.0).space;

    let offset = [node.offset[0], node.offset[1], node.offset[2]];
    let v: Vec<[f32; 3]> = picture
        .iter()
        .map(|u| wheel_input(*u, gain, space, offset))
        .collect();
    let mut lumas: Vec<f32> = v.iter().map(|v| maths::luma(*v)).collect();
    let black = percentile(&mut lumas, 0.005);
    let white = percentile(&mut lumas, 0.995);
    let lows = mean(v.iter().filter(|v| maths::luma(**v) <= black).copied());
    let highs = mean(v.iter().filter(|v| maths::luma(**v) >= white).copied());

    let mut lift = [0.0f32; 3];
    let mut gain_row = [1.0f32; 3];
    if let (Some(lows), Some(highs)) = (lows, highs) {
        for c in 0..3 {
            let e = (-2.0 * node.gamma[c].clamp(-4.0, 4.0)).exp2();
            let (l, g) = black_and_white(lows[c], highs[c], e);
            lift[c] = l;
            gain_row[c] = g;
        }
    }

    Ok(vec![
        ("temperature", RowValue::Float(temperature)),
        ("tint", RowValue::Float(tint)),
        ("lift", RowValue::Colour(lift)),
        ("gain", RowValue::Colour(gain_row)),
    ])
}

/// Lift and Gain that put `black` at 0 and `white` at [`WHITE`] through a
/// gamma exponent `e`. Lift 0 and Gain 1 where the two meet or white is
/// already past 1.
fn black_and_white(black: f32, white: f32, e: f32) -> (f32, f32) {
    if black >= 1.0 - 1e-4 || white <= black + 1e-4 {
        return (0.0, 1.0);
    }
    // `x + L·(1 − x) = 0` at black.
    let lift = (-black / (1.0 - black)).clamp(-1.0, 1.0);
    let top = white + lift * (1.0 - white);
    if top <= 1e-4 {
        return (lift, 1.0);
    }
    (lift, (WHITE / maths::sgnpow(top, e)).clamp(0.0, 4.0))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::grade_solve::test_support::{declared, graded, with_rows};

    /// A grey ramp with a few quiet colours in it, under a warm, green cast,
    /// comes back neutral with its black at 0 and its white near 0.95.
    #[test]
    fn a_cast_picture_comes_back_neutral() {
        let (w, h) = (64u32, 16u32);
        let cast = [1.18, 1.06, 0.8];
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let t = (x as f32 + 0.5) / w as f32;
                let level = 0.02 + 0.85 * t * t;
                // Colours in the middle of two rows in four, so the picture
                // is not all grey and the grey-world pool has to pick.
                let tint = match (y % 4, (0.25..0.75).contains(&t)) {
                    (1, true) => [1.3, 0.9, 0.8],
                    (3, true) => [0.8, 1.2, 0.9],
                    _ => [1.0, 1.0, 1.0],
                };
                for c in 0..3 {
                    rgba.push(level * tint[c] * cast[c]);
                }
                rgba.push(1.0);
            }
        }
        let node = with_rows(&[], &Vec::new());
        let rows = auto_balance(&rgba, &node).unwrap();
        assert!(declared(&rows));
        let out = graded(&rgba, w, h, &with_rows(&[], &rows));

        // A grey pixel of the ramp (row 0) in the middle.
        let at = |x: u32| {
            let i = (x * 4) as usize;
            [0, 1, 2].map(|c| maths::encode(out[i + c], 0))
        };
        let mid = at(w / 2);
        assert!(
            (mid[0] - mid[1]).abs() < 0.02 && (mid[2] - mid[1]).abs() < 0.02,
            "{mid:?}"
        );
        let black = at(0);
        let white = at(w - 1);
        assert!(black.iter().all(|c| c.abs() < 0.03), "{black:?}");
        assert!(white.iter().all(|c| (c - WHITE).abs() < 0.03), "{white:?}");
    }
}
