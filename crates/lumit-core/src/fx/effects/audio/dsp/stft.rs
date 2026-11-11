//! Short-time Fourier analysis and overlap-add synthesis, streamed a block at
//! a time or run over a whole span.
//!
//! # In plain terms
//!
//! A spectral effect cuts the sound into overlapping frames, windows each one,
//! turns it into a spectrum, changes the spectrum, turns it back and adds the
//! frames up again. Done right, a spectrum left alone comes back as the very
//! sound that went in. Three pieces here make that so.
//!
//! [`Fft`] is one real transform of a fixed size, planned once with its
//! scratch, so a frame costs no allocation. [`Framer`] is the streaming half:
//! it takes interleaved blocks, hands the effect each windowed frame as it
//! completes, and adds what comes back into the output, which runs exactly
//! `n - 1` frames late. [`frame_at`] and [`overlap_add`] are the same
//! framing over a whole span, for the analysis an effect does in its look.
//! Frame `j` covers the same samples in both, so a statistic gathered over the
//! span lines up with the frame the stream is about to process.
//!
//! The window is the square root of a periodic Hann on both sides. Squared it
//! is a Hann, and Hann frames a quarter apart add up to exactly two, so the
//! synthesis window carries a half and an untouched spectrum reconstructs at
//! unity.
//!
//! Sizes scale with the rate, so a frame is the same length in seconds at
//! 44.1, 48 and 96 kHz and an effect sounds the same at each
//! ([`size_at`]).

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// The rate frame sizes are quoted at.
pub const QUOTED_RATE: f64 = 48_000.0;

/// A frame size of `at_48k` samples at 48 kHz, scaled to `rate`.
///
/// The answer is the nearest whole number to the scaled size that divides by
/// four, so a 75% overlap has a whole hop, and has no prime factor above five,
/// so the transform stays fast: 2048 at 48 kHz is 1920 at 44.1 kHz and 4096 at
/// 96 kHz.
#[must_use]
pub fn size_at(rate: u32, at_48k: usize) -> usize {
    let want = (at_48k as f64 * f64::from(rate.max(1)) / QUOTED_RATE).max(4.0);
    let smooth = |n: usize| {
        let mut m = n;
        for p in [2, 3, 5] {
            while m.is_multiple_of(p) {
                m /= p;
            }
        }
        m == 1
    };
    // Walk out from the wanted size in steps of four until a smooth size
    // turns up on either side; one always does within a few per cent.
    let base = ((want / 4.0).round() as usize).max(1) * 4;
    for step in 0..base {
        let up = base + step * 4;
        let down = base.saturating_sub(step * 4);
        let (near, far) = if (up as f64 - want).abs() <= (want - down as f64).abs() {
            (up, down)
        } else {
            (down, up)
        };
        for n in [near, far] {
            if n >= 4 && smooth(n) {
                return n;
            }
        }
    }
    base
}

/// The square root of a periodic Hann window of `n` points.
#[must_use]
pub fn sqrt_hann(n: usize) -> Vec<f32> {
    hann(n).into_iter().map(f32::sqrt).collect()
}

/// A periodic Hann window of `n` points: the one whose copies a quarter or a
/// half apart add up to a constant.
#[must_use]
pub fn hann(n: usize) -> Vec<f32> {
    let step = std::f64::consts::TAU / n.max(1) as f64;
    (0..n)
        .map(|i| (0.5 - 0.5 * (step * i as f64).cos()) as f32)
        .collect()
}

/// What the synthesis window must be scaled by for frames `hop` apart to
/// rebuild the signal at unity, when the analysis and synthesis windows
/// multiply to a periodic Hann: Hann copies `hop` apart add up to
/// `n / (2 hop)`.
#[must_use]
pub fn hann_overlap_scale(n: usize, hop: usize) -> f32 {
    (2.0 * hop as f64 / n.max(1) as f64) as f32
}

/// One real transform of a fixed size, forwards and back, with its scratch
/// held so a frame allocates nothing.
pub struct Fft {
    n: usize,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl Fft {
    /// Plan a transform of `n` real samples.
    #[must_use]
    pub fn new(n: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(n.max(2));
        let inverse = planner.plan_fft_inverse(n.max(2));
        let scratch_len = forward.get_scratch_len().max(inverse.get_scratch_len());
        Self {
            n: n.max(2),
            forward,
            inverse,
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
        }
    }

    /// Samples in a frame.
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Never: a transform has at least two samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Bins in a spectrum: `n / 2 + 1`.
    #[must_use]
    pub fn bins(&self) -> usize {
        self.n / 2 + 1
    }

    /// A spectrum's worth of zeros, for the caller to hold.
    #[must_use]
    pub fn spectrum(&self) -> Vec<Complex<f32>> {
        vec![Complex::new(0.0, 0.0); self.bins()]
    }

    /// `frame` (already windowed, `n` long) into `spectrum` (`bins` long).
    /// `frame` is used as scratch and holds nothing useful afterwards. A
    /// length that does not fit leaves the spectrum silent rather than
    /// failing.
    pub fn forward(&mut self, frame: &mut [f32], spectrum: &mut [Complex<f32>]) {
        if self
            .forward
            .process_with_scratch(frame, spectrum, &mut self.scratch)
            .is_err()
        {
            spectrum.fill(Complex::new(0.0, 0.0));
        }
    }

    /// `spectrum` back into `frame`, scaled by `1 / n` so a forward and an
    /// inverse are the identity. `spectrum` is used as scratch.
    pub fn inverse(&mut self, spectrum: &mut [Complex<f32>], frame: &mut [f32]) {
        // A real signal's spectrum has no imaginary part at nought or at the
        // top bin, and a gain applied to a complex number can leave a rounding
        // there; the transform would say so and carry on.
        if let Some(first) = spectrum.first_mut() {
            first.im = 0.0;
        }
        if let Some(last) = spectrum.last_mut() {
            last.im = 0.0;
        }
        let _ = self
            .inverse
            .process_with_scratch(spectrum, frame, &mut self.scratch);
        let scale = 1.0 / self.n as f32;
        for s in frame.iter_mut() {
            *s *= scale;
        }
    }
}

/// The streaming half: interleaved blocks in, one windowed frame per hop
/// handed to the effect, the frames that come back added up into the output.
///
/// The output runs exactly [`Framer::latency`] frames behind the input. Frame
/// `j` covers input samples `j hop + hop - n` up to `j hop + hop`, the ones
/// before the first sample being silence, which is also what [`frame_at`]
/// reads, so a frame here and the same-numbered frame of a whole-span pass
/// agree.
pub struct Framer {
    n: usize,
    hop: usize,
    channels: usize,
    analysis: Vec<f32>,
    synthesis: Vec<f32>,
    fifo_in: Vec<Vec<f32>>,
    fifo_out: Vec<Vec<f32>>,
    sum: Vec<Vec<f32>>,
    work: Vec<Vec<f32>>,
    rover: usize,
    frames: u64,
}

impl Framer {
    /// A framer of `n`-sample frames `hop` apart over `channels` interleaved
    /// channels. `analysis` windows each frame on the way to the effect and
    /// `synthesis` on the way back; either may be shorter than `n`, which
    /// counts as ones past its end, though every caller here passes `n`.
    #[must_use]
    pub fn new(
        n: usize,
        hop: usize,
        channels: usize,
        analysis: Vec<f32>,
        synthesis: Vec<f32>,
    ) -> Self {
        let n = n.max(1);
        let hop = hop.clamp(1, n);
        let channels = channels.max(1);
        let pad = |mut w: Vec<f32>| {
            w.resize(n, 1.0);
            w
        };
        Self {
            n,
            hop,
            channels,
            analysis: pad(analysis),
            synthesis: pad(synthesis),
            fifo_in: vec![vec![0.0; n]; channels],
            fifo_out: vec![vec![0.0; hop]; channels],
            sum: vec![vec![0.0; n]; channels],
            work: vec![vec![0.0; n]; channels],
            rover: n - hop,
            frames: 0,
        }
    }

    /// A framer of `n`-sample frames a quarter apart with the square-root
    /// Hann both sides, scaled to rebuild at unity.
    #[must_use]
    pub fn sqrt_hann(n: usize, channels: usize) -> Self {
        let hop = (n / 4).max(1);
        let window = sqrt_hann(n);
        let scale = hann_overlap_scale(n, hop);
        let synthesis = window.iter().map(|w| w * scale).collect();
        Self::new(n, hop, channels, window, synthesis)
    }

    /// Frames of delay between a sample going in and coming out: a frame
    /// less one, since a sample at the start of a frame is only finished when
    /// that frame is.
    #[must_use]
    pub fn latency(&self) -> usize {
        self.n - 1
    }

    /// Samples between one frame and the next.
    #[must_use]
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// How many frames have been handed out so far, which is also the index
    /// of the next one.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// One run of interleaved samples through. `frame` is called once per
    /// completed frame with every channel's windowed frame, and changes them
    /// in place into what should be added back; it is told the frame's index.
    /// `output` must be as long as `input`.
    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        mut frame: impl FnMut(u64, &mut [Vec<f32>]),
    ) {
        let reset = self.n - self.hop;
        let channels = self.channels;
        for (came, goes) in input
            .chunks_exact(channels)
            .zip(output.chunks_exact_mut(channels))
        {
            for (c, sample) in came.iter().enumerate() {
                if let Some(at) = self.fifo_in.get_mut(c).and_then(|f| f.get_mut(self.rover)) {
                    *at = *sample;
                }
            }
            self.rover += 1;
            if self.rover >= self.n {
                self.rover = reset;
                self.turn(&mut frame);
            }
            // The oldest hop of the last frame is complete the moment that
            // frame is, so it starts coming out straight away.
            let read = self.rover - reset;
            for (c, slot) in goes.iter_mut().enumerate() {
                *slot = self
                    .fifo_out
                    .get(c)
                    .and_then(|fifo| fifo.get(read))
                    .copied()
                    .unwrap_or(0.0);
            }
        }
    }

    /// One whole frame: window, hand over, window again, add up, move on.
    fn turn(&mut self, frame: &mut impl FnMut(u64, &mut [Vec<f32>])) {
        let hop = self.hop;
        for (work, fifo) in self.work.iter_mut().zip(&self.fifo_in) {
            for ((w, x), a) in work.iter_mut().zip(fifo).zip(&self.analysis) {
                *w = x * a;
            }
        }
        frame(self.frames, &mut self.work);
        self.frames += 1;
        for (((work, sum), out), fifo) in self
            .work
            .iter()
            .zip(self.sum.iter_mut())
            .zip(self.fifo_out.iter_mut())
            .zip(self.fifo_in.iter_mut())
        {
            for ((s, w), y) in sum.iter_mut().zip(work).zip(&self.synthesis) {
                *s += w * y;
            }
            out.copy_from_slice(sum.get(..hop).unwrap_or(&[]));
            sum.copy_within(hop.., 0);
            let n = sum.len();
            if let Some(tail) = sum.get_mut(n - hop..) {
                tail.fill(0.0);
            }
            fifo.copy_within(hop.., 0);
        }
    }
}

/// Frames a whole span of `frames_long` samples is cut into by a framer of
/// size `n` and hop `hop`, counting every frame that touches a sample.
#[must_use]
pub fn frame_count(frames_long: usize, n: usize, hop: usize) -> usize {
    if frames_long == 0 {
        return 0;
    }
    (frames_long - 1 + n.saturating_sub(hop)) / hop.max(1) + 1
}

/// Frame `j` of channel `channel` of an interleaved span, windowed by
/// `window`, into `out` (`window.len()` long). The samples before the span's
/// start and past its end are silence. Lines up with [`Framer`]'s frame `j`.
pub fn frame_at(
    interleaved: &[f32],
    channels: usize,
    channel: usize,
    j: usize,
    hop: usize,
    window: &[f32],
    out: &mut [f32],
) {
    let n = window.len();
    let channels = channels.max(1);
    let len = interleaved.len() / channels;
    // Where the frame starts, which may be before the span does.
    let start = (j * hop + hop) as i64 - n as i64;
    for (i, (slot, w)) in out.iter_mut().zip(window).enumerate() {
        let t = start + i as i64;
        *slot = if t >= 0 && (t as usize) < len {
            interleaved
                .get(t as usize * channels + channel)
                .map_or(0.0, |x| x * w)
        } else {
            0.0
        };
    }
}

/// The mono mix of frame `j` of an interleaved span, windowed, into `out`.
pub fn mono_frame_at(
    interleaved: &[f32],
    channels: usize,
    j: usize,
    hop: usize,
    window: &[f32],
    out: &mut [f32],
) {
    let n = window.len();
    let channels = channels.max(1);
    let len = interleaved.len() / channels;
    let start = (j * hop + hop) as i64 - n as i64;
    let scale = 1.0 / channels as f32;
    for (i, (slot, w)) in out.iter_mut().zip(window).enumerate() {
        let t = start + i as i64;
        *slot = if t >= 0 && (t as usize) < len {
            let at = t as usize * channels;
            interleaved
                .get(at..at + channels)
                .map_or(0.0, |f| f.iter().sum::<f32>() * scale * w)
        } else {
            0.0
        };
    }
}

/// Add synthesised frame `j` (already windowed for synthesis) into a mono
/// span, the part that falls outside it dropped. The inverse of
/// [`frame_at`]: every frame of a span added back this way rebuilds it.
pub fn overlap_add(span: &mut [f32], j: usize, hop: usize, frame: &[f32]) {
    let start = (j * hop + hop) as i64 - frame.len() as i64;
    for (i, x) in frame.iter().enumerate() {
        let t = start + i as i64;
        if t >= 0 {
            if let Some(slot) = span.get_mut(t as usize) {
                *slot += x;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **Untouched is unity**, streamed and over a whole span, at every rate a
    /// bake runs at: a spectral effect at nought must be the sound it was
    /// given, only later.
    #[test]
    fn an_untouched_spectrum_rebuilds_the_sound() {
        for rate in [44_100, 48_000, 96_000] {
            let n = size_at(rate, 2048);
            let len = 20_000;
            let input: Vec<f32> = (0..len * 2)
                .map(|i| ((i as f32 * 0.37).sin() + (i as f32 * 0.011).cos()) * 0.4)
                .collect();
            let mut framer = Framer::sqrt_hann(n, 2);
            let mut fft = Fft::new(n);
            let mut spectrum = fft.spectrum();
            let mut out = vec![0.0; input.len()];
            framer.process(&input, &mut out, |_, frames| {
                for f in frames.iter_mut() {
                    fft.forward(f, &mut spectrum);
                    fft.inverse(&mut spectrum, f);
                }
            });
            let lat = framer.latency();
            for t in 0..len - lat {
                for c in 0..2 {
                    let want = input[t * 2 + c];
                    let got = out[(t + lat) * 2 + c];
                    assert!(
                        (want - got).abs() < 1e-5,
                        "{rate}: frame {t} came back {got}, not {want}"
                    );
                }
            }

            // The same framing over the whole span, one channel.
            let hop = n / 4;
            let window = sqrt_hann(n);
            let scale = hann_overlap_scale(n, hop);
            let mut span = vec![0.0; len];
            let mut frame = vec![0.0; n];
            for j in 0..frame_count(len, n, hop) {
                frame_at(&input, 2, 1, j, hop, &window, &mut frame);
                for (x, w) in frame.iter_mut().zip(&window) {
                    *x *= w * scale;
                }
                overlap_add(&mut span, j, hop, &frame);
            }
            for t in 0..len {
                assert!(
                    (span[t] - input[t * 2 + 1]).abs() < 1e-5,
                    "{rate}: span sample {t}"
                );
            }
        }
    }
}
