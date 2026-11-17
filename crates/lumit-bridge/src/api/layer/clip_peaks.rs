//! The wave of one clip of footage, and the samples themselves for a wave
//! zoomed in past one sample per pixel.

use super::*;

impl LayerReference {
    /// [`Self::clip_audio_peaks`] for a clip of footage, on this layer or not.
    #[frb(ignore)]
    pub(crate) fn footage_peaks(
        &self,
        clip: &lumit_core::sequence::Clip,
        start_seconds: f64,
        end_seconds: f64,
        buckets: u32,
        multiwave: bool,
        stereo: bool,
    ) -> Result<BridgeAudioPeaks, BridgeError> {
        let lumit_core::sequence::ClipSource::Footage(item) = clip.source else {
            return Ok(BridgeAudioPeaks::empty());
        };

        #[cfg(feature = "media")]
        {
            let path = {
                let proj = self.project()?;
                let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
                let snapshot = proj.store.snapshot();
                let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item)
                else {
                    return Ok(BridgeAudioPeaks::empty());
                };
                crate::api::footage::FootageReference::resolve_path(&proj, footage)
            };
            let Some(path) = path else {
                return Ok(BridgeAudioPeaks::empty());
            };
            // The streams this clip sounds are the ones its wave is of.
            let sound =
                lumit_media::audio::SoundSource::streams_of(path, clip.audio_streams.places());
            let Some(pyramid) = crate::peaks::pyramid_for(&sound) else {
                return Ok(BridgeAudioPeaks::empty());
            };

            let buckets = buckets.clamp(1, MAX_PEAK_BUCKETS) as usize;
            let clip_start = clip.place_start.to_f64();
            let clip_end = clip_start + clip.place_duration.to_f64();
            let (start, end) = if end_seconds > start_seconds {
                (
                    start_seconds.max(clip_start).min(clip_end),
                    end_seconds.max(clip_start).min(clip_end),
                )
            } else {
                (clip_start, clip_end)
            };
            if end <= start {
                return Ok(BridgeAudioPeaks::empty());
            }
            let step = (end - start) / buckets as f64;
            // Where each bucket's edge lands in the source, through the clip's
            // map. One more edge than buckets, so neighbouring buckets share
            // theirs and no sliver of source falls between two columns.
            let edges: Vec<f64> = (0..=buckets)
                .map(|i| clip.source_time(start + step * i as f64))
                .collect();
            let bands = BridgeAudioPeaks::bands_of(multiwave);
            // Left over right where asked and the sound has two channels.
            let sides = if stereo {
                crate::peaks::channel_pyramids_for(&sound)
            } else {
                None
            };
            let summaries: Vec<&lumit_audio::peaks::PeakPyramid> = match &sides {
                Some(both) => both.iter().collect(),
                None => vec![&pyramid],
            };
            let mut values = Vec::with_capacity(summaries.len() * bands.len() * buckets * 3);
            for summary in &summaries {
                for band in &bands {
                    for i in 0..buckets {
                        let (Some(&a), Some(&b)) = (edges.get(i), edges.get(i + 1)) else {
                            values.extend_from_slice(&[0.0, 0.0, 0.0]);
                            continue;
                        };
                        let block = summary.window(*band, a, b);
                        values.extend_from_slice(&[block.min, block.max, block.rms]);
                    }
                }
            }
            Ok(BridgeAudioPeaks {
                duration_seconds: pyramid.duration_seconds(),
                start_seconds: start,
                end_seconds: end,
                bands: bands.len() as u32,
                buckets: buckets as u32,
                values,
                channels: summaries.len() as u32,
            })
        }

        #[cfg(not(feature = "media"))]
        {
            let _ = (item, start_seconds, end_seconds, buckets, multiwave, stereo);
            Ok(BridgeAudioPeaks::empty())
        }
    }
}

/// The most frames per channel one sample query answers: a wave asks for
/// samples only past one sample per pixel, so a window of a few screens of
/// them is all it ever wants.
#[cfg(feature = "media")]
const MAX_SAMPLE_FRAMES: u64 = 16_384;

/// A short window of a sound's samples, for a wave zoomed in past one
/// sample per pixel. Taken at the rate the peaks are, from the same streams,
/// before any clip gain or layer Gain, which the painter applies.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAudioSamples {
    /// Where the first frame sits and where one past the last would, in the
    /// clock the peaks use: source seconds for a layer, clip-local seconds
    /// for a clip. Frame `i` sits at
    /// `start_seconds + i * (end_seconds - start_seconds) / frames`.
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Frames per channel.
    pub frames: u32,
    /// 1 for the sound mixed, or 2 for left and right apart.
    pub channels: u32,
    /// Channel-major: channel `c`'s frame `i` at `c * frames + i`, each in
    /// -1..1 before gain.
    pub values: Vec<f32>,
}

impl BridgeAudioSamples {
    #[frb(ignore)]
    fn empty() -> BridgeAudioSamples {
        BridgeAudioSamples {
            start_seconds: 0.0,
            end_seconds: 0.0,
            frames: 0,
            channels: 1,
            values: Vec::new(),
        }
    }
}

impl LayerReference {
    /// This layer's samples over `[start_seconds, end_seconds)` of the clock
    /// [`Self::audio_peaks`] takes, mixed to one channel as its wave is.
    /// Empty for a layer with no sound of its own, or a window holding more
    /// than 16,384 frames. Not sync: it decodes the window.
    pub fn audio_samples(
        &self,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<BridgeAudioSamples, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Footage { item, .. } = layer.kind else {
            return Ok(BridgeAudioSamples::empty());
        };
        let map = |t: f64| match layer.retime {
            Some(_) => layer.source_time_at(t),
            None => t,
        };
        let places = layer.audio_streams.places().to_vec();
        self.samples_of(item, &places, (start_seconds, end_seconds), &map, false)
    }

    /// One Sequence clip's samples over `[start_seconds, end_seconds)` of its
    /// own placed clock, clamped to the clip, as [`Self::clip_audio_peaks`]
    /// takes it. With `stereo`, left and right apart where the sound has
    /// two. Empty for a clip with no sound of its own (a multicam clip
    /// included), or a window holding more than 16,384 frames. Not sync: it
    /// decodes the window.
    pub fn clip_audio_samples(
        &self,
        clip: Uuid,
        start_seconds: f64,
        end_seconds: f64,
        stereo: bool,
    ) -> Result<BridgeAudioSamples, BridgeError> {
        let (clips, index) = self.clips_and_index(clip)?;
        let Some(clip) = clips.get(index) else {
            return Ok(BridgeAudioSamples::empty());
        };
        let lumit_core::sequence::ClipSource::Footage(item) = clip.source else {
            return Ok(BridgeAudioSamples::empty());
        };
        let clip_start = clip.place_start.to_f64();
        let clip_end = clip_start + clip.place_duration.to_f64();
        let window = (
            start_seconds.max(clip_start).min(clip_end),
            end_seconds.max(clip_start).min(clip_end),
        );
        let places = clip.audio_streams.places().to_vec();
        self.samples_of(item, &places, window, &|t| clip.source_time(t), stereo)
    }

    /// `item`'s `streams` over `window` of a clock that `map` takes to
    /// source seconds, decoded at the peaks' rate. Over a window this short
    /// the map is taken as the straight line through its two ends.
    #[frb(ignore)]
    fn samples_of(
        &self,
        item: Uuid,
        streams: &[u32],
        window: (f64, f64),
        map: &dyn Fn(f64) -> f64,
        stereo: bool,
    ) -> Result<BridgeAudioSamples, BridgeError> {
        let (start, end) = window;
        let (s0, s1) = (map(start), map(end));
        if !(end > start) || !s0.is_finite() || !s1.is_finite() || s0 == s1 {
            return Ok(BridgeAudioSamples::empty());
        }
        #[cfg(feature = "media")]
        {
            let path = {
                let proj = self.project()?;
                let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
                let snapshot = proj.store.snapshot();
                let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(item)
                else {
                    return Ok(BridgeAudioSamples::empty());
                };
                crate::api::footage::FootageReference::resolve_path(&proj, footage)
            };
            let Some(path) = path else {
                return Ok(BridgeAudioSamples::empty());
            };
            let sound = lumit_media::audio::SoundSource::streams_of(path, streams);
            let rate = f64::from(crate::peaks::PEAK_RATE);
            let first = (s0.min(s1).max(0.0) * rate).floor() as u64;
            let last = (s0.max(s1).max(0.0) * rate).ceil() as u64;
            let frames = last.saturating_sub(first);
            if frames == 0 || frames > MAX_SAMPLE_FRAMES {
                return Ok(BridgeAudioSamples::empty());
            }
            let Ok(mut reader) =
                lumit_media::audio::SoundReader::open(&sound, crate::peaks::PEAK_RATE)
            else {
                return Ok(BridgeAudioSamples::empty());
            };
            let Ok((mut read, _)) = reader.read(first, frames as usize, false) else {
                return Ok(BridgeAudioSamples::empty());
            };
            let n = frames as usize;
            read.resize(n * 2, 0.0);
            // Two channels where the wave draws two, as the peaks decide it.
            let two = stereo && crate::peaks::channel_pyramids_for(&sound).is_some();
            let mut values: Vec<f32> = if two {
                let left = read.chunks_exact(2).map(|f| f[0]);
                left.chain(read.chunks_exact(2).map(|f| f[1])).collect()
            } else {
                read.chunks_exact(2).map(|f| 0.5 * (f[0] + f[1])).collect()
            };
            // Played backwards, the frames run the other way.
            let backwards = s1 < s0;
            if backwards {
                for channel in values.chunks_exact_mut(n) {
                    channel.reverse();
                }
            }
            // Each frame's own moment taken back into the caller's clock.
            let per_source = (end - start) / (s1 - s0);
            let first_at = if backwards { last - 1 } else { first } as f64 / rate;
            let start_seconds = start + (first_at - s0) * per_source;
            let end_seconds = start_seconds + n as f64 * per_source.abs() / rate;
            Ok(BridgeAudioSamples {
                start_seconds,
                end_seconds,
                frames: n as u32,
                channels: if two { 2 } else { 1 },
                values,
            })
        }
        #[cfg(not(feature = "media"))]
        {
            let _ = (item, streams, stereo);
            Ok(BridgeAudioSamples::empty())
        }
    }
}
