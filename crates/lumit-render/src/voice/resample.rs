//! Changing one channel's rate, for the voice model, which only hears 48 kHz.
//!
//! A windowed sinc read at exact rational positions: output sample `n` sits at
//! input position `n * from / to`, so nothing drifts over a long sound and the
//! result lines up with the input. The filter is symmetric, so it adds no
//! delay, and its cutoff sits just under the lower of the two rates' Nyquist
//! frequencies, so going down a rate folds nothing back.
//!
//! # Thread role and contract
//!
//! Pure arithmetic on the calling thread, the same numbers every time.

/// Taps either side of the centre at the input's own rate. Widened when the
/// rate goes down, where the filter's cutoff falls with it. Thirty-two keeps
/// a round trip from 44.1 kHz to 48 kHz and back within 0.1 dB to 19.3 kHz,
/// with every alias 89 dB down; sixteen started to fall at 18 kHz, and as
/// the mix takes the voice from the untouched input, that top end was heard
/// in Background alone.
const HALF_TAPS: f64 = 32.0;

/// How far under Nyquist the cutoff sits, as a share of it.
const CUTOFF: f64 = 0.94;

/// The Kaiser window's shape: about 80 dB down outside the pass band.
const BETA: f64 = 8.0;

/// The most filter phases kept. Rates whose ratio needs more read the nearest
/// one, a step finer than any sound can show.
const MOST_PHASES: usize = 4096;

/// `input` at `from` samples a second, as `frames` samples at `to`.
#[must_use]
pub fn resample(input: &[f32], from: u32, to: u32, frames: usize) -> Vec<f32> {
    if from == to || from == 0 || to == 0 {
        let mut out = input.to_vec();
        out.resize(frames, 0.0);
        return out;
    }
    let shared = gcd(from, to);
    let (up, down) = (u64::from(to / shared), u64::from(from / shared));
    let scale = (f64::from(to) / f64::from(from)).min(1.0);
    let cutoff = CUTOFF * scale;
    let half = (HALF_TAPS / scale).ceil() as i64;
    let phases = usize::try_from(up).unwrap_or(MOST_PHASES).min(MOST_PHASES);
    let table = table(phases, half, cutoff);
    let taps = (2 * half) as usize;

    (0..frames as u64)
        .map(|n| {
            let at = n * down;
            let whole = (at / up) as i64;
            let phase = ((at % up) as usize * phases) / up as usize;
            let row = table.get(phase * taps..(phase + 1) * taps).unwrap_or(&[]);
            let first = whole - half + 1;
            // Away from the ends every tap has a sample: four running sums,
            // added in a fixed order, which the compiler can keep side by side.
            if let Some(near) = usize::try_from(first)
                .ok()
                .and_then(|at| input.get(at..at + taps))
            {
                let (weights, samples) = (row.chunks_exact(4), near.chunks_exact(4));
                let mut tail = 0.0f64;
                for (w, x) in weights.remainder().iter().zip(samples.remainder()) {
                    tail += f64::from(*x) * f64::from(*w);
                }
                let mut lanes = [0.0f64; 4];
                for (w, x) in weights.zip(samples) {
                    for ((sum, w), x) in lanes.iter_mut().zip(w).zip(x) {
                        *sum += f64::from(*x) * f64::from(*w);
                    }
                }
                let [a, b, c, d] = lanes;
                return ((a + b) + (c + d) + tail) as f32;
            }
            let mut sum = 0.0f64;
            for (k, weight) in row.iter().enumerate() {
                if let Some(x) = usize::try_from(first + k as i64)
                    .ok()
                    .and_then(|i| input.get(i))
                {
                    sum += f64::from(*x) * f64::from(*weight);
                }
            }
            sum as f32
        })
        .collect()
}

/// One row of `2 * half` weights per phase. Row `p` is read for an output
/// that falls `p / phases` of the way past an input sample, its first weight
/// belonging to the sample `half - 1` before it. Each row sums to one, so a
/// constant comes through at its own level.
fn table(phases: usize, half: i64, cutoff: f64) -> Vec<f32> {
    let reach = half as f64 + 1.0;
    let mut out = Vec::with_capacity(phases * (2 * half) as usize);
    for p in 0..phases {
        let frac = p as f64 / phases as f64;
        let row: Vec<f64> = (0..2 * half)
            .map(|k| {
                let x = (k - half + 1) as f64 - frac;
                let t = x * cutoff;
                let sinc = if t.abs() < 1e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t)
                };
                let edge = (x / reach).clamp(-1.0, 1.0);
                cutoff * sinc * bessel_i0(BETA * (1.0 - edge * edge).sqrt()) / bessel_i0(BETA)
            })
            .collect();
        let sum: f64 = row.iter().sum();
        let sum = if sum.abs() < 1e-12 { 1.0 } else { sum };
        out.extend(row.iter().map(|w| (w / sum) as f32));
    }
    out
}

/// The modified Bessel function of the first kind, order nought, by its
/// series, which the window's arguments make converge in a few dozen terms.
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let quarter = x * x / 4.0;
    for k in 1..64 {
        term *= quarter / (f64::from(k) * f64::from(k));
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}
