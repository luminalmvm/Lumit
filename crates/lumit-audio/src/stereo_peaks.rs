//! A sound's left and right channels summarised apart, for a timeline that
//! draws them stacked as Vegas does. The mixed summary in [`crate::peaks`] is
//! untouched: these are two more pyramids of the same kind, one a channel,
//! built in one pass through the file and kept in peak files of their own.

use std::path::{Path, PathBuf};

use lumit_media::audio::SoundSource;

use crate::peaks::{peak_path, PeakPyramid, PyramidBuilder};

/// Frames read a stretch at a time while building.
const BUILD_FRAMES: usize = 1 << 15;

/// Where one channel's peak file goes, beside the mixed one.
fn channel_path(
    dir: &Path,
    fp: &lumit_media::Fingerprint,
    sound: &SoundSource,
    rate: u32,
    side: &str,
) -> PathBuf {
    peak_path(dir, fp, sound.streams(), rate).with_extension(format!("{side}.peak"))
}

/// This sound's left and right pyramids at `sample_rate`, read from their peak
/// files in `cache_dir` or built by decoding the file once and written there.
/// `None` when the file cannot be decoded, or when its two channels are the
/// same, which is a mono sound and has its one wave.
#[must_use]
pub fn load_or_build_channels(
    sound: &SoundSource,
    sample_rate: u32,
    cache_dir: Option<&Path>,
) -> Option<[PeakPyramid; 2]> {
    let fingerprint = lumit_media::Fingerprint::of(&sound.path).ok();
    if let (Some(dir), Some(fp)) = (cache_dir, &fingerprint) {
        let read = |side: &str| {
            std::fs::read(channel_path(dir, fp, sound, sample_rate, side))
                .ok()
                .and_then(|bytes| PeakPyramid::from_bytes(&bytes, fp, sample_rate))
        };
        if let (Some(left), Some(right)) = (read("l"), read("r")) {
            return Some([left, right]);
        }
    }
    let mut reader = lumit_media::audio::SoundReader::open(sound, sample_rate).ok()?;
    let mut left = PyramidBuilder::new(sample_rate);
    let mut right = PyramidBuilder::new(sample_rate);
    let mut differs = false;
    let mut at = 0u64;
    let mut l = Vec::new();
    let mut r = Vec::new();
    loop {
        let (chunk, _) = reader.read(at, BUILD_FRAMES, true).ok()?;
        if chunk.is_empty() {
            break;
        }
        at += (chunk.len() / 2) as u64;
        l.clear();
        r.clear();
        for frame in chunk.chunks_exact(2) {
            differs |= frame[0] != frame[1];
            // Each channel goes in as both sides, so the builder's mix of
            // the two is that channel alone.
            l.extend_from_slice(&[frame[0], frame[0]]);
            r.extend_from_slice(&[frame[1], frame[1]]);
        }
        left.push(&l);
        right.push(&r);
    }
    if !differs {
        return None;
    }
    let pyramids = [left.finish(), right.finish()];
    if pyramids[0].is_empty() {
        return None;
    }
    if let (Some(dir), Some(fp)) = (cache_dir, &fingerprint) {
        // Best-effort, as the mixed peak file is.
        if std::fs::create_dir_all(dir).is_ok() {
            for (side, pyramid) in ["l", "r"].iter().zip(&pyramids) {
                let _ = std::fs::write(
                    channel_path(dir, fp, sound, sample_rate, side),
                    pyramid.to_bytes(fp),
                );
            }
        }
    }
    Some(pyramids)
}
