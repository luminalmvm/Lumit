//! Mixing several placed audio sources into one comp buffer
//! (docs/09-AUDIO.md; the comp-audio half of the playback clock).
//!
//! In plain terms: a composition can have many layers that make sound, each
//! starting at its own moment on the timeline. To play the comp we lay every
//! source down on one long strip at the right offset and add them together —
//! exactly like a mixing desk summing channels. This module is the summing:
//! it takes already-decoded, already-resampled stereo sources (each tagged
//! with where it starts and how loud) and returns one interleaved stereo
//! buffer. It is pure arithmetic — no sound card, no decoding — so every
//! rule here is a plain deterministic test.

use crate::meter;
use crate::stream::Source;
use std::sync::Arc;

/// Master safety ceiling: −0.3 dBFS as a linear sample amplitude
/// (`10^(−0.3/20) = 0.966050…`). docs/09-AUDIO.md §3.1 asks for a hard safety
/// clip so a hot sum leaves headroom below full scale and never reaches the
/// encoder at 0 dBFS. This is a per-sample ceiling; true inter-sample-peak
/// limiting (4× oversampled, ITU-R BS.1770) is future — a sample clamp does
/// not bound reconstruction overshoot, only the sample values themselves.
pub const MASTER_CEILING: f32 = 0.966_050_9;

/// The Volume property's −∞ knee (docs/09 §6): at or below this many dB the
/// layer is truly silent — the UI shows "−inf" and the mixer multiplies by
/// exactly zero, not a denormal whisper.
pub const VOLUME_FLOOR_DB: f64 = -100.0;

/// dB → linear gain for the per-layer Volume property: 0 dB = unity,
/// +6 dB ≈ ×2, and anything at or under [`VOLUME_FLOOR_DB`] is exact silence.
/// (The master ceiling still bounds a hot boosted sum.)
#[must_use]
pub fn db_to_gain(db: f64) -> f32 {
    if db <= VOLUME_FLOOR_DB {
        0.0
    } else {
        10f64.powf(db / 20.0) as f32
    }
}

/// Full left / full right on the Pan property (docs/09 §6): a
/// percentage of the way to one side, so a value well reads "L 50" or "R 20"
/// without arithmetic. 0 is centre.
pub const PAN_FULL: f64 = 100.0;

/// Pan → the pair of channel gains a **constant-power stereo balance** puts
/// on a source. `pan` is in [`PAN_FULL`] units and is clamped.
///
/// In plain terms: turning a sound to one side has to take it off the other,
/// and the ear hears loudness closer to *power* than to amplitude, so simply
/// scaling one side down by how far you turned leaves the sound sagging in the
/// middle of the sweep. The constant-power law walks a quarter circle instead
/// — `cos` on the left, `sin` on the right — so the two gains squared always
/// sum to the same thing and the sound holds its apparent level all the way
/// across.
///
/// Scaled by √2 so that **centre is unity** rather than −3 dB, which is what
/// makes this a *balance* (a control over a stereo source that starts out
/// where it belongs) rather than a *pan* (a control that places a mono source
/// in a field). The price is +3 dB on the surviving side at the extremes; the
/// master limiter is directly downstream and that is what it is for.
///
/// It composes by multiplication — a per-channel gain is all this is — which
/// is how a Precomp layer's balance rides on top of the balances inside it.
#[must_use]
pub fn pan_gains(pan: f64) -> [f32; 2] {
    let p = (pan / PAN_FULL).clamp(-1.0, 1.0);
    // 0 at full left, π/4 at centre, π/2 at full right.
    let theta = (p + 1.0) * std::f64::consts::FRAC_PI_4;
    [
        (std::f64::consts::SQRT_2 * theta.cos()) as f32,
        (std::f64::consts::SQRT_2 * theta.sin()) as f32,
    ]
}

/// An animated volume and pan, baked to control-rate gain points across a
/// placed clip: `points[p]` is the `[left, right]` gain at placed frame
/// `p × stride`, and frames in between interpolate linearly — a ~10 ms
/// control rate, plenty for fades, cheap enough for the audio callback.
/// Baked by the host (which owns the keyframes); this crate only ever reads
/// it.
///
/// **One stage carries both**. Volume and pan are a single pair of
/// channel gains by the time the mixer sees them: a second envelope would be
/// a second walk of the same clip per frame, and two ways for a fade and a
/// sweep to disagree about which sample they landed on.
#[derive(Clone, Debug, PartialEq)]
pub struct GainEnvelope {
    /// Frames per control point (≥ 1).
    pub stride: u32,
    /// `[left, right]` gains at control points 0, stride, 2×stride, …;
    /// never empty.
    pub points: Vec<[f32; 2]>,
}

impl GainEnvelope {
    /// The interpolated `[left, right]` gain at placed frame `idx` (clamped
    /// at the ends).
    #[must_use]
    pub fn gain_at(&self, idx: usize) -> [f32; 2] {
        let stride = self.stride.max(1) as usize;
        let p = idx / stride;
        let Some(&a) = self.points.get(p) else {
            return self.points.last().copied().unwrap_or([1.0, 1.0]);
        };
        let b = self.points.get(p + 1).copied().unwrap_or(a);
        let frac = (idx % stride) as f32 / stride as f32;
        [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
    }
}

/// One decoded stereo source placed on the comp's output strip.
pub struct PlacedAudio<'a> {
    /// Output frame (per-channel sample index) where this source's first
    /// sample lands. May be negative: the head that falls before the strip
    /// is clipped off, not wrapped.
    pub start_frame: i64,
    /// Interleaved stereo samples (L R L R …); length is `frames × 2`.
    pub samples: &'a [f32],
    /// `[left, right]` linear gain — Volume and Pan already multiplied
    /// together (`[1.0, 1.0]` is unity, centred). Used when `envelope` is
    /// None (both static); an animated one rides the envelope instead.
    pub gain: [f32; 2],
    /// Control-rate gain curve for an animated Volume or Pan, indexed on
    /// placed frames (0 = this source's first audible frame).
    pub envelope: Option<GainEnvelope>,
}

/// Sum `sources` into a fresh `total_frames`-long interleaved stereo buffer,
/// at unity master — [`mix_stereo_at`] with a master gain of 1.0.
pub fn mix_stereo(sources: &[PlacedAudio], total_frames: usize) -> Vec<f32> {
    mix_stereo_at(sources, total_frames, 1.0)
}

/// Sum `sources` into a fresh `total_frames`-long interleaved stereo buffer.
/// Overlaps add; anything falling outside `[0, total_frames)` is clipped; the
/// sum passes the **master fader** (`master_gain`, linear) and is then clamped
/// to ±[`MASTER_CEILING`] (−0.3 dBFS, docs/09 §3.1) so a hot mix can't wrap,
/// blow the DAC, or reach the encoder at full scale.
///
/// The fader is a stage on the sum rather than a multiplier folded into each
/// source's gain. The samples would be the same either way —
/// multiplication distributes over the sum — but only a stage lets a strip's
/// meter read the layer's own level while the master reads what the device is
/// handed, and only a stage sits where the board draws it: **ahead of** the
/// limiter, so pulling the master down is what stops the limiter working.
pub fn mix_stereo_at(sources: &[PlacedAudio], total_frames: usize, master_gain: f32) -> Vec<f32> {
    let mut out = sum_stereo(sources, total_frames);
    for s in &mut out {
        *s = (*s * master_gain).clamp(-MASTER_CEILING, MASTER_CEILING);
    }
    out
}

/// Sum `sources` the same way, with **no fader and no ceiling**: the raw sum a
/// bus insert is handed (docs/09 §3.1).
///
/// A rack on a Precomp layer processes the nested comp's sum, and that sum is
/// somewhere in the middle of the desk. The limiter is the master's own last
/// stage, and holding a bus at the master's ceiling on the way past would put
/// a second one in the middle of the signal path.
#[must_use]
pub fn sum_stereo(sources: &[PlacedAudio], total_frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; total_frames * 2];
    for src in sources {
        add_placed(&mut out, src);
    }
    out
}

/// Add one placed source into a strip being summed: [`sum_stereo`]'s own step,
/// on its own so a mix of thousands of sources can read each one, add it and
/// let it go instead of holding them all until the sum. Sources added in the
/// same order give the same samples as the one call.
pub fn add_placed(out: &mut [f32], src: &PlacedAudio) {
    let total_frames = out.len() / 2;
    if (src.gain == [0.0, 0.0] && src.envelope.is_none()) || src.samples.is_empty() {
        return;
    }
    let src_frames = src.samples.len() / 2;
    // The output frame range this source covers, clipped to the strip.
    let out_start = src.start_frame.max(0);
    let out_end = (src.start_frame + src_frames as i64).min(total_frames as i64);
    if out_end <= out_start {
        return;
    }
    for out_f in out_start..out_end {
        // The matching source frame (out_f - start_frame >= 0 here).
        let src_f = (out_f - src.start_frame) as usize;
        let g = src.envelope.as_ref().map_or(src.gain, |e| e.gain_at(src_f));
        let o = out_f as usize * 2;
        out[o] += src.samples[src_f * 2] * g[0];
        out[o + 1] += src.samples[src_f * 2 + 1] * g[1];
    }
}

/// Fold an interleaved stereo buffer down to one channel.
///
/// **The law is sum-and-halve — `(L + R) / 2`**, which is each side attenuated
/// by 6 dB before the sum. In plain terms: a mono fold-down has to answer
/// "what does one loudspeaker play?", and the arithmetic mean is the answer
/// that keeps a centred signal at exactly its own level, cannot clip a
/// correlated pair on the way down, and leaves a signal present on one side
/// only 6 dB quieter — which is what a listener in mono should hear when half
/// the picture is missing.
///
/// The alternative, ×1/√2 on each side (−3 dB), preserves the *power* of two
/// uncorrelated sides instead and is the right law for upmixing; it can
/// overshoot full scale on a correlated pair, so delivery fold-downs take the
/// mean. An odd trailing sample (a buffer that is not whole stereo frames) is
/// dropped rather than guessed at.
pub fn downmix_to_mono(interleaved: &[f32]) -> Vec<f32> {
    interleaved
        .chunks_exact(2)
        .map(|f| (f[0] + f[1]) * 0.5)
        .collect()
}

/// Where one layer's decoded audio lands on the comp strip. The footage
/// audio's sample 0 is at comp time `offset_s` (the layer's start offset);
/// the layer is only audible across its comp-timeline span `[in_s, out_s)`.
/// Returns `(output_start_frame, source_start_frame, length_frames)`, or None
/// when the layer contributes nothing (silent span, or trimmed past the end).
pub fn place_on_timeline(
    in_s: f64,
    out_s: f64,
    offset_s: f64,
    source_frames: usize,
    rate: u32,
) -> Option<(i64, usize, usize)> {
    let rate_f = f64::from(rate);
    // Can't hear the source before its own start (comp time offset_s).
    let audible_start = in_s.max(offset_s);
    if out_s <= audible_start {
        return None;
    }
    let src_start = ((audible_start - offset_s) * rate_f).round().max(0.0) as usize;
    if src_start >= source_frames {
        return None;
    }
    let out_start = (audible_start * rate_f).round() as i64;
    let want_len = ((out_s - audible_start) * rate_f).round() as usize;
    let len = want_len.min(source_frames - src_start);
    if len == 0 {
        return None;
    }
    Some((out_start, src_start, len))
}

/// One placed clip in a live [`MixPlan`]: a shared source, where it lands on
/// the comp strip, which stretch of it plays, and its gain. The same
/// placement triple [`place_on_timeline`] produces.
#[derive(Clone)]
pub struct PlacedClip {
    /// The sound: a file read in blocks, or a buffer already in memory
    /// ([`crate::stream`]).
    pub source: Arc<Source>,
    /// Output frame where the source's frame `src_start` lands (may be
    /// negative).
    pub start_frame: i64,
    pub src_start: usize,
    pub len: usize,
    /// `[left, right]` linear gain — Volume and Pan already multiplied
    /// together. Used when `envelope` is None (both static); an animated one
    /// rides the envelope instead.
    pub gain: [f32; 2],
    /// Control-rate gain curve for an animated Volume or Pan, indexed on
    /// placed frames (0 = this clip's first audible frame). Shared so plan
    /// clones stay cheap; callback-safe (read-only, allocation-free).
    pub envelope: Option<std::sync::Arc<GainEnvelope>>,
    /// Which mixer strip this clip's level is metered into
    /// ([`crate::meter`]), or [`NO_METER`] for a clip with no bar of its own.
    /// Several clips may share a strip — a Precomp layer's whole contents
    /// meter as the one row the mixer actually shows.
    pub meter: u8,
}

/// [`PlacedClip::meter`] for a clip that is not metered: past
/// [`crate::meter::MAX_STRIPS`] sounding strips, or a plan built by
/// something with no mixer to draw.
pub const NO_METER: u8 = u8::MAX;

/// How long a block a waiting caller has just had decoded stays safe from
/// being dropped for another, in milliseconds: long enough to be read.
const WARM_HOLD_MS: u64 = 2_000;

/// How many frames of a bucket [`MixPlan::peaks`] reads at most before it
/// starts striding: enough that a column's level is honest, few enough that
/// four thousand columns over a long comp stay a short walk.
pub const PEAK_SAMPLES_PER_BUCKET: usize = 256;

/// A comp's audio as a *plan* rather than a baked buffer: the placed clips
/// and the strip length. The realtime callback sums the clips sounding in
/// each buffer it is asked for ([`MixPlan::mix_into`]), so editing audio
/// (solo, mute, move, trim) is a plan swap, heard on the next callback,
/// instead of a whole-comp re-bake. The clips' sound is not in the plan: each
/// names a [`Source`], whose blocks come and go under the pool's budget
/// ([`crate::stream`]), so a plan for a two-hour cut is a few hundred
/// kilobytes whatever it plays.
#[derive(Clone)]
pub struct MixPlan {
    pub clips: Vec<PlacedClip>,
    pub total_frames: usize,
    /// The comp's **master fader** as a linear gain, applied to the sum and
    /// ahead of the ceiling. 1.0 is unity — which is why this type
    /// spells its own `Default` out rather than deriving one: a derived
    /// zero here would be a silent mix that looked like an empty field.
    pub master_gain: f32,
}

impl Default for MixPlan {
    fn default() -> Self {
        Self {
            clips: Vec::new(),
            total_frames: 0,
            master_gain: 1.0,
        }
    }
}

impl MixPlan {
    /// The mix summarised over `[start_s, end_s)` in `buckets` buckets, as the
    /// waveform lanes take a source's peaks: `min`, `max`, `rms` per bucket,
    /// each in −1..1, laid out as `3 * bucket`. What the Timeline's Sound mix
    /// row draws - the whole comp through every fader and the limiter, which
    /// is the sound that leaves the machine.
    ///
    /// Read off the plan frame by frame rather than off a summary built at
    /// import: the mix changes with every fader move, and there is no file to
    /// summarise. A bucket longer than [`PEAK_SAMPLES_PER_BUCKET`] frames is
    /// read on a stride, so a comp-wide view costs a bounded walk whatever
    /// the comp's length.
    // ponytail: a strided bucket can miss a one-frame transient at a wide
    // zoom; a pyramid built on the prepare worker is the upgrade if the
    // row ever needs to be cut against rather than read.
    #[must_use]
    pub fn peaks(&self, rate: u32, start_s: f64, end_s: f64, buckets: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; buckets * 3];
        if buckets == 0 || rate == 0 || end_s <= start_s {
            return out;
        }
        let rate_f = f64::from(rate);
        let step_s = (end_s - start_s) / buckets as f64;
        for b in 0..buckets {
            let from = ((start_s + step_s * b as f64) * rate_f).floor().max(0.0) as usize;
            let to = ((start_s + step_s * (b + 1) as f64) * rate_f)
                .ceil()
                .max(0.0) as usize;
            let to = to.min(self.total_frames);
            if to <= from {
                continue;
            }
            let stride = ((to - from) / PEAK_SAMPLES_PER_BUCKET).max(1);
            let (mut lo, mut hi, mut sq, mut n) = (0.0f32, 0.0f32, 0.0f64, 0u32);
            // The clips sounding in this bucket, found once for the bucket
            // and not once per frame read from it.
            let sounding: Vec<&PlacedClip> = self
                .clips
                .iter()
                .filter(|c| Self::sounding(c, from, to).is_some())
                .collect();
            let mut i = from;
            while i < to {
                let (l, r) = self.frame_of(i, sounding.iter().copied());
                lo = lo.min(l.min(r));
                hi = hi.max(l.max(r));
                sq += f64::from(l * l + r * r) * 0.5;
                n += 1;
                i += stride;
            }
            if n > 0 {
                out[b * 3] = lo;
                out[b * 3 + 1] = hi;
                out[b * 3 + 2] = (sq / f64::from(n)).sqrt() as f32;
            }
        }
        out
    }

    /// [`Self::peaks`] for a window too wide to read sample by sample: every
    /// file clip's part of each bucket comes from that file's own peak
    /// pyramid, asked of `pyramid_of`, and the parts are put together.
    ///
    /// # In plain terms
    ///
    /// Two hours of mix across a row two thousand columns wide is three and a
    /// half seconds a column. Reading that off the samples would mean having
    /// every file of the cut decoded, which is the thing a long cut cannot
    /// do. Each file already has a summary of itself at every zoom, so the
    /// row asks those: a clip's swing in a column is its file's swing over
    /// the same stretch, scaled by the clip's gain.
    ///
    /// Where one clip sounds in a column, which is most columns of a cut,
    /// that is the mix. Where two overlap it is an upper bound on the swing
    /// (the two extremes need not have fallen on the same sample) and the
    /// energies are added as if the two were unrelated sound, which is what
    /// two different shots are. A picture of how loud the mix is, not a
    /// measurement of it: [`Self::peaks`] reads the samples, and the caller
    /// asks that one as soon as the window is narrow enough to.
    ///
    /// A clip with no pyramid (a baked rack's output, a file that will not
    /// summarise) is read from whatever of it is in memory.
    #[must_use]
    pub fn peaks_wide(
        &self,
        rate: u32,
        start_s: f64,
        end_s: f64,
        buckets: usize,
        pyramid_of: &mut dyn FnMut(&std::path::Path) -> Option<Arc<crate::peaks::PeakPyramid>>,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; buckets * 3];
        if buckets == 0 || rate == 0 || end_s <= start_s {
            return out;
        }
        let rate_f = f64::from(rate);
        let step_s = (end_s - start_s) / buckets as f64;
        // Each bucket's low, high and mean square, summed over its clips.
        let mut sums = vec![(0.0f32, 0.0f32, 0.0f64); buckets];
        for clip in &self.clips {
            let c0 = clip.start_frame as f64 / rate_f;
            let c1 = c0 + clip.len as f64 / rate_f;
            let (from_s, to_s) = (
                c0.max(start_s),
                c1.min(end_s).min(self.total_frames as f64 / rate_f),
            );
            if to_s <= from_s {
                continue;
            }
            let pyramid = clip.source.path().and_then(&mut *pyramid_of);
            let first = ((from_s - start_s) / step_s).floor().max(0.0) as usize;
            let last = (((to_s - start_s) / step_s).ceil().max(0.0) as usize).min(buckets);
            for (b, sum) in sums.iter_mut().enumerate().take(last).skip(first) {
                let b0 = start_s + step_s * b as f64;
                let (a, z) = (b0.max(from_s), (b0 + step_s).min(to_s));
                if z <= a {
                    continue;
                }
                // The clip's frames under this bucket, and its gain half way.
                let at = ((a - c0) * rate_f) as usize;
                let count = (((z - a) * rate_f) as usize).max(1);
                let g = clip
                    .envelope
                    .as_ref()
                    .map_or(clip.gain, |e| e.gain_at(at + count / 2));
                let (lo, hi, rms) = match &pyramid {
                    Some(pyramid) => {
                        let s0 = (clip.src_start + at) as f64 / rate_f;
                        let w = pyramid.window(crate::peaks::Band::Full, s0, s0 + (z - a));
                        // The pyramid is of the two sides' mean.
                        let gm = 0.5 * (g[0] + g[1]);
                        (w.min * gm, w.max * gm, w.rms * gm)
                    }
                    None => {
                        let stride = (count / PEAK_SAMPLES_PER_BUCKET).max(1);
                        let (mut lo, mut hi, mut sq, mut n) = (0.0f32, 0.0f32, 0.0f64, 0u32);
                        for i in (at..at + count).step_by(stride) {
                            let Some((l, r)) = clip.source.frame(clip.src_start + i) else {
                                continue;
                            };
                            let (l, r) = (l * g[0], r * g[1]);
                            lo = lo.min(l.min(r));
                            hi = hi.max(l.max(r));
                            sq += f64::from(l * l + r * r) * 0.5;
                            n += 1;
                        }
                        let rms = if n == 0 {
                            0.0
                        } else {
                            (sq / f64::from(n)).sqrt() as f32
                        };
                        (lo, hi, rms)
                    }
                };
                sum.0 += lo;
                sum.1 += hi;
                // Weighted by how much of the bucket the clip covers.
                sum.2 += f64::from(rms * rms) * ((z - a) / step_s);
            }
        }
        for (b, (lo, hi, sq)) in sums.into_iter().enumerate() {
            let held = |v: f32| (v * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING);
            out[b * 3] = held(lo);
            out[b * 3 + 1] = held(hi);
            out[b * 3 + 2] = held(sq.sqrt() as f32);
        }
        out
    }

    /// The `(left, right)` of output frame `i`: every covering clip summed,
    /// through the master fader, clamped to ±[`MASTER_CEILING`] like the baked
    /// mix. A frame whose block is not in memory reads as silence.
    ///
    /// The plain statement of what the mix is, one frame at a time and every
    /// clip asked: what [`Self::mix_into`] is held against, and what the
    /// readers that want a frame here and there use. Not for the callback.
    #[must_use]
    pub fn frame_at(&self, i: usize) -> (f32, f32) {
        self.frame_of(i, self.clips.iter())
    }

    /// [`Self::frame_at`] over the clips the caller has already narrowed to.
    fn frame_of<'a>(&self, i: usize, clips: impl Iterator<Item = &'a PlacedClip>) -> (f32, f32) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for clip in clips {
            let Ok(idx) = usize::try_from(i as i64 - clip.start_frame) else {
                continue; // this clip starts later
            };
            if idx >= clip.len {
                continue; // this clip has ended
            }
            if let Some((sl, sr)) = clip.source.frame(clip.src_start + idx) {
                let g = clip.envelope.as_ref().map_or(clip.gain, |e| e.gain_at(idx));
                l += sl * g[0];
                r += sr * g[1];
            }
        }
        (
            (l * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING),
            (r * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING),
        )
    }

    /// The stretch of `clip` sounding in output frames `[from, to)`: how far
    /// into the stretch asked for it begins, the placed frame it begins at,
    /// and how many frames.
    fn sounding(clip: &PlacedClip, from: usize, to: usize) -> Option<(usize, usize, usize)> {
        let a = (from as i64).max(clip.start_frame);
        let b = (to as i64).min(clip.start_frame.saturating_add(clip.len as i64));
        (b > a).then(|| {
            (
                (a - from as i64) as usize,
                (a - clip.start_frame) as usize,
                (b - a) as usize,
            )
        })
    }

    /// **The callback's mix.** Output frames `start..` into `out` (interleaved
    /// stereo, overwritten), each the frame [`Self::frame_at`] answers, with
    /// every clip's own contribution folded into its strip's accumulator and
    /// the limited output into [`crate::meter::MASTER`] when `acc` is given.
    /// Answers how many frames of some clip were due and not in memory, which
    /// were mixed as silence.
    ///
    /// Allocation-free and lock-free. The clips are asked once per call, not
    /// once per frame: each is tested against the whole stretch and only the
    /// ones sounding in it are read, in the plan's order, so the sum is the
    /// same sum in the same order as the frame-at-a-time one. It keeps no
    /// place between calls, so a seek or a swapped plan needs nothing told.
    ///
    /// A strip reads **pre-master**, which is what a strip's bar means: it
    /// says how loud that layer is, not how loud the fader has left it. The
    /// master slot reads what the device is actually handed.
    // ponytail: every clip is tested per call, which is 2,000 compares a
    // buffer for a long cut and nothing beside the mixing. An index of clips
    // by start is the upgrade if a plan ever holds hundreds of thousands.
    pub fn mix_into(
        &self,
        start: usize,
        out: &mut [f32],
        mut acc: Option<&mut [meter::MeterAcc; meter::SLOTS]>,
    ) -> usize {
        out.fill(0.0);
        let frames = out.len() / 2;
        let mut missed = 0usize;
        for clip in &self.clips {
            let Some((at, first, count)) = Self::sounding(clip, start, start + frames) else {
                continue;
            };
            let mut strip = acc
                .as_deref_mut()
                .and_then(|acc| acc.get_mut(clip.meter as usize));
            missed += clip
                .source
                .read_runs(clip.src_start + first, count, |into, run| {
                    let Some(dst) = out.get_mut((at + into) * 2..) else {
                        return;
                    };
                    for (k, (s, o)) in run.chunks_exact(2).zip(dst.chunks_exact_mut(2)).enumerate()
                    {
                        let g = clip
                            .envelope
                            .as_ref()
                            .map_or(clip.gain, |e| e.gain_at(first + into + k));
                        let (cl, cr) = (s[0] * g[0], s[1] * g[1]);
                        if let Some(strip) = strip.as_deref_mut() {
                            strip.add(cl, cr, MASTER_CEILING);
                        }
                        o[0] += cl;
                        o[1] += cr;
                    }
                });
        }
        for frame in out.chunks_exact_mut(2) {
            frame[0] = (frame[0] * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING);
            frame[1] = (frame[1] * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING);
            if let Some(acc) = acc.as_deref_mut() {
                acc[meter::MASTER].add(frame[0], frame[1], MASTER_CEILING);
            }
        }
        missed
    }

    /// **One step of filling ahead.** Mark every block sounding in output
    /// frames `[from, to)` as wanted until `until` (on
    /// [`crate::stream::now`]'s clock) and decode the one that is due soonest
    /// and not in memory. `false` when there was nothing to decode: the
    /// stretch is ready to play.
    ///
    /// One block a call, so the thread that fills ahead looks at the playhead
    /// again between blocks and a seek is answered by the next decode, not
    /// after a queue of them. Waits for the decode; never called from the
    /// callback.
    pub fn fill_step(&self, from: usize, to: usize, until: u64) -> bool {
        let mut next: Option<(usize, &Arc<Source>, usize)> = None;
        for clip in &self.clips {
            let Some((at, first, count)) = Self::sounding(clip, from, to) else {
                continue;
            };
            if let Some((block, into)) = clip.source.want(clip.src_start + first, count, until) {
                if next.as_ref().is_none_or(|n| at + into < n.0) {
                    next = Some((at + into, &clip.source, block));
                }
            }
        }
        match next {
            Some((_, source, block)) => {
                let _ = source.block(block, false, until);
                true
            }
            None => false,
        }
    }

    /// Decode everything sounding in `[from, to)` that is not in memory, and
    /// wait for it: a scrub about to sound, a stretch about to be summarised.
    pub fn warm(&self, from: usize, to: usize) {
        let until = crate::stream::now().saturating_add(WARM_HOLD_MS);
        while self.fill_step(from, to, until) {}
    }

    /// How many frames of file sound the plan places, every clip counted: what
    /// the whole plan would hold if all of it were in memory at once.
    #[must_use]
    pub fn file_frames(&self) -> usize {
        self.clips
            .iter()
            .filter(|c| c.source.is_file())
            .map(|c| c.len)
            .sum()
    }

    /// Timeline waveform peaks straight off the plan — no whole-comp buffer
    /// is ever materialised (that buffer was the memory blowup). Same bucket
    /// shape as [`waveform_peaks`].
    #[must_use]
    pub fn waveform_peaks(&self, buckets: usize) -> Vec<(f32, f32)> {
        if self.total_frames == 0 || buckets == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(buckets);
        for b in 0..buckets {
            let start = b * self.total_frames / buckets;
            let end =
                (((b + 1) * self.total_frames / buckets).max(start + 1)).min(self.total_frames);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in start..end {
                let (l, r) = self.frame_at(i);
                let m = 0.5 * (l + r);
                lo = lo.min(m);
                hi = hi.max(m);
            }
            if lo > hi {
                (lo, hi) = (0.0, 0.0);
            }
            out.push((lo, hi));
        }
        out
    }
}

/// Down-sample interleaved-stereo PCM to `buckets` `(min, max)` pairs of the
/// mono mixdown — the timeline waveform. Each bucket spans an equal slice of
/// the audio; empty input or zero buckets yields an empty result. Pure, so the
/// waveform is a plain deterministic test like everything else here.
pub fn waveform_peaks(interleaved: &[f32], buckets: usize) -> Vec<(f32, f32)> {
    let frames = interleaved.len() / 2;
    if frames == 0 || buckets == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let start = b * frames / buckets;
        let end = (((b + 1) * frames / buckets).max(start + 1)).min(frames);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in start..end {
            let m = 0.5 * (interleaved[i * 2] + interleaved[i * 2 + 1]);
            lo = lo.min(m);
            hi = hi.max(m);
        }
        if lo > hi {
            (lo, hi) = (0.0, 0.0);
        }
        out.push((lo, hi));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn placement_confines_audio_to_the_active_span() {
        // GEN-4 bug 3: a layer must only sound across its comp-time span.
        // 4 s of 48 kHz source, audible only across comp time [1, 2).
        let rate = 48_000u32;
        let src = tone(4 * rate as usize, 0.5);
        let (out_start, src_start, len) =
            place_on_timeline(1.0, 2.0, 0.0, src.len() / 2, rate).unwrap();
        // Exactly one second, landing at comp second 1.
        assert_eq!(out_start, i64::from(rate));
        assert_eq!(src_start, rate as usize);
        assert_eq!(len, rate as usize);
        // Mixed onto a 3 s strip: silence outside [1, 2), sound within it.
        let placed = PlacedAudio {
            start_frame: out_start,
            samples: &src[src_start * 2..(src_start + len) * 2],
            gain: [1.0, 1.0],
            envelope: None,
        };
        let out = mix_stereo(&[placed], 3 * rate as usize);
        assert!(
            out[..rate as usize * 2].iter().all(|s| *s == 0.0),
            "no audio before the in-point"
        );
        assert!(
            out[2 * rate as usize * 2..].iter().all(|s| *s == 0.0),
            "no audio after the out-point"
        );
        assert!(
            out[rate as usize * 2..2 * rate as usize * 2]
                .iter()
                .all(|s| (*s - 0.5).abs() < 1e-6),
            "the source sounds across the whole active span"
        );
    }

    fn tone(frames: usize, value: f32) -> Vec<f32> {
        vec![value; frames * 2]
    }

    #[test]
    fn placement_before_comp_start_clips_the_pre_zero_head() {
        // GEN-3: a layer dragged so it starts before comp time 0. Its
        // in point and start offset move together (the body-drag covenant), so
        // in_s == offset_s == -1: at comp 0 the source is already 1 s in. The
        // active span intersected with the comp window [0, 2) is what sounds;
        // the second of source that falls before comp 0 is clipped, not wrapped.
        let rate = 48_000u32;
        let src_frames = 4 * rate as usize; // 4 s source
        let (out_start, src_start, len) =
            place_on_timeline(-1.0, 2.0, -1.0, src_frames, rate).unwrap();
        // audible_start = max(in, offset) = -1; source runs from its own frame 0,
        // landing one second before the strip; length spans -1..2 = 3 s.
        assert_eq!(out_start, -i64::from(rate));
        assert_eq!(src_start, 0);
        assert_eq!(len, 3 * rate as usize);
        // Mixed onto a 2 s comp strip: the pre-0 second is dropped and the whole
        // in-window span [0, 2) sounds the source from its 1 s mark onward.
        let src = tone(src_frames, 0.5);
        let placed = PlacedAudio {
            start_frame: out_start,
            samples: &src[src_start * 2..(src_start + len) * 2],
            gain: [1.0, 1.0],
            envelope: None,
        };
        let out = mix_stereo(&[placed], 2 * rate as usize);
        assert_eq!(out.len(), 2 * rate as usize * 2);
        assert!(
            out.iter().all(|s| (*s - 0.5).abs() < 1e-6),
            "the whole in-window span sounds; nothing before comp 0 bleeds in"
        );
    }

    /// A volume fade must sound identical through the baked mixer and the
    /// live plan — the same preview == export contract the static mix keeps.
    #[test]
    fn an_enveloped_fade_rides_through_both_mixers_identically() {
        use std::sync::Arc;
        let env = GainEnvelope {
            stride: 2,
            points: vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]],
        };
        let src = tone(4, 0.8);
        let baked = mix_stereo(
            &[PlacedAudio {
                start_frame: 0,
                samples: &src,
                gain: [1.0, 1.0],
                envelope: Some(env.clone()),
            }],
            4,
        );
        // Placed frames 0..4 fade 0 → 1: gains 0, 0.25, 0.5, 0.75.
        for (i, want) in [0.0f32, 0.2, 0.4, 0.6].iter().enumerate() {
            assert!(
                (baked[i * 2] - want).abs() < 1e-6,
                "frame {i}: {} vs {want}",
                baked[i * 2]
            );
        }
        let plan = MixPlan {
            clips: vec![PlacedClip {
                source: Source::whole(Arc::new(lumit_media::AudioBuffer {
                    rate: 48_000,
                    samples: src,
                })),
                start_frame: 0,
                src_start: 0,
                len: 4,
                gain: [1.0, 1.0],
                envelope: Some(Arc::new(env)),
                meter: 0,
            }],
            total_frames: 4,
            master_gain: 1.0,
        };
        for i in 0..4 {
            let (l, r) = plan.frame_at(i);
            assert!(
                (l - baked[i * 2]).abs() < 1e-6 && (r - baked[i * 2 + 1]).abs() < 1e-6,
                "frame {i}: plan and baked mixes disagree"
            );
        }
    }

    /// The Sound mix row's peaks are the mix itself, bucketed: a half-scale
    /// tone over the first half of a plan reads 0.5 in the first bucket and
    /// silence in the second, and a bucket long enough to be strided still
    /// finds the level it holds.
    #[test]
    fn plan_peaks_bucket_the_mixed_output() {
        use std::sync::Arc;
        let rate = 100;
        let mut samples = vec![0.5f32; 100];
        samples.extend(std::iter::repeat_n(0.0f32, 100));
        let plan = MixPlan {
            clips: vec![PlacedClip {
                source: Source::whole(Arc::new(lumit_media::AudioBuffer { rate, samples })),
                start_frame: 0,
                src_start: 0,
                len: 100,
                gain: [1.0, 1.0],
                envelope: None,
                meter: 0,
            }],
            total_frames: 100,
            master_gain: 1.0,
        };
        let peaks = plan.peaks(rate, 0.0, 1.0, 2);
        assert_eq!(peaks.len(), 6);
        assert!(
            (peaks[1] - 0.5).abs() < 1e-6,
            "the first bucket's max is the tone"
        );
        assert!((peaks[2] - 0.5).abs() < 1e-6, "and so is its rms");
        assert_eq!(&peaks[3..], &[0.0, 0.0, 0.0], "the second bucket is silent");

        // One bucket over the whole second: 100 frames is under the stride
        // threshold; a plan thirty thousand frames long is not, and the tone
        // still reads through the stride.
        let wide = MixPlan {
            total_frames: 30_000,
            clips: vec![PlacedClip {
                len: 30_000,
                source: Source::whole(Arc::new(lumit_media::AudioBuffer {
                    rate,
                    samples: vec![0.5f32; 60_000],
                })),
                ..plan.clips[0].clone()
            }],
            master_gain: 1.0,
        };
        let peaks = wide.peaks(rate, 0.0, 300.0, 1);
        assert!((peaks[1] - 0.5).abs() < 1e-6);

        // Nothing to bucket comes back as silence, never a panic.
        assert!(plan.peaks(rate, 1.0, 0.5, 4).iter().all(|v| *v == 0.0));
        assert!(plan.peaks(rate, 0.0, 1.0, 0).is_empty());
    }

    #[test]
    fn gain_scales_the_source() {
        let s = tone(2, 0.8);
        let out = mix_stereo(
            &[PlacedAudio {
                start_frame: 0,
                samples: &s,
                gain: [0.5, 0.5],
                envelope: None,
            }],
            2,
        );
        assert!(out.iter().all(|v| (v - 0.4).abs() < 1e-6));
    }

    /// The live plan must sound exactly like the baked mix: same placements,
    /// same overlap summing, same ceiling — sample for sample. This is the
    /// contract that lets the engine swap plans instead of re-baking.
    #[test]
    fn a_mix_plan_matches_the_baked_mix_sample_for_sample() {
        use std::sync::Arc;
        let buf = |frames: usize, v: f32| {
            Arc::new(lumit_media::AudioBuffer {
                rate: 48_000,
                samples: vec![v; frames * 2],
            })
        };
        let (a, b) = (buf(6, 0.6), buf(4, 0.7));
        // a: frames 0..6 at 0.6; b: frames 4..8 at 0.7 → overlap sums and
        // clamps to the master ceiling; head/tail come from one clip each.
        let baked = mix_stereo(
            &[
                PlacedAudio {
                    start_frame: 0,
                    samples: &a.samples,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
                PlacedAudio {
                    start_frame: 4,
                    samples: &b.samples,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
            ],
            10,
        );
        let plan = MixPlan {
            clips: vec![
                PlacedClip {
                    source: Source::whole(a),
                    start_frame: 0,
                    src_start: 0,
                    len: 6,
                    gain: [1.0, 1.0],
                    envelope: None,
                    meter: 0,
                },
                PlacedClip {
                    source: Source::whole(b),
                    start_frame: 4,
                    src_start: 0,
                    len: 4,
                    gain: [1.0, 1.0],
                    envelope: None,
                    meter: 0,
                },
            ],
            total_frames: 10,
            master_gain: 1.0,
        };
        for i in 0..10 {
            let (l, r) = plan.frame_at(i);
            assert!(
                (l - baked[i * 2]).abs() < 1e-6 && (r - baked[i * 2 + 1]).abs() < 1e-6,
                "frame {i}: plan ({l},{r}) vs baked ({},{})",
                baked[i * 2],
                baked[i * 2 + 1]
            );
        }
        // Trimmed clips read the right slice: src_start offsets into the source.
        let c = Arc::new(lumit_media::AudioBuffer {
            rate: 48_000,
            samples: (0..8)
                .flat_map(|n| [n as f32 * 0.1, n as f32 * 0.1])
                .collect(),
        });
        let trimmed = MixPlan {
            clips: vec![PlacedClip {
                source: Source::whole(c),
                start_frame: 0,
                src_start: 3,
                len: 2,
                gain: [1.0, 1.0],
                envelope: None,
                meter: 0,
            }],
            total_frames: 4,
            master_gain: 1.0,
        };
        assert!((trimmed.frame_at(0).0 - 0.3).abs() < 1e-6);
        assert!((trimmed.frame_at(1).0 - 0.4).abs() < 1e-6);
        assert_eq!(trimmed.frame_at(2), (0.0, 0.0), "past the trim: silence");
        // And the waveform straight off the plan matches the buckets' extremes.
        let peaks = trimmed.waveform_peaks(2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0].1 - 0.4).abs() < 1e-6);
    }

    /// Metering is an observation, never a change: `mix_into` with meters
    /// returns exactly what `frame_at` does, each clip's own contribution lands on
    /// its own strip **before** the sum, and the master slot reads the
    /// limited output — which is what makes a strip's bar say how loud that
    /// layer is rather than how loud the mix ended up.
    #[test]
    fn metering_reads_each_strip_pre_sum_and_the_master_post_limiter() {
        use std::sync::Arc;
        let buf = |v: f32| {
            Arc::new(lumit_media::AudioBuffer {
                rate: 48_000,
                samples: vec![v; 8],
            })
        };
        let clip = |v: f32, strip: u8| PlacedClip {
            source: Source::whole(buf(v)),
            start_frame: 0,
            src_start: 0,
            len: 4,
            gain: [1.0, 1.0],
            envelope: None,
            meter: strip,
        };
        // Two loud layers on their own strips: each is under the ceiling, the
        // sum is not.
        let plan = MixPlan {
            clips: vec![clip(0.8, 0), clip(0.5, 1)],
            total_frames: 4,
            master_gain: 1.0,
        };
        let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
        let mut out = [0.0f32; 8];
        assert_eq!(plan.mix_into(0, &mut out, Some(&mut acc)), 0);
        for i in 0..4 {
            assert_eq!(
                (out[i * 2], out[i * 2 + 1]),
                plan.frame_at(i),
                "frame {i}: metering must not change the sound"
            );
        }
        let meters = meter::Meters::default();
        meters.publish(&acc);

        let a = meters.read(0);
        assert!((a.peak[0] - 0.8).abs() < 1e-6, "the strip's own level");
        assert!((a.rms[0] - 0.8).abs() < 1e-6, "a constant tone's RMS is it");
        assert!(!a.clipped, "0.8 is under the ceiling on its own");
        assert!((meters.read(1).peak[0] - 0.5).abs() < 1e-6);

        let master = meters.read(meter::MASTER);
        assert!(
            (master.peak[0] - MASTER_CEILING).abs() < 1e-6,
            "the master reads what the device is handed — 1.3 held at the ceiling"
        );
        assert!(master.clipped, "and says the limiter had to hold it");

        // A clip with no strip is still heard and simply has no bar.
        let unmetered = MixPlan {
            clips: vec![clip(0.6, NO_METER)],
            total_frames: 4,
            master_gain: 1.0,
        };
        let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
        let mut out = [0.0f32; 2];
        unmetered.mix_into(0, &mut out, Some(&mut acc));
        assert!((out[0] - 0.6).abs() < 1e-6);
        let meters = meter::Meters::default();
        meters.publish(&acc);
        assert_eq!(meters.read(0), meter::MeterReading::default());
        assert!((meters.read(meter::MASTER).peak[0] - 0.6).abs() < 1e-6);
    }

    /// The master fader is a **stage ahead of the limiter**, not a
    /// per-source multiplier: a sum hot enough to be held at the ceiling
    /// comes back under it when the master is pulled down — which is the
    /// whole point of a fader in front of a limiter. It reads the same
    /// through the baked mixer and the live plan, and it leaves the strips'
    /// own bars alone.
    #[test]
    fn the_master_fader_sits_ahead_of_the_limiter_in_both_mixers() {
        use std::sync::Arc;
        let src = tone(4, 0.8);
        let placed = || PlacedAudio {
            start_frame: 0,
            samples: &src,
            gain: [1.0, 1.0],
            envelope: None,
        };
        // Two of these sum to 1.6 — over the ceiling at unity master.
        let hot = mix_stereo_at(&[placed(), placed()], 4, 1.0);
        assert!(
            (hot[0] - MASTER_CEILING).abs() < 1e-6,
            "held at the ceiling"
        );

        // −6 dB on the master: 1.6 × 0.5012 = 0.802, under the ceiling. A
        // fader folded into each source would give the same number here —
        // what makes it a stage is that it happens after the sum and before
        // the clamp, which is what the next assertion is really about.
        let half = db_to_gain(-6.0);
        let faded = mix_stereo_at(&[placed(), placed()], 4, half);
        assert!(
            (faded[0] - 1.6 * half).abs() < 1e-5,
            "pulling the master down is what stops the limiter working, got {}",
            faded[0]
        );

        // The live plan agrees sample for sample, and its strip meters still
        // read the layer's own level rather than the fader's.
        let plan = MixPlan {
            clips: vec![
                PlacedClip {
                    source: Source::whole(Arc::new(lumit_media::AudioBuffer {
                        rate: 48_000,
                        samples: src.clone(),
                    })),
                    start_frame: 0,
                    src_start: 0,
                    len: 4,
                    gain: [1.0, 1.0],
                    envelope: None,
                    meter: 0,
                },
                PlacedClip {
                    source: Source::whole(Arc::new(lumit_media::AudioBuffer {
                        rate: 48_000,
                        samples: src.clone(),
                    })),
                    start_frame: 0,
                    src_start: 0,
                    len: 4,
                    gain: [1.0, 1.0],
                    envelope: None,
                    meter: 1,
                },
            ],
            total_frames: 4,
            master_gain: half,
        };
        let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
        let mut out = [0.0f32; 8];
        plan.mix_into(0, &mut out, Some(&mut acc));
        for i in 0..4 {
            let (l, r) = (out[i * 2], out[i * 2 + 1]);
            assert!(
                (l - faded[i * 2]).abs() < 1e-6 && (r - faded[i * 2 + 1]).abs() < 1e-6,
                "frame {i}: the live plan and the baked mix disagree about the master"
            );
        }
        let meters = meter::Meters::default();
        meters.publish(&acc);
        assert!(
            (meters.read(0).peak[0] - 0.8).abs() < 1e-6,
            "a strip's bar is the layer's own level, whatever the master does"
        );
        assert!(
            (meters.read(meter::MASTER).peak[0] - 1.6 * half).abs() < 1e-5,
            "the master's bar is what the device is handed"
        );

        // And a plan built by hand with no master named plays at unity, not
        // at silence — the reason `MixPlan` spells its own Default out.
        assert_eq!(MixPlan::default().master_gain, 1.0);
    }

    #[test]
    fn master_limiter_holds_minus_0_3_dbfs_both_polarities() {
        // docs/09 §3.1: the safety clip leaves −0.3 dBFS of headroom, so a hot
        // sum never reaches full scale on either polarity.
        let hot_pos = tone(2, 1.5);
        let hot_neg = tone(2, -1.5);
        let out_pos = mix_stereo(
            &[PlacedAudio {
                start_frame: 0,
                samples: &hot_pos,
                gain: [1.0, 1.0],
                envelope: None,
            }],
            2,
        );
        let out_neg = mix_stereo(
            &[PlacedAudio {
                start_frame: 0,
                samples: &hot_neg,
                gain: [1.0, 1.0],
                envelope: None,
            }],
            2,
        );
        // The ceiling really is below full scale (−0.3 dBFS ≈ 0.9661), so the
        // clamped output stays under 1.0 — i.e. the limiter left headroom.
        assert!(out_pos.iter().all(|v| (v - MASTER_CEILING).abs() < 1e-6));
        assert!(out_neg.iter().all(|v| (v + MASTER_CEILING).abs() < 1e-6));
        assert!(out_pos.iter().all(|v| *v < 1.0));
    }

    /// **The callback's mix is the frame-at-a-time mix**, sample for sample:
    /// the plan is asked once per buffer where it used to be asked once per
    /// frame, and that must not change a single sample of what is heard.
    ///
    /// The plan has everything a cut has. A clip that starts before the comp
    /// does, a gap, two clips crossfading under their own envelopes, a hot
    /// overlap the ceiling has to hold, and a file read in blocks, twice over
    /// and across a block's seam. It is read whole, then in stretches of odd
    /// lengths from an odd place, which is what a seek leaves the callback
    /// doing.
    #[test]
    fn the_buffer_mix_is_the_frame_mix_over_gaps_overlaps_and_a_seek() {
        use crate::stream::tests::{ramp_wav, Scratch};
        const RATE: u32 = 8_000;
        let dir = Scratch::new("mix");
        let path = ramp_wav(&dir.0, "ramp.wav", RATE, 5);
        let pool = crate::stream::Pool::new(64 * 1024 * 1024);
        let file = pool.source(&path, RATE);
        let block = crate::stream::BLOCK_SECONDS * RATE as usize;

        let wave = |frames: usize, step: f32| {
            Source::whole(Arc::new(lumit_media::AudioBuffer {
                rate: RATE,
                samples: (0..frames)
                    .flat_map(|n| {
                        let v = (n as f32 * step).sin();
                        [v * 0.8, -v * 0.6]
                    })
                    .collect(),
            }))
        };
        let ramp = |points: Vec<[f32; 2]>| Some(Arc::new(GainEnvelope { stride: 16, points }));
        let clip =
            |source: &Arc<Source>, start_frame: i64, src_start: usize, len: usize| PlacedClip {
                source: Arc::clone(source),
                start_frame,
                src_start,
                len,
                gain: [1.0, 1.0],
                envelope: None,
                meter: 0,
            };
        let (a, b) = (wave(300, 0.05), wave(300, 0.11));
        let plan = MixPlan {
            clips: vec![
                // Starts before the comp does.
                clip(&a, -20, 0, 100),
                // A gap, then two clips crossing under opposed ramps.
                PlacedClip {
                    envelope: ramp(vec![[1.0, 0.9], [0.7, 0.6], [0.2, 0.1], [0.0, 0.0]]),
                    ..clip(&a, 200, 30, 48)
                },
                PlacedClip {
                    envelope: ramp(vec![[0.0, 0.0], [0.3, 0.4], [0.8, 0.9], [1.0, 1.0]]),
                    meter: 1,
                    ..clip(&b, 216, 10, 48)
                },
                // Two loud clips at once: over the ceiling.
                PlacedClip {
                    gain: [1.5, 1.5],
                    ..clip(&a, 300, 100, 60)
                },
                PlacedClip {
                    gain: [1.5, 1.5],
                    meter: NO_METER,
                    ..clip(&b, 310, 100, 60)
                },
                // The file, across the seam between its first two blocks, and
                // the same file again a little later and overlapping.
                PlacedClip {
                    gain: [0.9, 0.7],
                    meter: 2,
                    ..clip(&file, 400, block - 30, 80)
                },
                clip(&file, 440, 2 * block - 10, 90),
            ],
            total_frames: 560,
            master_gain: 0.9,
        };
        let frames_of = |plan: &MixPlan| -> Vec<(f32, f32)> {
            (0..plan.total_frames).map(|i| plan.frame_at(i)).collect()
        };

        // Before anything of the file is decoded: its clips are silence, and
        // the callback's mix says how much of them was due.
        let mut out = vec![1.0f32; plan.total_frames * 2];
        let missed = plan.mix_into(0, &mut out, None);
        assert_eq!(missed, 80 + 90, "every frame of both file clips was due");
        let cold = frames_of(&plan);
        assert!(out.chunks_exact(2).map(|f| (f[0], f[1])).eq(cold.clone()));
        assert_eq!(cold[450], (0.0, 0.0));

        // Filled, as the thread that fills ahead leaves it.
        plan.warm(0, plan.total_frames);
        let want = frames_of(&plan);
        assert_ne!(want[450], (0.0, 0.0), "the file sounds once it is there");
        assert_eq!(plan.mix_into(0, &mut out, None), 0);
        assert!(out.chunks_exact(2).map(|f| (f[0], f[1])).eq(want.clone()));
        assert!(
            want.iter().any(|f| f.0 == MASTER_CEILING),
            "the hot overlap reached the ceiling, so the limiter is in this"
        );

        // From an odd place in stretches of odd lengths.
        let mut at = 137;
        for len in [1usize, 7, 64, 3, 250, 99] {
            let len = len.min(plan.total_frames - at);
            let mut out = vec![0.0f32; len * 2];
            let mut acc = [meter::MeterAcc::default(); meter::SLOTS];
            plan.mix_into(at, &mut out, Some(&mut acc));
            assert!(
                out.chunks_exact(2)
                    .map(|f| (f[0], f[1]))
                    .eq(want[at..at + len].iter().copied()),
                "{len} frames from {at}"
            );
            at += len;
        }

        // And the blocks are the file: the same plan over the file decoded
        // whole is the same sound.
        let whole = Source::whole(Arc::new(
            lumit_media::audio::decode_all(&path, RATE).unwrap(),
        ));
        let mut over_whole = plan.clone();
        for c in over_whole.clips.iter_mut().filter(|c| c.source.is_file()) {
            c.source = Arc::clone(&whole);
        }
        assert!(frames_of(&over_whole) == want);
    }
}
