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
//!
//! The live mix, a graph of nodes made from a plan, is [`crate::plan`] and
//! [`crate::block`]. These helpers run on whichever thread calls them.

/// Master safety ceiling: −0.3 dBFS as a linear sample amplitude
/// (`10^(−0.3/20) = 0.966050…`). docs/09-AUDIO.md §3.1 asks for a hard safety
/// clip so a hot sum leaves headroom below full scale and never reaches the
/// encoder at 0 dBFS. The baked mixers here clamp each sample to it; the live
/// mix's limiter ([`crate::limiter`]) holds the true peak under a plan's
/// ceiling, which is this when the comp names none.
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

    /// The master fader is a **stage ahead of the limiter**, not a
    /// per-source multiplier: a sum hot enough to be held at the ceiling
    /// comes back under it when the master is pulled down — which is the
    /// whole point of a fader in front of a limiter.
    #[test]
    fn the_master_fader_sits_ahead_of_the_ceiling() {
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
}
