//! The fine stage: a pair's offset to a fraction of a millisecond, and its
//! drift.
//!
//! Around a dozen places the coarse line says the two inputs meet, 8 s of
//! each one's sound are compared by GCC-PHAT. Whitening the cross spectrum
//! weighs every frequency alike, so the match turns on timing rather than on
//! how each mic coloured the sound, and the peak comes out one sample wide.
//! A straight line through those offsets gives the start and the drift.

use realfft::RealFftPlanner;

use super::coarse::Found;
use super::{SyncError, Windows, RATE};

/// 8 s at 8 kHz.
const WINDOW: usize = 65_536;
/// How far either side of the coarse line to look: 40 ms.
const REACH: usize = 320;
const SIZE: usize = 131_072;
const MOST: usize = 12;
/// A window's peak must stand this far over the median of its search range.
const PEAK_RATIO: f32 = 3.0;
/// Keeps near-silent bins from being whitened up to full weight.
const REGULARISER: f32 = 1e-3;
/// Drift is only measured over this much overlap, from this many windows,
/// and only when it moves the sound by more than a millisecond.
const DRIFT_OVERLAP: f64 = 120.0;
const DRIFT_POINTS: usize = 4;
const DRIFT_SHIFT: f64 = 0.001;
/// Windows straying further than this from the line mean the drift is not
/// steady, so the pair is not sure.
const STEADY: f64 = 0.002;

/// The two inputs of a pair, as the fine stage reads them.
pub(super) struct Pair<'a> {
    pub(super) windows: &'a dyn Windows,
    pub(super) short: usize,
    pub(super) long: usize,
    pub(super) short_seconds: f64,
    pub(super) long_seconds: f64,
}

/// A pair's final line: at short time `mid` the long input is `offset`
/// ahead, and `b` is the drift, nought where it was not measured.
pub(super) struct Fit {
    pub(super) mid: f64,
    pub(super) offset: f64,
    pub(super) b: f64,
    /// False when the windows disagree or none could be read.
    pub(super) sure: bool,
}

/// Pin down the pair `found` roughly placed.
pub(super) fn refine(
    planner: &mut RealFftPlanner<f32>,
    pair: &Pair,
    found: &Found,
    drift: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Fit, SyncError> {
    let rate = f64::from(RATE);
    let fwd = planner.plan_fft_forward(SIZE);
    let inv = planner.plan_fft_inverse(SIZE);
    let mut buf = fwd.make_input_vec();
    let mut scratch = fwd.make_scratch_vec();
    let mut inv_scratch = inv.make_scratch_vec();
    let mut short_spec = fwd.make_output_vec();
    let mut cross = fwd.make_output_vec();
    let mut g = inv.make_output_vec();

    let mut points: Vec<(f64, f64)> = Vec::new();
    for t in spread(&found.times) {
        if cancelled() {
            return Err(SyncError::Cancelled);
        }
        let ss = ((t - WINDOW as f64 / 2.0 / rate) * rate).round() as i64;
        let ls = ((ss as f64 / rate + found.line.at(t)) * rate).round() as i64 - REACH as i64;
        let short = pair.windows.window(pair.short, ss as f64 / rate, WINDOW);
        let long = pair
            .windows
            .window(pair.long, ls as f64 / rate, WINDOW + 2 * REACH);
        let (Some(short), Some(long)) = (short, long) else {
            continue;
        };
        buf.fill(0.0);
        for (b, v) in buf.iter_mut().zip(&short) {
            *b = *v;
        }
        fwd.process_with_scratch(&mut buf, &mut short_spec, &mut scratch)?;
        buf.fill(0.0);
        for (b, v) in buf.iter_mut().zip(&long) {
            *b = *v;
        }
        fwd.process_with_scratch(&mut buf, &mut cross, &mut scratch)?;
        let mut total = 0f64;
        for (x, s) in cross.iter_mut().zip(&short_spec) {
            *x *= s.conj();
            total += f64::from(x.norm());
        }
        let eps = REGULARISER * (total / cross.len() as f64) as f32;
        if eps <= 0.0 {
            continue;
        }
        for x in cross.iter_mut() {
            *x /= x.norm() + eps;
        }
        if let Some(c) = cross.first_mut() {
            c.im = 0.0;
        }
        if let Some(c) = cross.last_mut() {
            c.im = 0.0;
        }
        inv.process_with_scratch(&mut cross, &mut g, &mut inv_scratch)?;
        if let Some(lag) = peak(g.get(..=2 * REACH).unwrap_or_default()) {
            let centre = (ss as f64 + WINDOW as f64 / 2.0) / rate;
            points.push((centre, (ls as f64 + lag - ss as f64) / rate));
        }
    }

    if points.is_empty() {
        let mid = median(&mut found.times.clone());
        return Ok(Fit {
            mid,
            offset: found.line.at(mid),
            b: 0.0,
            sure: false,
        });
    }
    let a0 = found.line.a;
    let overlap = pair.short_seconds.min(pair.long_seconds - a0) - (-a0).max(0.0);
    let mut b = 0.0;
    if drift && points.len() >= DRIFT_POINTS && overlap >= DRIFT_OVERLAP {
        // Theil-Sen: the median of every pair's slope shrugs off a stray window.
        let mut slopes = Vec::new();
        for (i, (t1, o1)) in points.iter().enumerate() {
            for (t2, o2) in points.iter().skip(i + 1) {
                if t2 != t1 {
                    slopes.push((o2 - o1) / (t2 - t1));
                }
            }
        }
        b = median(&mut slopes);
        if (b * overlap).abs() <= DRIFT_SHIFT {
            b = 0.0;
        }
    }
    let mut rest: Vec<f64> = points.iter().map(|(t, o)| o - b * t).collect();
    let a = median(&mut rest);
    // Where the windows sit: an unmeasured drift costs least there.
    let mid = median(&mut points.iter().map(|(t, _)| *t).collect::<Vec<_>>());
    let rms = (points
        .iter()
        .map(|(t, o)| (o - a - b * t).powi(2))
        .sum::<f64>()
        / points.len() as f64)
        .sqrt();
    // With drift off a long pair strays by design; only a fitted line is
    // held to being straight.
    Ok(Fit {
        mid,
        offset: a + b * mid,
        b,
        sure: !drift || rms <= STEADY,
    })
}

/// Up to [`MOST`] of `times`, spread evenly from first to last.
fn spread(times: &[f64]) -> Vec<f64> {
    if times.len() <= MOST {
        return times.to_vec();
    }
    (0..MOST)
        .filter_map(|i| times.get(i * (times.len() - 1) / (MOST - 1)).copied())
        .collect()
}

/// The lag of the clear peak of `|g|`, between samples by a parabola through
/// the three around it. None when it does not stand out.
fn peak(g: &[f32]) -> Option<f64> {
    let mag: Vec<f32> = g.iter().map(|v| v.abs()).collect();
    let (j, top) = mag
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))?;
    let mut sorted = mag.clone();
    let at = sorted.len() / 2;
    let median = *sorted.select_nth_unstable_by(at, f32::total_cmp).1;
    if top <= 0.0 || top < PEAK_RATIO * median {
        return None;
    }
    let side = |i: Option<usize>| i.and_then(|i| mag.get(i)).map(|v| f64::from(*v));
    let frac = match (side(j.checked_sub(1)), side(j.checked_add(1))) {
        (Some(alpha), Some(gamma)) => {
            let beta = f64::from(top);
            let den = alpha - 2.0 * beta + gamma;
            if den < 0.0 {
                (0.5 * (alpha - gamma) / den).clamp(-0.5, 0.5)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    Some(j as f64 + frac)
}

/// The median of `x`, sorting it; the mean of the middle two when even.
fn median(x: &mut [f64]) -> f64 {
    x.sort_by(f64::total_cmp);
    let n = x.len();
    match (x.get(n.wrapping_sub(1) / 2), x.get(n / 2)) {
        (Some(lo), Some(hi)) => 0.5 * (lo + hi),
        _ => 0.0,
    }
}
