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

/// Full left / full right on the Pan property (docs/09 §6, K-694): a
/// percentage of the way to one side, so a value well reads "L 50" or "R 20"
/// without arithmetic. 0 is centre.
pub const PAN_FULL: f64 = 100.0;

/// Pan → the pair of channel gains a **constant-power stereo balance** puts
/// on a source (K-694). `pan` is in [`PAN_FULL`] units and is clamped.
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
/// **One stage carries both** (K-694). Volume and pan are a single pair of
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
/// source's gain (K-691). The samples would be the same either way —
/// multiplication distributes over the sum — but only a stage lets a strip's
/// meter read the layer's own level while the master reads what the device is
/// handed, and only a stage sits where the board draws it: **ahead of** the
/// limiter, so pulling the master down is what stops the limiter working.
pub fn mix_stereo_at(sources: &[PlacedAudio], total_frames: usize, master_gain: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; total_frames * 2];
    for src in sources {
        if (src.gain == [0.0, 0.0] && src.envelope.is_none()) || src.samples.is_empty() {
            continue;
        }
        let src_frames = src.samples.len() / 2;
        // The output frame range this source covers, clipped to the strip.
        let out_start = src.start_frame.max(0);
        let out_end = (src.start_frame + src_frames as i64).min(total_frames as i64);
        if out_end <= out_start {
            continue;
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
    for s in &mut out {
        *s = (*s * master_gain).clamp(-MASTER_CEILING, MASTER_CEILING);
    }
    out
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

/// One placed clip in a live [`MixPlan`]: a shared decoded buffer, where it
/// lands on the comp strip, which slice of it plays, and its gain. The same
/// placement triple [`place_on_timeline`] produces.
#[derive(Clone)]
pub struct PlacedClip {
    pub buffer: std::sync::Arc<lumit_media::AudioBuffer>,
    /// Output frame where `samples[src_start]` lands (may be negative).
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

/// A comp's audio as a *plan* rather than a baked buffer: the placed clips
/// and the strip length. The realtime callback sums the covering clips per
/// frame ([`MixPlan::frame_at`]) — a handful of multiply-adds — so editing
/// audio (solo, mute, move, trim) is a plan swap, heard on the next
/// callback, instead of a whole-comp re-bake. This is the live half of the
/// docs/09 §2 lazy-decode direction; decoded buffers are shared `Arc`s from
/// the byte-budgeted cache.
#[derive(Clone)]
pub struct MixPlan {
    pub clips: Vec<PlacedClip>,
    pub total_frames: usize,
    /// The comp's **master fader** as a linear gain, applied to the sum and
    /// ahead of the ceiling (K-691). 1.0 is unity — which is why this type
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
    /// The `(left, right)` of output frame `i`: every covering clip summed,
    /// through the master fader, clamped to ±[`MASTER_CEILING`] like the baked
    /// mix. Allocation-free and lock-free — callback-safe. O(clips) per frame,
    /// fine at layer counts.
    #[must_use]
    pub fn frame_at(&self, i: usize) -> (f32, f32) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for clip in &self.clips {
            let Ok(idx) = usize::try_from(i as i64 - clip.start_frame) else {
                continue; // this clip starts later
            };
            if idx >= clip.len {
                continue; // this clip has ended
            }
            let s = (clip.src_start + idx) * 2;
            if let (Some(&sl), Some(&sr)) =
                (clip.buffer.samples.get(s), clip.buffer.samples.get(s + 1))
            {
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

    /// The `(left, right)` of output frame `i`, as [`Self::frame_at`], with
    /// each clip's own contribution folded into its strip's accumulator and
    /// the limited output into [`crate::meter::MASTER`].
    ///
    /// The metering is the same arithmetic the bars need and none that they
    /// do not: two compares and two multiply-adds per clip per frame, on the
    /// callback's own stack. Nothing is published until the buffer is done,
    /// so a frame of sound costs no atomic traffic at all.
    ///
    /// A strip reads **pre-master**, which is what a strip's bar means: it
    /// says how loud that layer is, not how loud the fader has left it. The
    /// master slot reads what the device is actually handed.
    #[must_use]
    pub fn frame_metered(&self, i: usize, acc: &mut [meter::MeterAcc; meter::SLOTS]) -> (f32, f32) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for clip in &self.clips {
            let Ok(idx) = usize::try_from(i as i64 - clip.start_frame) else {
                continue;
            };
            if idx >= clip.len {
                continue;
            }
            let s = (clip.src_start + idx) * 2;
            if let (Some(&sl), Some(&sr)) =
                (clip.buffer.samples.get(s), clip.buffer.samples.get(s + 1))
            {
                let g = clip.envelope.as_ref().map_or(clip.gain, |e| e.gain_at(idx));
                let (cl, cr) = (sl * g[0], sr * g[1]);
                if let Some(strip) = acc.get_mut(clip.meter as usize) {
                    strip.add(cl, cr, MASTER_CEILING);
                }
                l += cl;
                r += cr;
            }
        }
        let (l, r) = (
            (l * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING),
            (r * self.master_gain).clamp(-MASTER_CEILING, MASTER_CEILING),
        );
        acc[meter::MASTER].add(l, r, MASTER_CEILING);
        (l, r)
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
    fn waveform_peaks_bucket_min_max() {
        // 4 frames: (1,1) (-1,-1) (0,0) (0,0) → mono 1, -1, 0, 0.
        let audio = [1.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 0.0];
        let peaks = waveform_peaks(&audio, 2);
        assert_eq!(peaks, vec![(-1.0, 1.0), (0.0, 0.0)]);
        // Degenerate inputs are safe.
        assert!(waveform_peaks(&[], 8).is_empty());
        assert!(waveform_peaks(&audio, 0).is_empty());
        // More buckets than frames still returns one (min,max) per bucket.
        assert_eq!(waveform_peaks(&audio, 8).len(), 8);
    }

    #[test]
    fn placement_full_clip_at_origin() {
        // 2 s of 48 kHz audio, no offset, in/out spanning it all.
        let p = place_on_timeline(0.0, 2.0, 0.0, 96_000, 48_000).unwrap();
        assert_eq!(p, (0, 0, 96_000));
    }

    #[test]
    fn placement_offset_shifts_the_output_start() {
        // Same clip started 1 s into the comp: lands at output frame 48000.
        let p = place_on_timeline(1.0, 3.0, 1.0, 96_000, 48_000).unwrap();
        assert_eq!(p, (48_000, 0, 96_000));
    }

    #[test]
    fn placement_trims_head_when_in_point_is_inside_source() {
        // Layer trimmed so it starts 0.5 s into its source.
        let p = place_on_timeline(0.5, 2.0, 0.0, 96_000, 48_000).unwrap();
        assert_eq!(p, (24_000, 24_000, 72_000));
    }

    #[test]
    fn placement_clips_length_to_available_source() {
        // Out point beyond the source end: length caps at what's left.
        let p = place_on_timeline(0.0, 10.0, 0.0, 96_000, 48_000).unwrap();
        assert_eq!(p, (0, 0, 96_000));
    }

    #[test]
    fn placement_none_when_silent_or_past_end() {
        assert!(place_on_timeline(2.0, 1.0, 0.0, 96_000, 48_000).is_none());
        assert!(place_on_timeline(5.0, 6.0, 0.0, 96_000, 48_000).is_none()); // src_start past end
    }

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
        // GEN-3 (K-153): a layer dragged so it starts before comp time 0. Its
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

    #[test]
    fn empty_mix_is_silence() {
        assert_eq!(mix_stereo(&[], 4), vec![0.0; 8]);
    }

    #[test]
    fn db_to_gain_unity_boost_and_the_inf_knee() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert!((db_to_gain(20.0) - 10.0).abs() < 1e-4);
        assert!((db_to_gain(-6.0) - 0.5012).abs() < 1e-4);
        // At and below the knee: exact silence, not a denormal whisper.
        assert_eq!(db_to_gain(VOLUME_FLOOR_DB), 0.0);
        assert_eq!(db_to_gain(-200.0), 0.0);
        assert!(db_to_gain(VOLUME_FLOOR_DB + 0.1) > 0.0);
    }

    #[test]
    fn envelope_interpolates_between_control_points_and_clamps() {
        let e = GainEnvelope {
            stride: 4,
            points: vec![[0.0, 0.0], [1.0, 1.0]],
        };
        assert_eq!(e.gain_at(0), [0.0, 0.0]);
        assert!((e.gain_at(2)[0] - 0.5).abs() < 1e-6);
        assert_eq!(e.gain_at(4), [1.0, 1.0]);
        assert_eq!(
            e.gain_at(100),
            [1.0, 1.0],
            "holds the last point past the end"
        );

        // The two channels are independent, which is what lets one envelope
        // carry a fade and a pan sweep at once (K-694).
        let swept = GainEnvelope {
            stride: 2,
            points: vec![[1.0, 0.0], [0.0, 1.0]],
        };
        assert_eq!(swept.gain_at(1), [0.5, 0.5]);
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
                buffer: Arc::new(lumit_media::AudioBuffer {
                    rate: 48_000,
                    samples: src,
                }),
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

    #[test]
    fn single_source_lands_at_its_offset() {
        let s = tone(2, 0.5);
        let out = mix_stereo(
            &[PlacedAudio {
                start_frame: 1,
                samples: &s,
                gain: [1.0, 1.0],
                envelope: None,
            }],
            4,
        );
        // Frame 0 silent, frames 1–2 = 0.5, frame 3 silent.
        assert_eq!(out, vec![0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn overlapping_sources_sum() {
        let a = tone(4, 0.3);
        let b = tone(4, 0.2);
        let out = mix_stereo(
            &[
                PlacedAudio {
                    start_frame: 0,
                    samples: &a,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
                PlacedAudio {
                    start_frame: 0,
                    samples: &b,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
            ],
            4,
        );
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
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

    #[test]
    fn negative_offset_clips_the_head() {
        // Source of 4 frames (marker values in range) starting at -2: only
        // its second half lands on the strip.
        let s: Vec<f32> = (0..4)
            .flat_map(|i| [i as f32 * 0.2, i as f32 * 0.2])
            .collect();
        let out = mix_stereo(
            &[PlacedAudio {
                start_frame: -2,
                samples: &s,
                gain: [1.0, 1.0],
                envelope: None,
            }],
            4,
        );
        // Output frame 0 = source frame 2 (0.4), frame 1 = source frame 3 (0.6).
        assert_eq!(out, vec![0.4, 0.4, 0.6, 0.6, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn tail_past_the_strip_is_clipped() {
        let s = tone(10, 0.5);
        let out = mix_stereo(
            &[PlacedAudio {
                start_frame: 2,
                samples: &s,
                gain: [1.0, 1.0],
                envelope: None,
            }],
            4,
        );
        assert_eq!(out.len(), 8);
        assert_eq!(&out[..4], &[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(&out[4..], &[0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn hot_sum_is_clamped_not_wrapped() {
        let a = tone(2, 0.8);
        let b = tone(2, 0.8);
        let out = mix_stereo(
            &[
                PlacedAudio {
                    start_frame: 0,
                    samples: &a,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
                PlacedAudio {
                    start_frame: 0,
                    samples: &b,
                    gain: [1.0, 1.0],
                    envelope: None,
                },
            ],
            2,
        );
        // 0.8 + 0.8 = 1.6, held at the master ceiling, not wrapped.
        assert!(out.iter().all(|v| (v - MASTER_CEILING).abs() < 1e-6));
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
                    buffer: a,
                    start_frame: 0,
                    src_start: 0,
                    len: 6,
                    gain: [1.0, 1.0],
                    envelope: None,
                    meter: 0,
                },
                PlacedClip {
                    buffer: b,
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
                buffer: c,
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

    /// Metering is an observation, never a change: `frame_metered` returns
    /// exactly what `frame_at` does, each clip's own contribution lands on
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
            buffer: buf(v),
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
        for i in 0..4 {
            assert_eq!(
                plan.frame_metered(i, &mut acc),
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
        let (l, _) = unmetered.frame_metered(0, &mut acc);
        assert!((l - 0.6).abs() < 1e-6);
        let meters = meter::Meters::default();
        meters.publish(&acc);
        assert_eq!(meters.read(0), meter::MeterReading::default());
        assert!((meters.read(meter::MASTER).peak[0] - 0.6).abs() < 1e-6);
    }

    /// The master fader is a **stage ahead of the limiter** (K-691), not a
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
                    buffer: Arc::new(lumit_media::AudioBuffer {
                        rate: 48_000,
                        samples: src.clone(),
                    }),
                    start_frame: 0,
                    src_start: 0,
                    len: 4,
                    gain: [1.0, 1.0],
                    envelope: None,
                    meter: 0,
                },
                PlacedClip {
                    buffer: Arc::new(lumit_media::AudioBuffer {
                        rate: 48_000,
                        samples: src.clone(),
                    }),
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
        for i in 0..4 {
            let (l, r) = plan.frame_metered(i, &mut acc);
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

    /// The fold-down law, stated four ways: a centred signal keeps its level,
    /// one side alone arrives 6 dB down, a correlated pair at full scale
    /// cannot overshoot, and an odd trailing sample is dropped rather than
    /// guessed at.
    #[test]
    fn the_mono_fold_down_is_sum_and_halve() {
        assert_eq!(downmix_to_mono(&[0.5, 0.5, -0.25, -0.25]), vec![0.5, -0.25]);
        assert_eq!(downmix_to_mono(&[1.0, 0.0]), vec![0.5]);
        assert_eq!(downmix_to_mono(&[1.0, 1.0]), vec![1.0]);
        assert_eq!(downmix_to_mono(&[0.25, 0.75]), vec![0.5]);
        // Out of phase cancels, which is what one loudspeaker really does.
        assert_eq!(downmix_to_mono(&[0.6, -0.6]), vec![0.0]);
        assert_eq!(downmix_to_mono(&[0.25, 0.75, 0.5]), vec![0.5]);
        assert!(downmix_to_mono(&[]).is_empty());
    }
}
