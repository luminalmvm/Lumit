//! The timeline spectrogram: what a stretch of sound *contains*, by frequency,
//! summarised once and answerable at any zoom (docs/09 §4).
//!
//! In plain terms: a waveform says how loud each moment is; a spectrogram says
//! what is in it — kicks along the bottom, hats along the top, a voice in the
//! middle. This module runs the same short-window FFT the beat detector runs
//! (docs/impl/beat-detection.md's STFT sizing), folds each frame's spectrum
//! into a few dozen log-spaced bands, and keeps the result as a small grid of
//! bytes: one column per analysis hop, one row per band, brightness in dB.
//!
//! Like the waveform's peak pyramid ([`crate::peaks`]), the grid is kept at
//! **three levels of detail** — the analysis hop, then two tiers folded eight
//! times coarser each — so a lane fitted to a whole song and a lane zoomed to
//! one bar both answer with a handful of merges per pixel column. Folding
//! takes the **maximum**, not the mean: a transient is the thing worth seeing,
//! and averaging it away zoomed out would hide exactly what the lane is for.
//!
//! Everything here is pure arithmetic over samples decoded elsewhere, so all
//! of it is a plain deterministic test.

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

/// Frequency bands per column. Enough to tell a kick from a snare from a hat
/// in a lane a few tens of pixels tall; a lane never has more rows of pixels
/// than this has bands at the heights the Timeline draws.
pub const BINS: usize = 40;

/// The top of the picture, in Hz — the board's own caption ("0–12 kHz").
/// Musical energy above it is hats and air, already summarised by the top
/// band's fold.
pub const MAX_HZ: f32 = 12_000.0;

/// The bottom of the log spacing, in Hz. Bands below it would each cover less
/// than one FFT bin at the analysis window and read as stretched noise.
const MIN_HZ: f32 = 40.0;

/// The dB floor: a byte of 0. Full scale is a byte of 255; the 60 dB span is
/// what a picture can show before the quiet end is one shade of black anyway.
const FLOOR_DB: f32 = -60.0;

/// How much coarser each tier is than the one below it.
const TIER_RATIO: usize = 8;

/// How many tiers the grid holds.
const TIERS: usize = 3;

/// One level of detail: `cols` columns of [`BINS`] bytes, column-major.
struct Tier {
    seconds_per_col: f64,
    cols: usize,
    values: Vec<u8>,
}

/// A whole source's spectrogram, at every zoom.
pub struct Spectrogram {
    tiers: Vec<Tier>,
    duration_seconds: f64,
}

/// STFT sizing for `rate` Hz — the beat detector's own (~43 ms window, a
/// quarter-window hop), so the two analyses describe the same moments.
fn window_hop(rate: u32) -> (usize, usize) {
    let target = (0.043 * f64::from(rate)).round() as usize;
    let window = target.next_power_of_two().clamp(256, 8192);
    (window, (window / 4).max(1))
}

impl Spectrogram {
    /// Build the grid from interleaved stereo at `rate` Hz. Empty input, or
    /// input shorter than one window, builds an empty grid that answers
    /// silence — the honest picture of a source with nothing to show.
    #[must_use]
    pub fn build(interleaved: &[f32], rate: u32) -> Spectrogram {
        let mono = crate::beat::downmix_stereo(interleaved);
        let duration_seconds = mono.len() as f64 / f64::from(rate.max(1));
        let (window, hop) = window_hop(rate.max(1));
        if mono.len() < window {
            return Spectrogram {
                tiers: Vec::new(),
                duration_seconds,
            };
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

        // Which band each FFT bin folds into, worked out once: log-spaced
        // between MIN_HZ and MAX_HZ, everything below the bottom into band 0,
        // everything above the top dropped.
        let hz_per_bin = rate as f32 / window as f32;
        let ratio = (MAX_HZ / MIN_HZ).ln() / BINS as f32;
        let band_of = |bin: usize| -> Option<usize> {
            let hz = bin as f32 * hz_per_bin;
            if hz > MAX_HZ {
                return None;
            }
            if hz <= MIN_HZ {
                return Some(0);
            }
            Some((((hz / MIN_HZ).ln() / ratio) as usize).min(BINS - 1))
        };
        // A full-scale sine under a Hann window lands about window/4 in its
        // bin; dividing by that reads full scale as 0 dB whatever the rate.
        let scale = 4.0 / window as f32;

        let n_frames = (mono.len() - window) / hop + 1;
        let mut finest = vec![0u8; n_frames * BINS];
        for f in 0..n_frames {
            let start = f * hop;
            for (i, w) in hann.iter().enumerate() {
                input[i] = mono[start + i] * w;
            }
            if fft.process(&mut input, &mut spectrum).is_err() {
                break;
            }
            let mut bands = [0f32; BINS];
            for (bin, c) in spectrum.iter().enumerate() {
                let Some(b) = band_of(bin) else { continue };
                let mag = c.norm() * scale;
                if mag > bands[b] {
                    bands[b] = mag;
                }
            }
            let col = &mut finest[f * BINS..(f + 1) * BINS];
            for (b, &mag) in bands.iter().enumerate() {
                let db = if mag > 0.0 {
                    20.0 * mag.log10()
                } else {
                    FLOOR_DB
                };
                let unit = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
                col[b] = (unit * 255.0).round() as u8;
            }
        }

        // The coarser tiers fold down by maximum, exactly as the peak pyramid
        // folds its blocks — a transient survives every zoom.
        let mut tiers = vec![Tier {
            seconds_per_col: hop as f64 / f64::from(rate),
            cols: n_frames,
            values: finest,
        }];
        for _ in 1..TIERS {
            let below = &tiers[tiers.len() - 1];
            let cols = below.cols.div_ceil(TIER_RATIO);
            let mut values = vec![0u8; cols * BINS];
            for (c, col) in values.chunks_exact_mut(BINS).enumerate() {
                for src in c * TIER_RATIO..((c + 1) * TIER_RATIO).min(below.cols) {
                    let from = &below.values[src * BINS..(src + 1) * BINS];
                    for (b, v) in col.iter_mut().enumerate() {
                        *v = (*v).max(from[b]);
                    }
                }
            }
            tiers.push(Tier {
                seconds_per_col: below.seconds_per_col * TIER_RATIO as f64,
                cols,
                values,
            });
        }
        Spectrogram {
            tiers,
            duration_seconds,
        }
    }

    /// Whether there is anything here at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tiers.is_empty()
    }

    /// How long the source runs.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// What the grid costs to keep, for the session cache's byte budget.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.tiers.iter().map(|t| t.values.len()).sum()
    }

    /// The window `[start_s, end_s)` in `cols` columns of [`BINS`] bytes,
    /// column-major — the lane's ask, one column per pixel.
    ///
    /// Columns outside the audio come back silent rather than missing, so a
    /// caller's column index and a byte offset always agree.
    #[must_use]
    pub fn range(&self, start_s: f64, end_s: f64, cols: usize) -> Vec<u8> {
        let mut out = vec![0u8; cols * BINS];
        if self.tiers.is_empty() || end_s <= start_s || !(end_s - start_s).is_finite() || cols == 0
        {
            return out;
        }
        let step = (end_s - start_s) / cols as f64;
        for (c, col) in out.chunks_exact_mut(BINS).enumerate() {
            let a = start_s + step * c as f64;
            self.window_into(a, a + step, col);
        }
        out
    }

    /// The loudest each band gets across `[a, b)`, written into `out` — the
    /// per-column query a retimed caller maps through its own clock.
    pub fn window_into(&self, a: f64, b: f64, out: &mut [u8]) {
        out.fill(0);
        if self.tiers.is_empty() || b <= a || !(b - a).is_finite() {
            return;
        }
        // The coarsest tier whose columns are still finer than the ask, so a
        // column costs at most TIER_RATIO merges; a window finer than the
        // finest tier reads the one column it lands in.
        let tier = self
            .tiers
            .iter()
            .rev()
            .find(|t| t.seconds_per_col <= (b - a))
            .unwrap_or(&self.tiers[0]);
        let first = (a / tier.seconds_per_col).floor().max(0.0) as usize;
        let last = ((b / tier.seconds_per_col).ceil() as usize).min(tier.cols);
        for src in first..last.max(first + 1).min(tier.cols) {
            let from = &tier.values[src * BINS..(src + 1) * BINS];
            for (i, v) in out.iter_mut().enumerate().take(BINS) {
                *v = (*v).max(from[i]);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Interleaved stereo of one sine at `hz`, `seconds` long.
    fn sine(hz: f32, seconds: f32, rate: u32) -> Vec<f32> {
        let n = (seconds * rate as f32) as usize;
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin();
            out.push(s);
            out.push(s);
        }
        out
    }

    /// The band a frequency lands in, mirroring the build's fold.
    fn band(hz: f32) -> usize {
        let ratio = (MAX_HZ / MIN_HZ).ln() / BINS as f32;
        if hz <= MIN_HZ {
            0
        } else {
            (((hz / MIN_HZ).ln() / ratio) as usize).min(BINS - 1)
        }
    }

    #[test]
    fn a_tone_lights_its_own_band_and_not_the_others() {
        let grid = Spectrogram::build(&sine(1_000.0, 1.0, 48_000), 48_000);
        assert!(!grid.is_empty());
        let cols = grid.range(0.25, 0.75, 8);
        assert_eq!(cols.len(), 8 * BINS);
        let lit = band(1_000.0);
        for c in 0..8 {
            let col = &cols[c * BINS..(c + 1) * BINS];
            assert!(col[lit] > 200, "the tone's band is bright: {}", col[lit]);
            // Two bands clear of the tone, near silence.
            assert!(col[band(100.0)] < 60, "a distant band stays dark");
        }
    }

    #[test]
    fn every_tier_answers_the_same_tone() {
        // A window per column so coarse it must come off the coarsest tier,
        // and one so fine it must come off the finest: both see the tone.
        let grid = Spectrogram::build(&sine(4_000.0, 2.0, 48_000), 48_000);
        let lit = band(4_000.0);
        let coarse = grid.range(0.0, 2.0, 2);
        assert!(coarse[lit] > 200 && coarse[BINS + lit] > 200);
        let fine = grid.range(1.0, 1.01, 1);
        assert!(fine[lit] > 200);
    }

    #[test]
    fn outside_the_audio_is_silence_not_a_missing_column() {
        let grid = Spectrogram::build(&sine(440.0, 1.0, 48_000), 48_000);
        let cols = grid.range(2.0, 3.0, 4);
        assert_eq!(cols.len(), 4 * BINS);
        assert!(cols.iter().all(|&v| v == 0));
    }

    #[test]
    fn too_short_to_analyse_is_an_empty_grid() {
        let grid = Spectrogram::build(&[0.0; 64], 48_000);
        assert!(grid.is_empty());
        assert!(grid.range(0.0, 1.0, 4).iter().all(|&v| v == 0));
    }
}
