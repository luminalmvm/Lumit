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

/// Master safety ceiling: −0.3 dBFS as a linear sample amplitude
/// (`10^(−0.3/20) = 0.966050…`). docs/09-AUDIO.md §3.1 asks for a hard safety
/// clip so a hot sum leaves headroom below full scale and never reaches the
/// encoder at 0 dBFS. This is a per-sample ceiling; true inter-sample-peak
/// limiting (4× oversampled, ITU-R BS.1770) is future — a sample clamp does
/// not bound reconstruction overshoot, only the sample values themselves.
pub const MASTER_CEILING: f32 = 0.966_050_9;

/// One decoded stereo source placed on the comp's output strip.
pub struct PlacedAudio<'a> {
    /// Output frame (per-channel sample index) where this source's first
    /// sample lands. May be negative: the head that falls before the strip
    /// is clipped off, not wrapped.
    pub start_frame: i64,
    /// Interleaved stereo samples (L R L R …); length is `frames × 2`.
    pub samples: &'a [f32],
    /// Linear gain (1.0 = unity).
    pub gain: f32,
}

/// Sum `sources` into a fresh `total_frames`-long interleaved stereo buffer.
/// Overlaps add; anything falling outside `[0, total_frames)` is clipped; the
/// final mix is clamped to ±[`MASTER_CEILING`] (−0.3 dBFS, docs/09 §3.1) so a
/// hot sum can't wrap, blow the DAC, or reach the encoder at full scale.
pub fn mix_stereo(sources: &[PlacedAudio], total_frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; total_frames * 2];
    for src in sources {
        if src.gain == 0.0 || src.samples.is_empty() {
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
            let o = out_f as usize * 2;
            out[o] += src.samples[src_f * 2] * src.gain;
            out[o + 1] += src.samples[src_f * 2 + 1] * src.gain;
        }
    }
    for s in &mut out {
        *s = s.clamp(-MASTER_CEILING, MASTER_CEILING);
    }
    out
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
            gain: 1.0,
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
            gain: 1.0,
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
    fn single_source_lands_at_its_offset() {
        let s = tone(2, 0.5);
        let out = mix_stereo(
            &[PlacedAudio {
                start_frame: 1,
                samples: &s,
                gain: 1.0,
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
                    gain: 1.0,
                },
                PlacedAudio {
                    start_frame: 0,
                    samples: &b,
                    gain: 1.0,
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
                gain: 0.5,
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
                gain: 1.0,
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
                gain: 1.0,
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
                    gain: 1.0,
                },
                PlacedAudio {
                    start_frame: 0,
                    samples: &b,
                    gain: 1.0,
                },
            ],
            2,
        );
        // 0.8 + 0.8 = 1.6, held at the master ceiling, not wrapped.
        assert!(out.iter().all(|v| (v - MASTER_CEILING).abs() < 1e-6));
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
                gain: 1.0,
            }],
            2,
        );
        let out_neg = mix_stereo(
            &[PlacedAudio {
                start_frame: 0,
                samples: &hot_neg,
                gain: 1.0,
            }],
            2,
        );
        assert!(out_pos.iter().all(|v| (v - MASTER_CEILING).abs() < 1e-6));
        assert!(out_neg.iter().all(|v| (v + MASTER_CEILING).abs() < 1e-6));
        // The ceiling really is below full scale (−0.3 dBFS ≈ 0.9661).
        assert!(MASTER_CEILING < 1.0 && MASTER_CEILING > 0.96);
    }
}
