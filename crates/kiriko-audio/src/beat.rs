//! Beat detection: spectral-flux onsets and a BPM estimate
//! (docs/impl/beat-detection.md). The boring, well-trodden MIR pipeline done
//! carefully — no neural models.
//!
//! In plain terms: we slide a short window along the audio, and each step ask
//! "how much *new* energy appeared since the last step?" (the spectral flux).
//! Sudden jumps — a kick, a snare — make that number spike, and the spikes are
//! the beats. Autocorrelating the spike train recovers the tempo (BPM).

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

/// One detected onset: time in seconds and a 0..1 confidence (its prominence).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Onset {
    pub time: f64,
    pub confidence: f32,
}

/// The result of analysing an audio buffer for beats.
#[derive(Debug, Clone)]
pub struct BeatAnalysis {
    /// Detected onsets in time order.
    pub onsets: Vec<Onset>,
    /// Estimated tempo in beats per minute (0.0 if indeterminate).
    pub bpm: f64,
    /// Onset-envelope frame rate (frames per second), for callers that want
    /// to draw the curve.
    pub env_fps: f64,
}

/// STFT frame sizing for `rate` Hz: ~43 ms window (a power of two) and a
/// quarter-window hop (~10.7 ms → ~93.75 fps at 48 kHz).
fn window_hop(rate: u32) -> (usize, usize) {
    let target = (0.043 * f64::from(rate)).round() as usize;
    let window = target.next_power_of_two().clamp(256, 8192);
    (window, (window / 4).max(1))
}

/// Average interleaved stereo (L R L R…) down to mono.
pub fn downmix_stereo(interleaved: &[f32]) -> Vec<f32> {
    interleaved
        .chunks_exact(2)
        .map(|s| 0.5 * (s[0] + s[1]))
        .collect()
}

/// The onset envelope: positive spectral flux per STFT frame, on a
/// log-compressed magnitude spectrum (`L = ln(1 + 100·|X|)`), Hann-windowed.
fn onset_envelope(mono: &[f32], window: usize, hop: usize) -> Vec<f32> {
    if mono.len() < window {
        return Vec::new();
    }
    let hann: Vec<f32> = (0..window)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (window as f32 - 1.0)).cos())
        })
        .collect();
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(window);
    let mut input = fft.make_input_vec();
    let mut spectrum: Vec<Complex<f32>> = fft.make_output_vec();
    let bins = spectrum.len();
    let mut prev_log = vec![0f32; bins];

    let log_mag = |spectrum: &[Complex<f32>], out: &mut [f32]| {
        for (k, c) in spectrum.iter().enumerate() {
            out[k] = (1.0 + 100.0 * c.norm()).ln();
        }
    };

    let n_frames = (mono.len() - window) / hop + 1;
    let mut env = Vec::with_capacity(n_frames);
    for f in 0..n_frames {
        let start = f * hop;
        for (i, w) in hann.iter().enumerate() {
            input[i] = mono[start + i] * w;
        }
        if fft.process(&mut input, &mut spectrum).is_err() {
            break;
        }
        let mut cur = vec![0f32; bins];
        log_mag(&spectrum, &mut cur);
        if f == 0 {
            // Prime the reference; the first frame has no predecessor to
            // difference against (else it reads as one giant false onset).
            prev_log = cur;
            env.push(0.0);
            continue;
        }
        let mut flux = 0f32;
        for k in 0..bins {
            let d = cur[k] - prev_log[k];
            if d > 0.0 {
                flux += d;
            }
        }
        prev_log = cur;
        env.push(flux);
    }
    env
}

/// The value at `frac` (0..1) through a sorted slice — a cheap percentile.
fn percentile(sorted: &[f32], frac: f64) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((frac * (sorted.len() as f64 - 1.0)).round() as usize).min(sorted.len() - 1);
    sorted[idx]
}

/// Peak-pick onsets from the envelope with an adaptive threshold
/// (docs/impl/beat-detection.md §2). `sensitivity` is δ (default 1.5; lower =
/// more markers). Times use the frame centre and parabolic sub-frame refining.
fn pick_onsets(
    env: &[f32],
    env_fps: f64,
    window: usize,
    hop: usize,
    sensitivity: f32,
) -> Vec<Onset> {
    let n = env.len();
    if n < 8 {
        return Vec::new();
    }
    let mut sorted = env.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lambda = 0.05 * percentile(&sorted, 0.9);
    let max_env = sorted[n - 1].max(1e-9);
    let delta = sensitivity.max(0.1);

    let local = 3usize; // ±32 ms local-max window
    let win = ((0.46 * env_fps).round() as usize).max(1); // ±460 ms adaptive window
    let debounce = 3usize;
    // Frame centre in samples → seconds, so an onset lands on the transient,
    // not a half-window early.
    let centre = window as f64 / 2.0;

    let mut onsets = Vec::new();
    let mut last: Option<usize> = None;
    for i in local..n.saturating_sub(local) {
        if !(i - local..=i + local).all(|j| env[i] >= env[j]) {
            continue;
        }
        let lo = i.saturating_sub(win);
        let hi = (i + win + 1).min(n);
        let mean = env[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        if env[i] < mean * delta + lambda {
            continue;
        }
        if let Some(l) = last {
            if i - l < debounce {
                continue;
            }
        }
        let (a, b, c) = (env[i - 1], env[i], env[i + 1]);
        let denom = a - 2.0 * b + c;
        let offset = if denom.abs() > 1e-9 {
            f64::from(0.5 * (a - c) / denom)
        } else {
            0.0
        };
        let time = ((i as f64 + offset) * hop as f64 + centre) / (env_fps * hop as f64);
        onsets.push(Onset {
            time,
            confidence: (env[i] / max_env).clamp(0.0, 1.0),
        });
        last = Some(i);
    }
    onsets
}

/// Estimate BPM by autocorrelating the (mean-removed, half-wave-rectified)
/// envelope with a harmonic comb, preferring the octave in 70–180 BPM
/// (docs/impl/beat-detection.md §3). Parabolic peak refining gives sub-lag
/// precision so the estimate isn't quantised to the frame rate.
fn estimate_bpm(env: &[f32], env_fps: f64) -> f64 {
    let n = env.len();
    if n < 16 || env_fps <= 0.0 {
        return 0.0;
    }
    let mean = env.iter().sum::<f32>() / n as f32;
    let x: Vec<f32> = env.iter().map(|v| (v - mean).max(0.0)).collect();
    let autocorr = |lag: usize| -> f32 {
        if lag == 0 || lag >= n {
            return 0.0;
        }
        (lag..n).map(|i| x[i] * x[i - lag]).sum()
    };
    let comb = |lag: usize| autocorr(lag) + 0.5 * autocorr(2 * lag) + 0.33 * autocorr(3 * lag);

    let lag_min = ((env_fps * 60.0 / 240.0).round() as usize).max(1); // 240 BPM
    let lag_max = ((env_fps * 60.0 / 30.0).round() as usize).min(n - 1); // 30 BPM
    if lag_min >= lag_max {
        return 0.0;
    }
    let mut best = (0usize, f32::MIN);
    for lag in lag_min..=lag_max {
        let bpm = env_fps * 60.0 / lag as f64;
        let pref = if (70.0..=180.0).contains(&bpm) {
            1.0
        } else {
            0.85
        };
        let s = comb(lag) * pref;
        if s > best.1 {
            best = (lag, s);
        }
    }
    if best.0 == 0 {
        return 0.0;
    }
    // Parabolic refine around the winning lag (on the raw comb score).
    let l = best.0;
    let refined = if l > lag_min && l < lag_max {
        let (a, b, c) = (comb(l - 1), comb(l), comb(l + 1));
        let denom = a - 2.0 * b + c;
        l as f64
            + if denom.abs() > 1e-9 {
                f64::from(0.5 * (a - c) / denom)
            } else {
                0.0
            }
    } else {
        l as f64
    };
    env_fps * 60.0 / refined
}

/// Grid assist (docs/impl/beat-detection.md §3): snap onset times to the tempo
/// grid implied by `bpm`. Each onset within `tolerance` seconds of a grid line
/// moves onto it; the rest stay. The grid's phase is the circular mean of the
/// onsets modulo the beat period, so the grid aligns to the actual beats and
/// the small analysis latency in the raw onsets is removed. Returns the onsets
/// unchanged if `bpm <= 0` or there are fewer than two.
pub fn snap_to_grid(onsets: &[f64], bpm: f64, tolerance: f64) -> Vec<f64> {
    if bpm <= 0.0 || onsets.len() < 2 {
        return onsets.to_vec();
    }
    let period = 60.0 / bpm;
    // Circular mean of the phases (onset mod period), so wrap-around near a
    // beat boundary averages correctly.
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    for &t in onsets {
        let angle = (t / period).rem_euclid(1.0) * std::f64::consts::TAU;
        sx += angle.cos();
        sy += angle.sin();
    }
    let phi = sy.atan2(sx).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * period;
    onsets
        .iter()
        .map(|&t| {
            let k = ((t - phi) / period).round();
            let grid = phi + k * period;
            if (t - grid).abs() <= tolerance {
                grid
            } else {
                t
            }
        })
        .collect()
}

/// Analyse a mono buffer at `rate` Hz for onsets and tempo. `sensitivity` is
/// the peak-picking δ (1.5 is a sensible default).
pub fn analyse_mono(mono: &[f32], rate: u32, sensitivity: f32) -> BeatAnalysis {
    let (window, hop) = window_hop(rate);
    let env_fps = f64::from(rate) / hop as f64;
    let env = onset_envelope(mono, window, hop);
    let onsets = pick_onsets(&env, env_fps, window, hop, sensitivity);
    let bpm = estimate_bpm(&env, env_fps);
    BeatAnalysis {
        onsets,
        bpm,
        env_fps,
    }
}

/// Analyse an interleaved-stereo buffer (the media hand-off format).
pub fn analyse_stereo(interleaved: &[f32], rate: u32, sensitivity: f32) -> BeatAnalysis {
    analyse_mono(&downmix_stereo(interleaved), rate, sensitivity)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A mono buffer of short percussive clicks at the given beat times, over
    /// faint noise (deterministic pseudo-random so tests are reproducible).
    fn clicks(rate: u32, beat_times: &[f64], secs: f64) -> Vec<f32> {
        let n = (secs * f64::from(rate)) as usize;
        let mut buf = vec![0f32; n];
        // Cheap deterministic noise floor.
        let mut seed = 0x1234_5678u32;
        for s in buf.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *s = ((seed >> 8) as f32 / f32::from(u16::MAX) - 0.5) * 0.01;
        }
        // Each click: a short exponentially-decaying burst (broadband onset).
        for &t in beat_times {
            let start = (t * f64::from(rate)) as usize;
            for k in 0..(rate as usize / 100) {
                let i = start + k;
                if i >= n {
                    break;
                }
                let env = (-(k as f32) / (rate as f32 * 0.003)).exp();
                let seed2 = (i as u32).wrapping_mul(2_246_822_519);
                let noise = (seed2 >> 8) as f32 / f32::from(u16::MAX) - 0.5;
                buf[i] += env * noise * 2.0;
            }
        }
        buf
    }

    #[test]
    fn finds_120bpm_clicks_accurately() {
        let rate = 48_000;
        let beats: Vec<f64> = (0..8).map(|i| 0.5 + i as f64 * 0.5).collect(); // 120 BPM
        let audio = clicks(rate, &beats, 5.0);
        let a = analyse_mono(&audio, rate, 1.5);

        // Every beat is matched within ~2 envelope frames (raw onsets carry an
        // inherent analysis latency of up to a frame or two; the grid-snap that
        // §3 layers on top removes the residual, this just proves detection).
        for &b in &beats {
            let hit = a.onsets.iter().any(|o| (o.time - b).abs() <= 0.025);
            assert!(hit, "missed beat at {b}s; onsets = {:?}", a.onsets);
        }
        assert!(
            a.onsets.len() <= beats.len() + 1,
            "too many onsets: {:?}",
            a.onsets
        );
        // Tempo lands on 120, not an octave, to within 2 BPM.
        assert!((a.bpm - 120.0).abs() < 2.0, "bpm was {}", a.bpm);
    }

    #[test]
    fn silence_and_tiny_buffers_are_safe() {
        let a = analyse_mono(&[0.0; 1000], 48_000, 1.5);
        assert!(a.onsets.is_empty());
        let b = analyse_mono(&[], 48_000, 1.5);
        assert!(b.onsets.is_empty() && b.bpm == 0.0);
    }

    #[test]
    fn stereo_downmix_averages_channels() {
        assert_eq!(downmix_stereo(&[1.0, 3.0, -2.0, 0.0]), vec![2.0, -1.0]);
    }

    #[test]
    fn grid_snap_aligns_jittered_onsets() {
        // Onsets a few ms off a 120 BPM grid (period 0.5 s).
        let onsets = [0.008, 0.494, 1.006, 1.997, 2.489];
        let snapped = snap_to_grid(&onsets, 120.0, 0.045);
        // Consecutive snapped onsets are exact grid multiples apart.
        for w in snapped.windows(2) {
            let gap = w[1] - w[0];
            let beats = (gap / 0.5).round();
            assert!(
                (gap - beats * 0.5).abs() < 1e-6,
                "gap {gap} not on the grid: {snapped:?}"
            );
        }
        // Nothing moved more than the tolerance.
        for (o, s) in onsets.iter().zip(&snapped) {
            assert!((o - s).abs() <= 0.045 + 1e-9);
        }
        // Beyond tolerance, an outlier is left alone.
        let out = snap_to_grid(&[0.0, 0.5, 0.73], 120.0, 0.045);
        assert!((out[2] - 0.73).abs() < 1e-9);
        // Degenerate inputs pass through.
        assert_eq!(snap_to_grid(&[1.0], 120.0, 0.045), vec![1.0]);
        assert_eq!(snap_to_grid(&[1.0, 2.0], 0.0, 0.045), vec![1.0, 2.0]);
    }
}
