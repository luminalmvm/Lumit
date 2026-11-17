//! DeepFilterNet's signal path around its three graphs: the window, the band
//! layout, the running feature averages, the filter and the overlap-add, ported
//! number for number from libDF so the graphs hear exactly what they were
//! trained on and the result lines up with the input sample for sample.
//!
//! # Thread role and contract
//!
//! Pure arithmetic on the calling thread. [`run`] is handed the graphs as a
//! closure, so everything here is tested without a model.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use realfft::{num_complex::Complex, ComplexToReal, RealFftPlanner, RealToComplex};

use crate::{error::MlError, manifest::deep_filter::DeepFilter};

use super::RATE;

/// How the frames are split into runs of the graphs.
#[derive(Debug, Clone, Copy)]
pub(super) struct Chunks {
    /// Frames each run answers for.
    pub size: usize,
    /// Frames run before those and thrown away, so the model's recurrent
    /// layers have settled by the first frame that counts.
    pub warm: usize,
    /// Frames over which one run's answer fades into the next's.
    pub fade: usize,
}

/// A minute a run, 30 seconds of settling before it and a one-second
/// handover. The recurrent layers take a long time to forget where they
/// started: after ten seconds of settling a run still drifted from one long
/// run for twenty seconds in quiet passages, and thirty seconds brings the
/// whole result to about 70 dB of it. A run holds about 400 MB of the
/// encoder's output, which is what caps the minute.
pub(super) const CHUNKS: Chunks = Chunks {
    size: 6000,
    warm: 3000,
    fade: 100,
};

/// The pack's numbers as the loops want them.
#[derive(Debug, Clone, Copy)]
pub(super) struct Shape {
    pub fft: usize,
    pub hop: usize,
    pub bins: usize,
    pub nb_erb: usize,
    pub nb_df: usize,
    pub order: usize,
    /// Frames of the filter's span before the one it makes.
    pub behind: usize,
    /// Frames ahead the encoder reads.
    pub ahead: usize,
    pub alpha: f32,
}

impl Shape {
    pub fn of(names: &DeepFilter) -> Self {
        let fft = names.fft_size as usize;
        let order = names.df_order as usize;
        Shape {
            fft,
            hop: names.hop_size as usize,
            bins: fft / 2 + 1,
            nb_erb: names.nb_erb as usize,
            nb_df: names.nb_df as usize,
            order,
            behind: order.saturating_sub(1 + names.df_lookahead as usize),
            ahead: names.conv_lookahead as usize,
            alpha: names.norm_alpha,
        }
    }
}

/// The longest transform a pack may ask for. DeepFilterNet's is 960, and
/// every buffer and loop here is sized by it, so a number read off disk is
/// held to it before anything is allocated.
const MOST_FFT: usize = 4096;

/// The most frames the filter may span, or the encoder read ahead. Five and
/// two in DeepFilterNet3.
const MOST_FRAMES: u32 = 16;

/// The band layout for a pack whose numbers this code can run, or `None`.
///
/// The window is only a perfect reconstruction at half overlap, and the bands
/// must cover every bin exactly once. Every number that sizes a buffer or a
/// loop has a ceiling as well as a floor, because `addon.json` can be
/// installed from disk with anything in it.
pub(super) fn runnable(names: &DeepFilter) -> Option<Vec<usize>> {
    let shape = Shape::of(names);
    let fits = names.sr == RATE
        && shape.hop > 0
        && shape.fft == 2 * shape.hop
        && shape.fft <= MOST_FFT
        && (2..=shape.bins).contains(&shape.nb_erb)
        && (names.min_nb_erb_freqs as usize) <= shape.bins
        && (2..=shape.bins).contains(&shape.nb_df)
        && names.df_order <= MOST_FRAMES
        && names.conv_lookahead <= MOST_FRAMES
        && names.df_lookahead < names.df_order
        && names.norm_alpha > 0.0
        && names.norm_alpha < 1.0;
    if !fits {
        return None;
    }
    let bands = bands(
        names.sr,
        shape.fft,
        shape.nb_erb,
        names.min_nb_erb_freqs as usize,
    );
    (bands.iter().sum::<usize>() == shape.bins).then_some(bands)
}

/// DeepFilterNet's band layout: equal steps on the ERB scale, each band at
/// least `min_width` bins wide, the last one taking whatever is left. The
/// arithmetic is libDF's own, in single precision, so the bands land on the
/// same bins.
fn bands(sr: u32, fft: usize, count: usize, min_width: usize) -> Vec<usize> {
    let to_erb = |hz: f32| 9.265 * (hz / (24.7 * 9.265)).ln_1p();
    let to_hz = |erb: f32| 24.7 * 9.265 * ((erb / 9.265).exp() - 1.0);
    let bin_width = sr as f32 / fft as f32;
    let low = to_erb(0.0);
    let step = (to_erb((sr / 2) as f32) - low) / count as f32;
    let min_width = min_width as i64;
    let mut widths = Vec::with_capacity(count);
    let (mut previous, mut over) = (0i64, 0i64);
    for band in 1..=count {
        let edge = (to_hz(low + band as f32 * step) / bin_width).round() as i64;
        let mut width = edge - previous - over;
        if width < min_width {
            over = min_width - width;
            width = min_width;
        } else {
            over = 0;
        }
        widths.push(width.max(0) as usize);
        previous = edge;
    }
    let bins = fft / 2 + 1;
    if let Some(last) = widths.last_mut() {
        *last += 1;
        let extra = widths.iter().sum::<usize>().saturating_sub(bins);
        if let Some(last) = widths.last_mut() {
            *last = last.saturating_sub(extra);
        }
    }
    widths
}

/// What one run of the graphs said, for frames from `first` on.
pub(super) struct Answer {
    pub first: usize,
    pub gains: Vec<f32>,
    pub coefs: Vec<f32>,
}

impl Answer {
    fn gains(&self, t: usize, shape: Shape) -> &[f32] {
        let at = (t - self.first) * shape.nb_erb;
        &self.gains[at..at + shape.nb_erb]
    }

    fn coefs(&self, t: usize, shape: Shape) -> &[f32] {
        let per = shape.nb_df * shape.order * 2;
        let at = (t - self.first) * per;
        &self.coefs[at..at + per]
    }
}

/// The graphs: asked for frames `first..end`, reading their features off the
/// [`Spectra`], answering with band gains and filter taps for each.
pub(super) type Graphs<'g> = dyn FnMut(&Spectra, usize, usize) -> Result<Answer, MlError> + 'g;

/// Every channel with the model's answer applied, written over the channel
/// itself, so the answer costs no memory beyond its input. On an error the
/// channels hold part of each.
///
/// In place is safe because the samples are read well ahead of where they
/// are written: frame `t` reads the samples from a hop before `t`'s to a hop
/// after it, its sound is written over the first of those hops, and by then
/// every frame up to the encoder's lookahead past `t` has been made.
///
/// The model hears the mean of the channels, and what it answers is applied
/// to each channel's own spectrum. Both halves of the answer are linear in
/// the spectrum, so the mean of what comes back is exactly what applying it
/// to the mean would give, and a voice off to one side stays there.
pub(super) fn run(
    shape: Shape,
    bands: &[usize],
    channels: &mut [Vec<f32>],
    stop: &AtomicBool,
    chunks: Chunks,
    graphs: &mut Graphs<'_>,
) -> Result<(), MlError> {
    let len = channels.first().map_or(0, Vec::len);
    if channels.iter().any(|c| c.len() != len) {
        return Err(MlError::ShapeMismatch);
    }
    if len == 0 {
        return Ok(());
    }
    let mut spectra = Spectra::new(shape, bands.to_vec(), len, channels.len());
    let window = vorbis(shape.fft);
    let mut overlaps: Vec<Overlap> = channels
        .iter()
        .map(|_| Overlap::new(shape, window.clone()))
        .collect();
    let band_of: Vec<usize> = bands
        .iter()
        .enumerate()
        .flat_map(|(band, width)| std::iter::repeat_n(band, *width))
        .collect();
    let mut frame = vec![Complex::new(0.0, 0.0); shape.bins];
    let mut gains = vec![0.0; shape.nb_erb];
    let mut coefs = vec![0.0; shape.nb_df * shape.order * 2];

    let size = chunks.size.max(1);
    let mut before: Option<Answer> = None;
    let mut lo = 0;
    while lo < spectra.frames {
        if stop.load(Ordering::Relaxed) {
            return Err(MlError::Cancelled);
        }
        let hi = (lo + size).min(spectra.frames);
        let first = lo.saturating_sub(chunks.warm);
        let end = (hi + chunks.fade).min(spectra.frames);
        spectra.keep_from(first.min(lo.saturating_sub(shape.behind)));
        spectra.fill_to(channels, end + shape.ahead.max(shape.order))?;
        let answer = graphs(&spectra, first, end)?;

        for t in lo..hi {
            gains.copy_from_slice(answer.gains(t, shape));
            coefs.copy_from_slice(answer.coefs(t, shape));
            // Within the handover the run before still has a say, fading
            // out as this one fades in, so no seam is ever a step.
            if let Some(old) = before.as_ref().filter(|_| t < lo + chunks.fade) {
                let mix = (t - lo + 1) as f32 / (chunks.fade + 1) as f32;
                blend(&mut gains, old.gains(t, shape), mix);
                blend(&mut coefs, old.coefs(t, shape), mix);
            }
            for (channel, (overlap, out)) in
                overlaps.iter_mut().zip(channels.iter_mut()).enumerate()
            {
                spectra.filter(channel, t, &gains, &coefs, &band_of, &mut frame);
                overlap.add(&mut frame, t, out)?;
            }
        }
        before = Some(answer);
        lo = hi;
    }
    Ok(())
}

/// `into` moved `mix` of the way from `old` to itself.
fn blend(into: &mut [f32], old: &[f32], mix: f32) {
    for (new, old) in into.iter_mut().zip(old) {
        *new = old + (*new - old) * mix;
    }
}

/// The channels' spectra and the model's features, frame by frame, for the
/// frames a run still needs. Frames are made in order and once each, because
/// the features carry a running average from one frame to the next.
pub(super) struct Spectra {
    pub shape: Shape,
    bands: Vec<usize>,
    /// Every frame the input makes, with the window's worth of silence after
    /// it that DeepFilterNet pads with.
    pub frames: usize,
    /// The first frame still held.
    first: usize,
    /// Each channel's frames, one after another.
    spectrum: Vec<Vec<Complex<f32>>>,
    erb: Vec<f32>,
    cplx: Vec<f32>,
    /// The running averages: band level in decibels, and bin magnitude.
    mean: Vec<f32>,
    unit: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    buffer: Vec<f32>,
    scratch: Vec<Complex<f32>>,
}

impl Spectra {
    /// Ready to make the frames of `count` channels of `len` samples.
    fn new(shape: Shape, bands: Vec<usize>, len: usize, count: usize) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(shape.fft);
        let spread = |count: usize, from: f32, to: f32| -> Vec<f32> {
            let step = (to - from) / (count - 1) as f32;
            (0..count).map(|i| from + i as f32 * step).collect()
        };
        Spectra {
            shape,
            bands,
            frames: (len + shape.fft) / shape.hop,
            first: 0,
            spectrum: vec![Vec::new(); count],
            erb: Vec::new(),
            cplx: Vec::new(),
            mean: spread(shape.nb_erb, -60.0, -90.0),
            unit: spread(shape.nb_df, 0.001, 0.0001),
            window: vorbis(shape.fft),
            buffer: fft.make_input_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
        }
    }

    /// One past the last frame held.
    fn held(&self) -> usize {
        self.first + self.erb.len() / self.shape.nb_erb
    }

    /// Forget every frame before `first`.
    fn keep_from(&mut self, first: usize) {
        let drop = first
            .saturating_sub(self.first)
            .min(self.held() - self.first);
        for spectrum in &mut self.spectrum {
            spectrum.drain(..drop * self.shape.bins);
        }
        self.erb.drain(..drop * self.shape.nb_erb);
        self.cplx.drain(..drop * self.shape.nb_df * 2);
        self.first += drop;
    }

    /// Make every frame of `channels` up to `end`, or to the last one there
    /// is.
    fn fill_to(&mut self, channels: &[Vec<f32>], end: usize) -> Result<(), MlError> {
        let shape = self.shape;
        let mut bins = vec![Complex::new(0.0, 0.0); shape.bins];
        let mut heard = vec![Complex::new(0.0, 0.0); shape.bins];
        let one = channels.len() == 1;
        let share = 1.0 / channels.len().max(1) as f32;
        let norm = 1.0 / ((shape.fft * shape.fft) as f32 / (2 * shape.hop) as f32);
        for t in self.held()..end.min(self.frames) {
            heard.fill(Complex::new(0.0, 0.0));
            for (channel, spectrum) in channels.iter().zip(&mut self.spectrum) {
                // Frame t is the window over the hop before t's and t's own.
                for (k, (slot, weight)) in self.buffer.iter_mut().zip(&self.window).enumerate() {
                    let sample = (t * shape.hop + k)
                        .checked_sub(shape.fft - shape.hop)
                        .and_then(|at| channel.get(at))
                        .copied()
                        .unwrap_or(0.0);
                    *slot = sample * weight;
                }
                self.fft
                    .process_with_scratch(&mut self.buffer, &mut bins, &mut self.scratch)
                    .map_err(|e| MlError::ModelFailed(e.to_string()))?;
                for bin in &mut bins {
                    *bin *= norm;
                }
                spectrum.extend_from_slice(&bins);
                // The transform is linear, so the mean of the channels'
                // spectra is the spectrum of their mean. One channel is its
                // own mean, number for number.
                if one {
                    heard.copy_from_slice(&bins);
                } else {
                    for (sum, x) in heard.iter_mut().zip(&bins) {
                        *sum += x * share;
                    }
                }
            }

            let (alpha, rest) = (shape.alpha, 1.0 - shape.alpha);
            let mut from = 0;
            for (width, mean) in self.bands.iter().zip(&mut self.mean) {
                let share = 1.0 / *width as f32;
                let power = heard[from..from + width]
                    .iter()
                    .fold(0.0, |sum, x| sum + (x.re * x.re + x.im * x.im) * share);
                let level = (power + 1e-10).log10() * 10.0;
                *mean = level * rest + *mean * alpha;
                self.erb.push((level - *mean) / 40.0);
                from += width;
            }
            for (x, unit) in heard.iter().zip(&mut self.unit) {
                // Floored at the smallest normal number. The bake flushes
                // denormals, so a long digital silence would take the average
                // to exactly nought and the division below to nought over
                // nought, and the model would hear that for minutes after.
                *unit = (x.norm() * rest + *unit * alpha).max(f32::MIN_POSITIVE);
                let scaled = x / unit.sqrt();
                self.cplx.extend_from_slice(&[scaled.re, scaled.im]);
            }
        }
        Ok(())
    }

    /// Frame `t`'s spectrum in `channel`, or `None` outside the frames held.
    fn spectrum(&self, channel: usize, t: usize) -> Option<&[Complex<f32>]> {
        let at = t.checked_sub(self.first)? * self.shape.bins;
        self.spectrum.get(channel)?.get(at..at + self.shape.bins)
    }

    /// Frame `t`'s band and bin features, or `None` outside the frames held.
    pub fn features(&self, t: usize) -> Option<(&[f32], &[f32])> {
        let row = t.checked_sub(self.first)?;
        let (e, c) = (self.shape.nb_erb, self.shape.nb_df * 2);
        Some((
            self.erb.get(row * e..(row + 1) * e)?,
            self.cplx.get(row * c..(row + 1) * c)?,
        ))
    }

    /// Frame `t` of `channel` with the model's answer applied: the filter
    /// across frames on the low bins, the band gains on the rest. The filter
    /// reads the input's spectrum, not the gained one, as DeepFilterNet does.
    fn filter(
        &self,
        channel: usize,
        t: usize,
        gains: &[f32],
        coefs: &[f32],
        band_of: &[usize],
        out: &mut [Complex<f32>],
    ) {
        let shape = self.shape;
        let Some(own) = self.spectrum(channel, t) else {
            out.fill(Complex::new(0.0, 0.0));
            return;
        };
        for ((slot, x), band) in out.iter_mut().zip(own).zip(band_of).skip(shape.nb_df) {
            *slot = x * gains[*band];
        }
        out[..shape.nb_df].fill(Complex::new(0.0, 0.0));
        for tap in 0..shape.order {
            let Some(near) = (t + tap)
                .checked_sub(shape.behind)
                .and_then(|at| self.spectrum(channel, at))
            else {
                continue;
            };
            for (f, (slot, x)) in out.iter_mut().zip(near).take(shape.nb_df).enumerate() {
                let at = (f * shape.order + tap) * 2;
                *slot += x * Complex::new(coefs[at], coefs[at + 1]);
            }
        }
    }
}

/// The inverse transform and the overlap-add, frame by frame in order.
struct Overlap {
    shape: Shape,
    window: Vec<f32>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    buffer: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    /// The second half of the last frame, waiting for the next one's first.
    carried: Vec<f32>,
}

impl Overlap {
    fn new(shape: Shape, window: Vec<f32>) -> Self {
        let ifft = RealFftPlanner::<f32>::new().plan_fft_inverse(shape.fft);
        Overlap {
            shape,
            window,
            buffer: ifft.make_output_vec(),
            scratch: ifft.make_scratch_vec(),
            ifft,
            carried: vec![0.0; shape.fft - shape.hop],
        }
    }

    /// Turn frame `t` back into sound and add it in. The transform's own
    /// delay of a window less a hop is taken off here, so `out` lines up
    /// with the input.
    fn add(
        &mut self,
        frame: &mut [Complex<f32>],
        t: usize,
        out: &mut [f32],
    ) -> Result<(), MlError> {
        let shape = self.shape;
        // A real signal's spectrum has no imaginary part at either end.
        for end in [0, shape.bins - 1] {
            frame[end].im = 0.0;
        }
        self.ifft
            .process_with_scratch(frame, &mut self.buffer, &mut self.scratch)
            .map_err(|e| MlError::ModelFailed(e.to_string()))?;
        for (sample, weight) in self.buffer.iter_mut().zip(&self.window) {
            *sample *= weight;
        }
        for (j, (sample, carried)) in self.buffer.iter().zip(&self.carried).enumerate() {
            if let Some(slot) = (t * shape.hop + j)
                .checked_sub(shape.fft - shape.hop)
                .and_then(|at| out.get_mut(at))
            {
                *slot = sample + carried;
            }
        }
        self.carried.copy_from_slice(&self.buffer[shape.hop..]);
        Ok(())
    }
}

/// The Vorbis window, `sin(pi/2 sin^2(pi (n + 1/2) / N))`, whose square sums
/// to one at half overlap, so the same window analyses and resynthesises.
fn vorbis(size: usize) -> Vec<f32> {
    let half = (size / 2) as f64;
    (0..size)
        .map(|i| {
            let inner = (0.5 * std::f64::consts::PI * (i as f64 + 0.5) / half).sin();
            (0.5 * std::f64::consts::PI * inner * inner).sin() as f32
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The test signal libDF's numbers below were taken from: three sines,
    /// half a second of them.
    fn three_sines() -> Vec<f32> {
        (0..24_000)
            .map(|i| {
                let i = f64::from(i);
                (0.3 * (0.01 * i).sin() + 0.1 * (0.37 * i).sin() + 0.05 * (1.9 * i).sin()) as f32
            })
            .collect()
    }

    /// **The graphs hear what libDF would feed them, and an answer that
    /// changes nothing gives the input back sample for sample.** Everything
    /// around the model, checked without one: the band layout, the spectrum,
    /// both running feature averages and their alignment, against numbers
    /// libDF 0.5.6 printed for the same signal; then the filter and the
    /// overlap-add, on every channel, with unit gains and a filter that only
    /// passes the frame it makes.
    #[test]
    fn the_signal_path_matches_libdf_and_reconstructs_its_input() {
        let names = DeepFilter::default();
        let shape = Shape::of(&names);
        let bands = runnable(&names).expect("the default pack runs");
        assert_eq!(
            bands,
            [
                2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 5, 5, 7, 7, 8, 10, 12, 13, 15, 18, 20, 24,
                28, 31, 37, 42, 50, 56, 67
            ],
            "libDF's band widths"
        );

        // Frame, band features at bands 0, 13 and 31, bin features at bins
        // 1, 40 and 95, and the spectrum at bins 2, 100 and 400.
        type Pair = (f64, f64);
        type Row = (usize, [f64; 3], [Pair; 3], [Pair; 3]);
        #[rustfmt::skip]
        let libdf: [Row; 5] = [
            (0, [0.846019, 0.049685, 0.098438],
                [(-0.804266, 1.022484), (0.021427, 0.000079), (0.014399, -0.000007)],
                [(-1.3143583e-2, -3.7569311e-2), (-1.2100926e-4, -3.9788819e-8), (-5.6717105e-5, -1.9751800e-9)]),
            (1, [0.822849, -0.678962, -0.248484],
                [(1.440347, 0.145384), (-0.000107, -0.000001), (0.000021, 0.000006)],
                [(-7.6646365e-2, -6.7092869e-3), (1.4497290e-7, 4.2645034e-8), (4.0374593e-9, -1.0908935e-9)]),
            (7, [0.785487, -0.636423, -0.233943],
                [(-0.689042, -0.569559), (0.000091, 0.000041), (-0.000014, -0.000022)],
                [(6.2865943e-2, 4.3401077e-2), (-8.6407582e-8, -1.5421777e-7), (3.5284455e-9, -3.5537902e-9)]),
            (30, [0.636267, -0.495922, -0.185660],
                [(0.232002, 0.483750), (0.000022, 0.000174), (-0.000003, -0.000017)],
                [(-3.7690710e-2, -6.5596193e-2), (-1.7432301e-8, -9.2859551e-8), (-1.6906749e-9, 3.7452086e-9)]),
            (51, [-1.328650, -0.424265, -0.157210],
                [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
                [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]),
        ];
        let input = three_sines();
        let channels = [input.clone()];
        let mut spectra = Spectra::new(shape, bands.clone(), input.len(), 1);
        assert_eq!(spectra.frames, 52, "libDF's frame count");
        spectra.fill_to(&channels, spectra.frames).unwrap();
        for (t, erb, cplx, spec) in libdf {
            let (bands_at, bins_at) = spectra.features(t).unwrap();
            for (b, want) in [0, 13, 31].into_iter().zip(erb) {
                assert!(
                    (f64::from(bands_at[b]) - want).abs() < 1e-4,
                    "band {b} at {t}: {}",
                    bands_at[b]
                );
            }
            for (f, want) in [1, 40, 95].into_iter().zip(cplx) {
                let got = (f64::from(bins_at[f * 2]), f64::from(bins_at[f * 2 + 1]));
                assert!(
                    (got.0 - want.0).abs() < 1e-4 && (got.1 - want.1).abs() < 1e-4,
                    "bin {f} at {t}: {got:?}"
                );
            }
            let own = spectra.spectrum(0, t).unwrap();
            for (f, want) in [2, 100, 400].into_iter().zip(spec) {
                let got = own[f];
                assert!(
                    (f64::from(got.re) - want.0).abs() < 1e-6
                        && (f64::from(got.im) - want.1).abs() < 1e-6,
                    "spectrum {f} at {t}: {got:?}"
                );
            }
        }

        // An answer that changes nothing: every band at one, and the filter
        // passing only the frame it makes.
        let mut unity = |spectra: &Spectra, first: usize, end: usize| {
            let shape = spectra.shape;
            let n = end - first;
            let mut coefs = vec![0.0; n * shape.nb_df * shape.order * 2];
            for frame in coefs.chunks_exact_mut(shape.nb_df * shape.order * 2) {
                for f in 0..shape.nb_df {
                    frame[(f * shape.order + shape.behind) * 2] = 1.0;
                }
            }
            Ok(Answer {
                first,
                gains: vec![1.0; n * shape.nb_erb],
                coefs,
            })
        };
        let other: Vec<f32> = input.iter().rev().map(|x| x * 0.5).collect();
        let never = AtomicBool::new(false);
        // Short runs, so the handover between them is in the signal too.
        let short = Chunks {
            size: 7,
            warm: 3,
            fade: 2,
        };
        let mut outs = [input.clone(), other.clone()];
        run(shape, &bands, &mut outs, &never, short, &mut unity).unwrap();
        for (out, want) in outs.iter().zip([&input, &other]) {
            assert_eq!(out.len(), want.len());
            let worst = out
                .iter()
                .zip(want.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-5, "came back {worst} away from its input");
        }

        let stopped = AtomicBool::new(true);
        let mut once = [input.clone()];
        let refusal = run(shape, &bands, &mut once, &stopped, CHUNKS, &mut unity).unwrap_err();
        assert_eq!(refusal, MlError::Cancelled);
    }

    /// Denormals flushed for as long as the guard lives, as the audio chain
    /// runs every effect.
    struct Flushed(#[allow(dead_code)] u32);

    impl Flushed {
        #[allow(unsafe_code)]
        fn on() -> Self {
            #[cfg(target_arch = "x86_64")]
            let csr = {
                let mut csr: u32 = 0;
                // SAFETY: `stmxcsr` writes four bytes to a live `u32`, and
                // `ldmxcsr` reads four from one. Flush to zero is bit 15 and
                // denormals are zero bit 6.
                unsafe { std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack)) };
                let raised = csr | 0x8000 | 0x0040;
                unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &raised, options(nostack)) };
                csr
            };
            #[cfg(not(target_arch = "x86_64"))]
            let csr = 0;
            Flushed(csr)
        }
    }

    impl Drop for Flushed {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: as `Flushed::on`.
            unsafe {
                std::arch::asm!("ldmxcsr [{}]", in(reg) &self.0, options(nostack));
            }
        }
    }

    /// **Two minutes of digital silence, then sound, with denormals flushed:
    /// what the graphs hear stays finite and the sound after comes through.**
    /// The running bin average used to decay to exactly nought in about 78 s
    /// and divide nought by it, so a muted microphone turned the voice to
    /// nothing for minutes after it came back. The answer here is built from
    /// the features, as a model's is, so anything they carry reaches the
    /// sound.
    #[test]
    fn long_silence_leaves_the_features_finite_and_the_sound_after_intact() {
        let names = DeepFilter::default();
        let shape = Shape::of(&names);
        let bands = runnable(&names).unwrap();
        let silence = 120 * RATE as usize;
        let mut signal = vec![0.0f32; silence];
        signal.extend(three_sines());
        let mut outs = [signal.clone()];
        let mut heard = |spectra: &Spectra, first: usize, end: usize| {
            let shape = spectra.shape;
            let (mut gains, mut coefs) = (Vec::new(), Vec::new());
            for t in first..end {
                let (erb, cplx) = spectra.features(t + shape.ahead).unwrap_or((&[], &[]));
                let finite = erb.iter().chain(cplx).all(|x| x.is_finite());
                let nudge = if finite { 0.0 } else { f32::NAN };
                gains.extend(std::iter::repeat_n(1.0 + nudge, shape.nb_erb));
                let mut taps = vec![nudge; shape.nb_df * shape.order * 2];
                for f in 0..shape.nb_df {
                    taps[(f * shape.order + shape.behind) * 2] = 1.0;
                }
                coefs.extend(taps);
            }
            Ok(Answer {
                first,
                gains,
                coefs,
            })
        };
        let flushed = Flushed::on();
        run(
            shape,
            &bands,
            &mut outs,
            &AtomicBool::new(false),
            CHUNKS,
            &mut heard,
        )
        .unwrap();
        drop(flushed);

        let out = &outs[0];
        assert!(out.iter().all(|x| x.is_finite()), "the output went NaN");
        let worst = out[silence..]
            .iter()
            .zip(&signal[silence..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "the sound after came back {worst} away");
    }
}
