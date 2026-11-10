//! A multicam clip's wave and head picture, read from its angle's files.

use flutter_rust_bridge::frb;
use lumit_core::multicam;
use lumit_core::sequence::Clip;
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::api::layer::{BridgeAudioPeaks, LayerReference, MAX_PEAK_BUCKETS};
use crate::api::BridgeError;

/// The wave of a multicam clip over `[start_seconds, end_seconds)` of its
/// placed clock: its angle's files one after another, each read as a clip of
/// footage is, and nothing where the angle has none. Empty for a clip whose
/// speed changes along it.
#[frb(ignore)]
pub(crate) fn of(
    layer: &LayerReference,
    clip: &Clip,
    start_seconds: f64,
    end_seconds: f64,
    buckets: u32,
    multiwave: bool,
    stereo: bool,
) -> Result<BridgeAudioPeaks, BridgeError> {
    let doc = layer
        .project()?
        .read()
        .map_err(|_| BridgeError::ReadFailed)?
        .store
        .snapshot();
    let pieces = multicam::flattened(&doc, clip).unwrap_or_default();
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
    let buckets = buckets.clamp(1, MAX_PEAK_BUCKETS);
    let step = (end - start) / f64::from(buckets);
    let mut whole: Option<BridgeAudioPeaks> = None;
    for piece in &pieces {
        // The whole buckets the piece covers.
        let a = piece.place_start.to_f64().max(start);
        let b = piece.place_end().to_f64().min(end);
        if b <= a {
            continue;
        }
        let first = (((a - start) / step).round() as u32).min(buckets);
        let count = (((b - start) / step).round() as u32).min(buckets) - first;
        if count == 0 {
            continue;
        }
        let part = layer.footage_peaks(
            piece,
            start + step * f64::from(first),
            start + step * f64::from(first + count),
            count,
            multiwave,
            stereo,
        )?;
        if part.buckets != count || part.channels == 0 {
            continue;
        }
        let whole = whole.get_or_insert_with(|| BridgeAudioPeaks {
            duration_seconds: clip_end - clip_start,
            start_seconds: start,
            end_seconds: end,
            bands: part.bands,
            buckets,
            values: vec![0.0; (part.channels * part.bands * buckets * 3) as usize],
            channels: part.channels,
        });
        if part.bands != whole.bands {
            continue;
        }
        // A file with one channel beside one with two draws it on both.
        for channel in 0..whole.channels {
            let from = channel.min(part.channels - 1);
            for band in 0..whole.bands {
                let read = (3 * ((from * part.bands + band) * count)) as usize;
                let write = (3 * ((channel * whole.bands + band) * buckets + first)) as usize;
                let len = (3 * count) as usize;
                if let (Some(src), Some(dst)) = (
                    part.values.get(read..read + len),
                    whole.values.get_mut(write..write + len),
                ) {
                    dst.copy_from_slice(src);
                }
            }
        }
    }
    Ok(whole.unwrap_or_else(BridgeAudioPeaks::empty))
}

/// The footage item and the second of it a multicam clip shows at layer time
/// `lt`, or with `head` the first picture it has at all.
#[frb(ignore)]
pub(crate) fn shown(
    layer: &LayerReference,
    clip: &Clip,
    lt: f64,
    head: bool,
) -> Result<Option<(Uuid, f64)>, BridgeError> {
    let at = Rational::from_f64_on_grid(lt, 1000).map_err(|_| BridgeError::InvalidTime)?;
    let doc = layer
        .project()?
        .read()
        .map_err(|_| BridgeError::ReadFailed)?
        .store
        .snapshot();
    let shown = if head {
        multicam::opens_on(&doc, clip)
    } else {
        multicam::footage_at(&doc, clip, at)
    };
    Ok(shown.map(|shown| (shown.item, shown.source_time.to_f64())))
}
