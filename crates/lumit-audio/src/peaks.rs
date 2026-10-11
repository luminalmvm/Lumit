//! Waveform peaks at every zoom, and the per-band "multiwave" stack
//! (docs/09-AUDIO.md §4).
//!
//! In plain terms: a timeline waveform is not the sound, it is a *summary* of
//! the sound — for each column of pixels, how far the speaker cone swung up
//! and down while that column's slice of time went by. The summary has to be
//! recomputed whenever the zoom changes, because a column that covered a whole
//! second when the comp was fitted covers a millisecond once you are cutting
//! on a hi-hat. Recomputing it from the raw samples every time would mean
//! reading millions of numbers per repaint, so this module does the reading
//! **once** and keeps the answer at three levels of detail — 256 samples per
//! block, 4 096, and 65 536. That is a mip-map, exactly like the ones a GPU
//! keeps for a texture: draw from the level nearest the size you are drawing
//! at, and the work per column stays tiny at every zoom.
//!
//! The second thing here is the **multiwave**. One waveform tells you how
//! *loud* a moment is and nothing about what is in it — a loud master is a
//! solid block ("a sausage") whether it is a kick, a snare or a vocal. So
//! alongside the plain wave this module also splits the sound into three
//! frequency bands (bass, middle, treble) with ordinary filters and summarises
//! each of them the same way. Stacked, the three read as a picture of *what*
//! is happening: the kick shows in the bottom band, the hats in the top, and a
//! cut can be aimed at either.
//!
//! The summarising is pure arithmetic over samples, so all of it is a plain
//! deterministic test. It takes them a stretch at a time ([`PyramidBuilder`]),
//! so a file is summarised as it is decoded and never held whole, and the
//! summary is written to a small file beside the frame index
//! ([`load_or_build`]) so the next run of Lumit reads it back instead of decoding
//! the file again.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The bands a multiwave stack draws, plus the plain full-range wave that a
/// single-wave lane draws. Stored side by side in one pyramid because they
/// come from one pass over the samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// The whole signal — what the single-wave lane draws.
    Full,
    /// Below `LOW_CROSSOVER_HZ`: kicks, bass, room rumble.
    Low,
    /// Between the two crossovers: most of a voice, most of a snare's body.
    Mid,
    /// Above `HIGH_CROSSOVER_HZ`: hats, sibilance, transient edge.
    High,
}

/// How many summaries one pyramid holds per block — the four of [`Band`].
const BAND_COUNT: usize = 4;

impl Band {
    /// Where this band's summaries sit inside a tier's band-major array.
    const fn index(self) -> usize {
        match self {
            Band::Full => 0,
            Band::Low => 1,
            Band::Mid => 2,
            Band::High => 3,
        }
    }

    /// The stack a multiwave lane draws, bottom band first — the order a
    /// spectrum is read in, so the picture matches the mental model.
    #[must_use]
    pub const fn stack() -> [Band; 3] {
        [Band::Low, Band::Mid, Band::High]
    }
}

/// Bass/middle crossover, in Hz. Low enough that a kick and a bass line land
/// under it and a voice's fundamental mostly does not.
const LOW_CROSSOVER_HZ: f32 = 200.0;

/// Middle/treble crossover, in Hz. Above it lives the transient edge — hats,
/// sibilance, the click of a kick — which is what an edit is usually aimed at.
const HIGH_CROSSOVER_HZ: f32 = 2_000.0;

/// The finest tier's block size, in samples: ~5 ms at 48 kHz, finer than any
/// single pixel column an editor can zoom a waveform to.
const FINEST_BLOCK: usize = 256;

/// How much coarser each tier is than the one below it.
const TIER_RATIO: usize = 16;

/// How many tiers a pyramid holds: 256 / 4 096 / 65 536 samples per block,
/// the three sizes docs/09 §4 names.
const TIERS: usize = 3;

/// How many of a tier's blocks must fit inside one bucket before that tier is
/// coarse enough to read it from. A bucket covers whole blocks, so it always
/// reaches a little past its own edges; asking for four blocks keeps that
/// overspill under a quarter of a bucket, which is below what an eye can see
/// on a lane, while still costing only a handful of merges per column.
const BLOCKS_PER_BUCKET: usize = 4;

/// How long a source may be and still have its samples kept beside the pyramid
/// (see [`PeakPyramid::samples`]). Past this the finest tier can never be
/// out-resolved anyway: the Timeline zooms to 64×, so a lane a couple of
/// thousand pixels wide bottoms out at roughly `duration / 128 000` seconds per
/// column, which only drops under one [`FINEST_BLOCK`] for sources shorter than
/// about ten minutes. Longer than that, keeping a sample copy would cost tens
/// of megabytes to answer a question nobody can ask.
const SAMPLE_KEEP_SECONDS: f64 = 600.0;

/// How much of the signal to run the band filters over *before* the window
/// being drawn, when a query is answered from the samples. A filter starts from
/// rest and takes a few hundred samples to settle; starting this far back means
/// the values inside the window are the ones the filter would have produced had
/// it been running from the beginning. 85 ms at 48 kHz — inaudible as a cost,
/// far more than the filters need.
const SAMPLE_PREROLL: usize = 4096;

/// The most blocks the finest tier may hold, so one pyramid's memory is
/// bounded however long the file is (docs/14 §5: budgeted allocations). At the
/// cap a pyramid costs about 12 MB; past it the finest tier is coarsened by
/// [`TIER_RATIO`] until it fits, which costs resolution only on files hours
/// long.
const MAX_BLOCKS: usize = 262_144;

/// One block's summary: how far the signal swung either way across it, and how
/// much energy it carried. `min`/`max` draw the body of the wave and `rms`
/// draws the solid core inside it (docs/15 §waveforms).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PeakBlock {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

impl PeakBlock {
    /// Silence — and what an empty query answers, so a caller never has to
    /// special-case a bucket that fell off the end of the audio.
    pub const SILENT: PeakBlock = PeakBlock {
        min: 0.0,
        max: 0.0,
        rms: 0.0,
    };

    /// The summary covering both of two neighbouring blocks. Extremes take the
    /// wider pair; the energy is the root of the mean of the two mean squares,
    /// which is exact when the two blocks are the same length — and inside a
    /// tier they always are, bar the last.
    #[must_use]
    fn merged(self, other: PeakBlock) -> PeakBlock {
        PeakBlock {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            rms: (0.5 * (self.rms * self.rms + other.rms * other.rms)).sqrt(),
        }
    }
}

/// A sample as stored beside the pyramid, and back again. Full scale is
/// `i16::MAX`; anything hotter is clamped, which is what a picture of a
/// clipping signal should show anyway.
fn to_i16(x: f32) -> i16 {
    (x * 32_767.0).clamp(-32_768.0, 32_767.0) as i16
}

fn from_i16(x: i16) -> f32 {
    f32::from(x) / 32_767.0
}

/// A running summary, kept while a block is being filled.
#[derive(Clone, Copy)]
struct Running {
    min: f32,
    max: f32,
    sum_sq: f64,
    count: usize,
}

/// A summary of several whole blocks, for folding them into one.
#[derive(Clone, Copy)]
struct Folded {
    min: f32,
    max: f32,
    sum_sq: f64,
}

impl Running {
    const EMPTY: Running = Running {
        min: f32::MAX,
        max: f32::MIN,
        sum_sq: 0.0,
        count: 0,
    };

    fn push(&mut self, x: f32) {
        self.min = self.min.min(x);
        self.max = self.max.max(x);
        self.sum_sq += f64::from(x) * f64::from(x);
        self.count += 1;
    }

    fn finish(self) -> PeakBlock {
        if self.count == 0 {
            return PeakBlock::SILENT;
        }
        PeakBlock {
            min: self.min,
            max: self.max,
            rms: (self.sum_sq / self.count as f64).sqrt() as f32,
        }
    }
}

/// One second-order section, transposed direct form II — the standard "cookbook"
/// biquad. Two of these in series make the 24 dB/octave slope the band split
/// uses, which is steep enough that a kick does not smear into the middle band.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// A Butterworth-Q section at `cutoff`, low-pass when `low` and high-pass
    /// otherwise. A cutoff at or above Nyquist (or at or below zero) yields a
    /// pass-through rather than a divide by zero: a 4 kHz file has no treble
    /// band to speak of, and answering "all of it" beats answering NaN.
    fn section(sample_rate: f32, cutoff: f32, low: bool) -> Biquad {
        let pass = Biquad {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        };
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || cutoff <= 0.0
            || cutoff >= sample_rate * 0.5
        {
            return pass;
        }
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let w0 = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        if a0 == 0.0 {
            return pass;
        }
        let (b0, b1, b2) = if low {
            let b1 = 1.0 - cos_w0;
            (b1 * 0.5, b1, b1 * 0.5)
        } else {
            let b1 = 1.0 + cos_w0;
            (b1 * 0.5, -b1, b1 * 0.5)
        };
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// A cascade of two sections: the 24 dB/octave slope each band edge uses.
#[derive(Clone, Copy)]
struct Slope([Biquad; 2]);

impl Slope {
    fn low(sample_rate: f32, cutoff: f32) -> Slope {
        Slope([
            Biquad::section(sample_rate, cutoff, true),
            Biquad::section(sample_rate, cutoff, true),
        ])
    }

    fn high(sample_rate: f32, cutoff: f32) -> Slope {
        Slope([
            Biquad::section(sample_rate, cutoff, false),
            Biquad::section(sample_rate, cutoff, false),
        ])
    }

    fn run(&mut self, x: f32) -> f32 {
        let mut y = x;
        for section in &mut self.0 {
            y = section.run(y);
        }
        y
    }
}

/// The three-way split a multiwave stack is made of.
struct Split {
    low: Slope,
    mid_high: Slope,
    mid_low: Slope,
    high: Slope,
}

impl Split {
    fn new(sample_rate: f32) -> Split {
        Split {
            low: Slope::low(sample_rate, LOW_CROSSOVER_HZ),
            // The middle band is what survives both edges.
            mid_high: Slope::high(sample_rate, LOW_CROSSOVER_HZ),
            mid_low: Slope::low(sample_rate, HIGH_CROSSOVER_HZ),
            high: Slope::high(sample_rate, HIGH_CROSSOVER_HZ),
        }
    }

    /// One mono sample in, the four band values out, in [`Band::index`] order.
    fn run(&mut self, x: f32) -> [f32; BAND_COUNT] {
        [
            x,
            self.low.run(x),
            self.mid_low.run(self.mid_high.run(x)),
            self.high.run(x),
        ]
    }
}

/// One level of detail: `len` blocks of `block` samples each, per band.
struct Tier {
    /// Samples per block.
    block: usize,
    /// Blocks per band.
    len: usize,
    /// Band-major: band `b`'s block `i` is at `b * len + i`.
    data: Vec<PeakBlock>,
}

impl Tier {
    fn at(&self, band: Band, index: usize) -> PeakBlock {
        self.data
            .get(band.index() * self.len + index)
            .copied()
            .unwrap_or(PeakBlock::SILENT)
    }
}

/// One source's waveform summarised at three levels of detail, for all four
/// bands — everything a lane needs to draw itself at any zoom without going
/// near the samples again.
///
/// Built once per source (the bridge keeps a small cache of them), then asked
/// for whatever window the lane is currently showing.
pub struct PeakPyramid {
    sample_rate: u32,
    frames: usize,
    /// Finest first.
    tiers: Vec<Tier>,
    /// The mono mixdown itself, as 16-bit samples — what a query zoomed in
    /// past the finest tier is answered from, so a fully zoomed lane draws the
    /// signal rather than a staircase of identical blocks.
    ///
    /// A pyramid summarises; at some zoom the summary runs out, and past that
    /// point the only honest answer is the samples. 16-bit rather than float
    /// because this is a picture: half the memory, and the difference is three
    /// ten-thousandths of a pixel on any lane ever drawn. Empty for a source
    /// longer than [`SAMPLE_KEEP_SECONDS`], where the summary never runs out.
    ///
    /// Set when the pyramid is built. One read back from its file has none
    /// yet: the file holds the summary, which is a tenth of the size, and the
    /// samples are decoded from [`Self::source`] the first time a lane zooms
    /// in far enough to ask for them.
    samples: OnceLock<Vec<i16>>,
    /// Whether a build of this source keeps its samples: it is short enough.
    keeps_samples: bool,
    /// The file the samples can be decoded from again.
    source: Option<PathBuf>,
}

/// A source being summarised a stretch at a time.
///
/// Hand it the sound in order, in pieces of any size, and it comes to the
/// same pyramid whatever the pieces were: nothing here looks at where one
/// piece ends and the next begins.
pub struct PyramidBuilder {
    sample_rate: u32,
    /// Samples per block of the tier being filled, which starts at
    /// [`FINEST_BLOCK`] and is folded up each time the tier fills.
    block: usize,
    /// The finished blocks, one list per band.
    done: [Vec<Folded>; BAND_COUNT],
    running: [Running; BAND_COUNT],
    filled: usize,
    frames: usize,
    split: Split,
    samples: Vec<i16>,
    /// How many frames a source may run to and still have its samples kept.
    keep: usize,
}

impl PyramidBuilder {
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            block: FINEST_BLOCK,
            done: std::array::from_fn(|_| Vec::new()),
            running: [Running::EMPTY; BAND_COUNT],
            filled: 0,
            frames: 0,
            split: Split::new(sample_rate as f32),
            samples: Vec::new(),
            keep: (SAMPLE_KEEP_SECONDS * f64::from(sample_rate)) as usize,
        }
    }

    /// The next stretch of interleaved-stereo PCM.
    pub fn push(&mut self, interleaved: &[f32]) {
        for frame in interleaved.chunks_exact(2) {
            let mono = 0.5 * (frame[0] + frame[1]);
            self.frames += 1;
            // Short enough so far that the zoom could out-resolve the finest
            // tier: keep the mono mixdown too. The moment it is not, the
            // samples kept so far are let go.
            if self.frames <= self.keep {
                self.samples.push(to_i16(mono));
            } else if !self.samples.is_empty() {
                self.samples = Vec::new();
            }
            let bands = self.split.run(mono);
            for (slot, value) in self.running.iter_mut().zip(bands) {
                slot.push(value);
            }
            self.filled += 1;
            if self.filled == self.block {
                self.close_block();
            }
        }
    }

    fn close_block(&mut self) {
        for (done, slot) in self.done.iter_mut().zip(&self.running) {
            done.push(Folded {
                min: slot.min,
                max: slot.max,
                sum_sq: slot.sum_sq,
            });
        }
        self.running = [Running::EMPTY; BAND_COUNT];
        self.filled = 0;
        // The tier is full: fold it down by [`TIER_RATIO`] and carry on at the
        // coarser block, so one pyramid's memory is bounded however long the
        // file is. Only files tens of minutes long ever reach this.
        if self.done[0].len() >= MAX_BLOCKS {
            for done in &mut self.done {
                *done = done
                    .chunks(TIER_RATIO)
                    .map(|group| {
                        let mut all = Folded {
                            min: f32::MAX,
                            max: f32::MIN,
                            sum_sq: 0.0,
                        };
                        for one in group {
                            all.min = all.min.min(one.min);
                            all.max = all.max.max(one.max);
                            all.sum_sq += one.sum_sq;
                        }
                        all
                    })
                    .collect();
            }
            self.block = self.block.saturating_mul(TIER_RATIO);
        }
    }

    /// The pyramid of everything pushed.
    #[must_use]
    pub fn finish(mut self) -> PeakPyramid {
        let keeps_samples = self.frames <= self.keep;
        if self.frames == 0 || self.sample_rate == 0 {
            return PeakPyramid {
                sample_rate: self.sample_rate.max(1),
                frames: 0,
                tiers: Vec::new(),
                samples: OnceLock::from(Vec::new()),
                keeps_samples,
                source: None,
            };
        }
        let len = self.frames.div_ceil(self.block);
        let mut data = vec![PeakBlock::SILENT; len * BAND_COUNT];
        for (b, done) in self.done.iter().enumerate() {
            for (i, one) in done.iter().enumerate() {
                if let Some(cell) = data.get_mut(b * len + i) {
                    *cell = PeakBlock {
                        min: one.min,
                        max: one.max,
                        rms: (one.sum_sq / self.block as f64).sqrt() as f32,
                    };
                }
            }
        }
        if self.filled > 0 {
            let index = self.done[0].len();
            for (b, slot) in self.running.iter().enumerate() {
                if let Some(cell) = data.get_mut(b * len + index) {
                    *cell = slot.finish();
                }
            }
        }
        self.samples.shrink_to_fit();
        PeakPyramid {
            sample_rate: self.sample_rate,
            frames: self.frames,
            tiers: coarser_tiers(Tier {
                block: self.block,
                len,
                data,
            }),
            samples: OnceLock::from(self.samples),
            keeps_samples,
            source: None,
        }
    }
}

/// The finest tier and the coarser ones folded down from it.
fn coarser_tiers(finest: Tier) -> Vec<Tier> {
    let mut tiers = vec![finest];
    for _ in 1..TIERS {
        let Some(finer) = tiers.last() else { break };
        if finer.len <= 1 {
            break;
        }
        let coarse_len = finer.len.div_ceil(TIER_RATIO);
        let mut coarse = vec![PeakBlock::SILENT; coarse_len * BAND_COUNT];
        for b in 0..BAND_COUNT {
            for i in 0..coarse_len {
                let mut acc: Option<PeakBlock> = None;
                for k in 0..TIER_RATIO {
                    let src = i * TIER_RATIO + k;
                    if src >= finer.len {
                        break;
                    }
                    let block = finer.data.get(b * finer.len + src).copied();
                    acc = match (acc, block) {
                        (Some(a), Some(x)) => Some(a.merged(x)),
                        (None, Some(x)) => Some(x),
                        (a, None) => a,
                    };
                }
                if let Some(cell) = coarse.get_mut(b * coarse_len + i) {
                    *cell = acc.unwrap_or(PeakBlock::SILENT);
                }
            }
        }
        tiers.push(Tier {
            block: finer.block.saturating_mul(TIER_RATIO),
            len: coarse_len,
            data: coarse,
        });
    }
    tiers
}

/// What a peak file starts with: seven bytes saying it is one, then the
/// version of this layout. The same nine bytes every cache file of Lumit's
/// opens with, and a file with another magic or a later version is not read.
const PEAK_MAGIC: &[u8; 7] = b"LMTPEAK";
const PEAK_VERSION: u16 = 1;

/// How many frames are decoded and summarised at a time while a file's
/// pyramid is built: about a third of a megabyte in hand, whatever the file.
const BUILD_FRAMES: usize = 1 << 15;

/// The peak file for a file with this fingerprint, in `dir`. Named as the
/// frame index beside it is, with the rate the sound was summarised at, since
/// the same file summarised at another rate is another pyramid.
#[must_use]
pub fn peak_path(dir: &Path, fingerprint: &lumit_media::Fingerprint, sample_rate: u32) -> PathBuf {
    dir.join(format!("{}.{sample_rate}.peak", fingerprint.cache_key()))
}

/// This file's pyramid at `sample_rate`: read from its peak file in
/// `cache_dir` when one is there for the file as it is now, else built by
/// decoding the file through once and written there for next time. `None`
/// when the file cannot be decoded.
///
/// The peak file carries the fingerprint of the file it summarises, so one
/// left behind by a file since replaced is not believed. The cache is a
/// convenience: with no directory, or one that cannot be written, the pyramid
/// is built and handed back all the same.
#[must_use]
pub fn load_or_build(
    path: &Path,
    sample_rate: u32,
    cache_dir: Option<&Path>,
) -> Option<PeakPyramid> {
    let fingerprint = lumit_media::Fingerprint::of(path).ok();
    if let (Some(dir), Some(fp)) = (cache_dir, &fingerprint) {
        let held = std::fs::read(peak_path(dir, fp, sample_rate))
            .ok()
            .and_then(|bytes| PeakPyramid::from_bytes(&bytes, fp, sample_rate));
        if let Some(mut pyramid) = held {
            pyramid.source = Some(path.to_path_buf());
            return Some(pyramid);
        }
    }
    let mut reader = lumit_media::audio::AudioReader::open(path, sample_rate).ok()?;
    let mut builder = PyramidBuilder::new(sample_rate);
    let mut at = 0u64;
    loop {
        let (chunk, _) = reader.read(at, BUILD_FRAMES, true).ok()?;
        if chunk.is_empty() {
            break;
        }
        at += (chunk.len() / 2) as u64;
        builder.push(&chunk);
    }
    let mut pyramid = builder.finish();
    if pyramid.is_empty() {
        return None;
    }
    pyramid.source = Some(path.to_path_buf());
    if let (Some(dir), Some(fp)) = (cache_dir, &fingerprint) {
        // Best-effort: a cache that cannot be written costs the next run the
        // decode again, never this one its answer.
        if std::fs::create_dir_all(dir).is_ok() {
            let _ = std::fs::write(peak_path(dir, fp, sample_rate), pyramid.to_bytes(fp));
        }
    }
    Some(pyramid)
}

impl PeakPyramid {
    /// Summarise interleaved-stereo PCM that is all in hand:
    /// [`PyramidBuilder`] handed the lot at once.
    #[must_use]
    pub fn build(interleaved: &[f32], sample_rate: u32) -> PeakPyramid {
        let mut builder = PyramidBuilder::new(sample_rate);
        builder.push(interleaved);
        builder.finish()
    }

    /// The pyramid as a peak file's bytes: the nine-byte head, the
    /// fingerprint of the file summarised, and the tiers. The samples are not
    /// written; see [`Self::samples`].
    #[must_use]
    pub fn to_bytes(&self, fingerprint: &lumit_media::Fingerprint) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.bytes());
        out.extend_from_slice(PEAK_MAGIC);
        out.extend_from_slice(&PEAK_VERSION.to_le_bytes());
        out.extend_from_slice(&fingerprint.size.to_le_bytes());
        out.extend_from_slice(&fingerprint.mtime_unix.to_le_bytes());
        let hash = fingerprint.content_hash.as_bytes();
        out.extend_from_slice(&(hash.len() as u32).to_le_bytes());
        out.extend_from_slice(hash);
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&(self.frames as u64).to_le_bytes());
        out.push(u8::from(self.keeps_samples));
        out.extend_from_slice(&(self.tiers.len() as u32).to_le_bytes());
        for tier in &self.tiers {
            out.extend_from_slice(&(tier.block as u64).to_le_bytes());
            out.extend_from_slice(&(tier.len as u64).to_le_bytes());
            for block in &tier.data {
                out.extend_from_slice(&block.min.to_le_bytes());
                out.extend_from_slice(&block.max.to_le_bytes());
                out.extend_from_slice(&block.rms.to_le_bytes());
            }
        }
        out
    }

    /// A pyramid read back from a peak file's bytes, or `None` for bytes that
    /// are not one, were written by a newer Lumit, summarise another file or
    /// another rate, or do not add up. A refusal costs a rebuild and nothing
    /// else, so anything doubtful is refused.
    #[must_use]
    pub fn from_bytes(
        bytes: &[u8],
        fingerprint: &lumit_media::Fingerprint,
        sample_rate: u32,
    ) -> Option<PeakPyramid> {
        /// The next `n` bytes, moving `rest` past them.
        fn take<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            let (head, tail) = rest.split_at_checked(n)?;
            *rest = tail;
            Some(head)
        }
        fn u32_of(rest: &mut &[u8]) -> Option<u32> {
            Some(u32::from_le_bytes(take(rest, 4)?.try_into().ok()?))
        }
        fn u64_of(rest: &mut &[u8]) -> Option<u64> {
            Some(u64::from_le_bytes(take(rest, 8)?.try_into().ok()?))
        }
        fn f32_of(rest: &mut &[u8]) -> Option<f32> {
            Some(f32::from_le_bytes(take(rest, 4)?.try_into().ok()?))
        }

        let mut rest = bytes;
        if take(&mut rest, 7)? != PEAK_MAGIC {
            return None;
        }
        if u16::from_le_bytes(take(&mut rest, 2)?.try_into().ok()?) != PEAK_VERSION {
            return None;
        }
        let size = u64_of(&mut rest)?;
        let mtime = i64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
        let hash_len = u32_of(&mut rest)? as usize;
        let hash = take(&mut rest, hash_len)?;
        if size != fingerprint.size
            || mtime != fingerprint.mtime_unix
            || hash != fingerprint.content_hash.as_bytes()
        {
            return None;
        }
        if u32_of(&mut rest)? != sample_rate {
            return None;
        }
        let frames = usize::try_from(u64_of(&mut rest)?).ok()?;
        let keeps_samples = *take(&mut rest, 1)?.first()? != 0;
        let tier_count = u32_of(&mut rest)? as usize;
        if frames == 0 || tier_count == 0 || tier_count > TIERS {
            return None;
        }
        let mut tiers = Vec::with_capacity(tier_count);
        for _ in 0..tier_count {
            let block = usize::try_from(u64_of(&mut rest)?).ok()?;
            let len = usize::try_from(u64_of(&mut rest)?).ok()?;
            // A tier holds no more blocks than the build ever makes, and
            // exactly as many as its block size makes of the frames.
            if block == 0 || len == 0 || len > MAX_BLOCKS + TIER_RATIO {
                return None;
            }
            let cells = len * BAND_COUNT;
            if rest.len() < cells * 12 {
                return None;
            }
            let mut data = Vec::with_capacity(cells);
            for _ in 0..cells {
                data.push(PeakBlock {
                    min: f32_of(&mut rest)?,
                    max: f32_of(&mut rest)?,
                    rms: f32_of(&mut rest)?,
                });
            }
            tiers.push(Tier { block, len, data });
        }
        if !rest.is_empty() || tiers.first()?.len != frames.div_ceil(tiers.first()?.block) {
            return None;
        }
        Some(PeakPyramid {
            sample_rate,
            frames,
            tiers,
            samples: OnceLock::new(),
            keeps_samples,
            source: None,
        })
    }

    /// The mono mixdown a fully zoomed lane draws from, or nothing for a
    /// source too long to keep one.
    ///
    /// A pyramid that was built holds it already. One read back from its
    /// file decodes it here, the first time it is asked for, from the file
    /// the pyramid summarises: once per run for a source somebody zooms
    /// right in on, and never for the rest.
    fn samples(&self) -> &[i16] {
        self.samples.get_or_init(|| {
            let (true, Some(path)) = (self.keeps_samples, &self.source) else {
                return Vec::new();
            };
            let Ok(mut reader) = lumit_media::audio::AudioReader::open(path, self.sample_rate)
            else {
                return Vec::new();
            };
            let mut samples = Vec::with_capacity(self.frames);
            while let Ok((chunk, _)) = reader.read(samples.len() as u64, BUILD_FRAMES, true) {
                if chunk.is_empty() {
                    break;
                }
                samples.extend(chunk.chunks_exact(2).map(|f| to_i16(0.5 * (f[0] + f[1]))));
            }
            samples
        })
    }

    /// How long the summarised audio runs, in seconds.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames as f64 / f64::from(self.sample_rate)
    }

    /// Whether anything was summarised at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames == 0 || self.tiers.is_empty()
    }

    /// Roughly how much memory this pyramid holds, for the cache's budget.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.tiers
            .iter()
            .map(|t| t.data.len() * std::mem::size_of::<PeakBlock>())
            .sum::<usize>()
            + self.samples.get().map_or(0, Vec::len) * std::mem::size_of::<i16>()
    }

    /// The tier to read a bucket of `samples_per_bucket` samples from: the
    /// coarsest whose blocks are at least [`BLOCKS_PER_BUCKET`] times smaller
    /// than the bucket, so the work per bucket stays near constant at every
    /// zoom and a bucket never drags in a neighbour's worth of sound. Zoomed
    /// past the finest tier's block there is nothing finer to reach for, and
    /// tier 0 is the answer.
    fn tier_for(&self, samples_per_bucket: f64) -> Option<&Tier> {
        let mut chosen = self.tiers.first()?;
        for tier in &self.tiers {
            if (tier.block.saturating_mul(BLOCKS_PER_BUCKET) as f64) <= samples_per_bucket {
                chosen = tier;
            }
        }
        Some(chosen)
    }

    /// The summary of one span of the source, in seconds. Used a bucket at a
    /// time by callers whose time mapping is not a straight line — a retimed
    /// clip, where each pixel column covers its own stretch of source.
    ///
    /// A backwards span (a clip playing in reverse) is read the same as its
    /// forwards twin; the picture of what is in the audio does not change
    /// because it is being played the other way.
    #[must_use]
    pub fn window(&self, band: Band, start_seconds: f64, end_seconds: f64) -> PeakBlock {
        let (a, b) = if start_seconds <= end_seconds {
            (start_seconds, end_seconds)
        } else {
            (end_seconds, start_seconds)
        };
        // One bucket of `range` — the span *is* the bucket.
        self.range(band, a, b, 1)
            .first()
            .copied()
            .unwrap_or(PeakBlock::SILENT)
    }

    /// `buckets` summaries evenly spanning `[start_seconds, end_seconds)` of
    /// the source — one per pixel column of a lane, which is what makes the
    /// drawn resolution follow the zoom.
    ///
    /// Buckets falling outside the audio come back silent rather than missing,
    /// so the caller's column index and the returned index always agree.
    #[must_use]
    pub fn range(
        &self,
        band: Band,
        start_seconds: f64,
        end_seconds: f64,
        buckets: usize,
    ) -> Vec<PeakBlock> {
        if buckets == 0 {
            return Vec::new();
        }
        if self.is_empty() || end_seconds <= start_seconds {
            return vec![PeakBlock::SILENT; buckets];
        }
        let rate = f64::from(self.sample_rate.max(1));
        let span = (end_seconds - start_seconds) * rate;
        let per_bucket = span / buckets as f64;
        let Some(tier) = self.tier_for(per_bucket) else {
            return vec![PeakBlock::SILENT; buckets];
        };
        let origin = start_seconds * rate;
        if self.wants_samples(per_bucket) {
            return self.range_from_samples(band, origin, per_bucket, buckets);
        }
        let mut out = Vec::with_capacity(buckets);
        for i in 0..buckets {
            let a = origin + per_bucket * i as f64;
            let b = a + per_bucket;
            let first = a.floor().max(0.0);
            let last = b.ceil().min(self.frames as f64);
            if last <= first {
                out.push(PeakBlock::SILENT);
                continue;
            }
            out.push(self.block_range(tier, band, first as usize, last as usize));
        }
        out
    }

    /// Whether a bucket this many samples wide has out-resolved the finest
    /// tier — the point past which a summary can only repeat itself, and the
    /// samples have to answer instead.
    ///
    /// The line is one block per bucket exactly. Above it every column still
    /// gets a block of its own and the summary is honest; below it columns
    /// start sharing blocks, which is the staircase. Drawing it here rather
    /// than at [`BLOCKS_PER_BUCKET`] blocks also keeps the sample scan bounded:
    /// at most one block's worth of samples per column.
    fn wants_samples(&self, samples_per_bucket: f64) -> bool {
        let finest = self.tiers.first().map_or(FINEST_BLOCK, |t| t.block);
        // Asked in this order so the samples are only fetched for a view that
        // would use them.
        samples_per_bucket < finest as f64 && !self.samples().is_empty()
    }

    /// `buckets` summaries taken straight off the samples, for a view zoomed in
    /// past what the finest tier can distinguish.
    ///
    /// One streaming pass: the band filter runs from [`SAMPLE_PREROLL`] samples
    /// before the window — so its output inside the window is what it would
    /// have been had it run from the start of the file — and each sample lands
    /// in whichever bucket its own position falls in. Nothing is allocated per
    /// sample and nothing is filtered twice.
    fn range_from_samples(
        &self,
        band: Band,
        origin: f64,
        per_bucket: f64,
        buckets: usize,
    ) -> Vec<PeakBlock> {
        let mut running = vec![Running::EMPTY; buckets];
        let first = origin.floor().max(0.0) as usize;
        let end_f = origin + per_bucket * buckets as f64;
        let last = (end_f.ceil().max(0.0) as usize).min(self.frames);
        if last <= first || per_bucket <= 0.0 {
            return vec![PeakBlock::SILENT; buckets];
        }
        // The full band is the signal itself, so it needs no filter and no
        // run-up; the three split bands need both.
        let plain = band == Band::Full;
        let pre = if plain {
            first
        } else {
            first.saturating_sub(SAMPLE_PREROLL)
        };
        let mut split = Split::new(self.sample_rate as f32);
        let samples = self.samples();
        for i in pre..last {
            let x = samples.get(i).copied().map_or(0.0, from_i16);
            let value = if plain {
                x
            } else {
                split.run(x).get(band.index()).copied().unwrap_or_default()
            };
            if i < first {
                continue; // still settling the filter
            }
            let at = ((i as f64 - origin) / per_bucket).floor();
            if at < 0.0 {
                continue;
            }
            if let Some(slot) = running.get_mut(at as usize) {
                slot.push(value);
            }
        }
        running.into_iter().map(Running::finish).collect()
    }

    /// Merge every block of `tier` that overlaps `[first, last)` samples.
    fn block_range(&self, tier: &Tier, band: Band, first: usize, last: usize) -> PeakBlock {
        if tier.block == 0 || tier.len == 0 || last <= first {
            return PeakBlock::SILENT;
        }
        let from = first / tier.block;
        // `last` is exclusive, so the final block is the one holding `last - 1`.
        let to = ((last - 1) / tier.block).min(tier.len.saturating_sub(1));
        if from > to {
            return PeakBlock::SILENT;
        }
        let mut acc = tier.at(band, from);
        for i in (from + 1)..=to {
            acc = acc.merged(tier.at(band, i));
        }
        acc
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Interleave a mono signal into the stereo shape the builder takes.
    fn stereo(mono: &[f32]) -> Vec<f32> {
        mono.iter().flat_map(|&s| [s, s]).collect()
    }

    /// A sine at `hz`, `seconds` long, at `rate`.
    fn sine(hz: f32, seconds: f32, rate: u32) -> Vec<f32> {
        let n = (seconds * rate as f32) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn the_full_band_keeps_the_signals_extremes() {
        // Half a second of full-scale square, so every block is ±1.
        let mono: Vec<f32> = (0..24_000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let p = PeakPyramid::build(&stereo(&mono), 48_000);
        assert!((p.duration_seconds() - 0.5).abs() < 1e-9);
        for block in p.range(Band::Full, 0.0, 0.5, 32) {
            assert!((block.max - 1.0).abs() < 1e-6, "max {}", block.max);
            assert!((block.min + 1.0).abs() < 1e-6, "min {}", block.min);
            assert!((block.rms - 1.0).abs() < 1e-3, "rms {}", block.rms);
        }
    }

    #[test]
    fn a_bass_tone_shows_in_the_low_band_and_not_the_high() {
        let p = PeakPyramid::build(&stereo(&sine(60.0, 0.5, 48_000)), 48_000);
        // Skip the first blocks: the filters start from rest and take a few
        // cycles to settle, which is a real property of filters and not a bug.
        let low = p.window(Band::Low, 0.25, 0.5).max;
        let high = p.window(Band::High, 0.25, 0.5).max;
        assert!(low > 0.7, "60 Hz should survive the low band, got {low}");
        assert!(
            high < 0.05,
            "60 Hz should not reach the high band, got {high}"
        );
    }

    #[test]
    fn coarse_tiers_agree_with_the_fine_one() {
        // The whole point of a mip-map: reading a wide window from a coarse
        // tier must give the same extremes as reading it from the finest.
        let mono: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 / 48_000.0 * 7.0).sin() * (i as f32 / 48_000.0))
            .collect();
        let p = PeakPyramid::build(&stereo(&mono), 48_000);
        let coarse = p.range(Band::Full, 0.0, 1.0, 8);
        // Ask for the same eight spans one at a time, each narrow enough that
        // `window` picks the finest tier it can.
        for (i, block) in coarse.iter().enumerate() {
            let a = i as f64 / 8.0;
            let fine = p.window(Band::Full, a, a + 1.0 / 8.0);
            assert!((fine.max - block.max).abs() < 1e-3, "bucket {i}");
            assert!((fine.min - block.min).abs() < 1e-3, "bucket {i}");
        }
    }

    #[test]
    fn queries_outside_the_audio_are_silent_not_missing() {
        let p = PeakPyramid::build(&stereo(&vec![0.5f32; 4_800]), 48_000);
        let out = p.range(Band::Full, -1.0, 2.0, 30);
        assert_eq!(out.len(), 30);
        assert_eq!(out[0], PeakBlock::SILENT);
        assert_eq!(out[29], PeakBlock::SILENT);
        assert!(out[10].max > 0.4);
        // A degenerate span answers silence rather than dividing by zero.
        assert_eq!(p.range(Band::Full, 0.5, 0.5, 3).len(), 3);
        assert!(p.range(Band::Full, 0.0, 1.0, 0).is_empty());
    }

    /// **A peak file read back is the pyramid that was built**: every band at
    /// every zoom draws the same, down to the view zoomed in past the summary,
    /// which the read-back pyramid answers by fetching the samples it did not
    /// store. And a peak file is not believed about a file that has changed
    /// since it was written.
    #[test]
    fn a_peak_file_read_back_draws_what_the_built_pyramid_draws() {
        use crate::stream::tests::{ramp_wav, Scratch};
        const RATE: u32 = 8_000;
        let dir = Scratch::new("peaks");
        let path = ramp_wav(&dir.0, "ramp.wav", RATE, 9);
        let cache = dir.0.join("cache");

        let whole = lumit_media::audio::decode_all(&path, RATE).unwrap();
        let in_memory = PeakPyramid::build(&whole.samples, RATE);
        // Built from the file a stretch at a time, which writes the peak file.
        let built = load_or_build(&path, RATE, Some(&cache)).unwrap();
        let fingerprint = lumit_media::Fingerprint::of(&path).unwrap();
        let file = peak_path(&cache, &fingerprint, RATE);
        assert!(file.is_file(), "the build left its peak file");
        // Read back: nothing is decoded to make this one.
        let read = load_or_build(&path, RATE, Some(&cache)).unwrap();
        assert!(
            read.samples.get().is_none(),
            "read from the file, not rebuilt"
        );

        let views = [
            (0.0, 9.0, 16),   // the whole source, from the coarsest tier
            (1.0, 7.5, 300),  // the middle tier
            (2.0, 4.0, 900),  // the finest tier
            (3.0, 3.25, 500), // past the summary: four samples a column
            (8.5, 10.0, 40),  // off the end
        ];
        for band in [Band::Full, Band::Low, Band::Mid, Band::High] {
            for (from, to, buckets) in views {
                let want = in_memory.range(band, from, to, buckets);
                assert!(
                    built.range(band, from, to, buckets) == want,
                    "{band:?} built"
                );
                assert!(read.range(band, from, to, buckets) == want, "{band:?} read");
            }
        }
        assert_eq!(read.duration_seconds(), in_memory.duration_seconds());
        assert_eq!(
            read.bytes(),
            in_memory.bytes(),
            "samples and all, once asked"
        );

        // The same file with another modification time is another file as far
        // as the cache can tell, and bytes that are not a peak file are not
        // one.
        let bytes = std::fs::read(&file).unwrap();
        let mut later = fingerprint.clone();
        later.mtime_unix += 1;
        assert!(PeakPyramid::from_bytes(&bytes, &fingerprint, RATE).is_some());
        assert!(PeakPyramid::from_bytes(&bytes, &later, RATE).is_none());
        assert!(PeakPyramid::from_bytes(&bytes, &fingerprint, 48_000).is_none());
        assert!(PeakPyramid::from_bytes(&bytes[..bytes.len() - 5], &fingerprint, RATE).is_none());
        assert!(PeakPyramid::from_bytes(b"LMTPEAK", &fingerprint, RATE).is_none());
    }
}
