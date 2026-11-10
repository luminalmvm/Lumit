//! The wave of one clip of footage.

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
