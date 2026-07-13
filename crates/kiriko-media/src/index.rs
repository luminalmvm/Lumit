//! The frame index (docs/impl/media-io.md §2): a packet scan without decoding
//! that maps frame number ↔ pts ↔ nearest keyframe, cached in the sidecar.

use crate::{Fingerprint, MediaError};
use rsmpeg::ffi;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexEntry {
    pub pts: i64,
    pub keyframe: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameIndex {
    /// Stream timebase: pts × (num/den) = seconds.
    pub timebase_num: i32,
    pub timebase_den: i32,
    /// Sorted by pts (B-frames arrive reordered; we sort once at build).
    pub entries: Vec<IndexEntry>,
    /// Variable frame rate detected (docs/impl/media-io.md §2 VFR policy).
    pub vfr: bool,
    /// Median pts delta between successive frames (frame duration in pts).
    pub median_delta: i64,
    pub fingerprint: Fingerprint,
}

impl FrameIndex {
    pub fn frame_count(&self) -> usize {
        self.entries.len()
    }

    pub fn pts_of_frame(&self, n: usize) -> Option<i64> {
        self.entries.get(n).map(|e| e.pts)
    }

    /// Frame number of the nearest keyframe at or before frame `n`.
    pub fn nearest_keyframe_at_or_before(&self, n: usize) -> usize {
        let n = n.min(self.entries.len().saturating_sub(1));
        (0..=n)
            .rev()
            .find(|&i| self.entries[i].keyframe)
            .unwrap_or(0)
    }

    /// Effective frames per second from the median delta.
    pub fn fps_estimate(&self) -> f64 {
        if self.median_delta <= 0 || self.timebase_num <= 0 {
            return 0.0;
        }
        f64::from(self.timebase_den) / (f64::from(self.timebase_num) * self.median_delta as f64)
    }

    // ---- sidecar cache -------------------------------------------------

    pub fn cache_path(dir: &Path, fp: &Fingerprint) -> PathBuf {
        dir.join(format!("{}.kidx", fp.cache_key()))
    }

    pub fn save_to(&self, dir: &Path) -> Result<PathBuf, MediaError> {
        std::fs::create_dir_all(dir)?;
        let path = Self::cache_path(dir, &self.fingerprint);
        let bytes = bincode::serialize(self).map_err(|e| MediaError::IndexCache(e.to_string()))?;
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Load a cached index if one matches the file's current fingerprint.
    pub fn load_cached(dir: &Path, fp: &Fingerprint) -> Option<Self> {
        let bytes = std::fs::read(Self::cache_path(dir, fp)).ok()?;
        let index: Self = bincode::deserialize(&bytes).ok()?;
        (index.fingerprint == *fp).then_some(index)
    }
}

/// Scan every packet of the primary video stream (no decoding) and build the
/// index. Seconds for an hour of 4K — run on a background thread.
pub fn build_frame_index(path: &Path) -> Result<FrameIndex, MediaError> {
    let fingerprint = Fingerprint::of(path)?;
    let mut input = crate::probe::open_input(path)?;

    let (stream_index, timebase) = input
        .streams()
        .iter()
        .find(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_VIDEO)
        .map(|s| (s.index, s.time_base))
        .ok_or(MediaError::NoStreams)?;

    let mut entries = Vec::new();
    while let Some(packet) = input.read_packet()? {
        if packet.stream_index != stream_index {
            continue;
        }
        let pts = if packet.pts != ffi::AV_NOPTS_VALUE {
            packet.pts
        } else {
            packet.dts
        };
        if pts == ffi::AV_NOPTS_VALUE {
            continue;
        }
        entries.push(IndexEntry {
            pts,
            keyframe: packet.flags & ffi::AV_PKT_FLAG_KEY as i32 != 0,
        });
    }
    entries.sort_by_key(|e| e.pts);
    entries.dedup_by_key(|e| e.pts);

    // VFR detection: > 1% of deltas deviating > 1% from the median.
    let mut deltas: Vec<i64> = entries.windows(2).map(|w| w[1].pts - w[0].pts).collect();
    deltas.sort_unstable();
    let median_delta = deltas.get(deltas.len() / 2).copied().unwrap_or(0);
    let vfr = if median_delta > 0 && !deltas.is_empty() {
        let tolerance = (median_delta / 100).max(1);
        let outliers = deltas
            .iter()
            .filter(|&&d| (d - median_delta).abs() > tolerance)
            .count();
        outliers * 100 > deltas.len()
    } else {
        false
    };

    Ok(FrameIndex {
        timebase_num: timebase.num,
        timebase_den: timebase.den,
        entries,
        vfr,
        median_delta,
        fingerprint,
    })
}

/// Test fixtures shared by index/decode tests and downstream crates' tests
/// (enable the `test-fixtures` feature).
#[cfg(any(test, feature = "test-fixtures"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub mod tests_support {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Locate an ffmpeg CLI for fixture generation (any version encodes fine).
    pub fn ffmpeg_bin() -> Option<&'static str> {
        [
            "ffmpeg",
            "/opt/homebrew/opt/ffmpeg@7/bin/ffmpeg",
            "/opt/homebrew/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
        ]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("-version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    /// 2 s of 60 fps test pattern, H.264, GOP 30 → 120 frames, keys at 0/30/60/90.
    pub fn fixture(dir: &Path) -> Option<PathBuf> {
        let bin = ffmpeg_bin()?;
        let out = dir.join("fixture.mp4");
        let status = Command::new(bin)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=320x240:rate=60",
                "-c:v",
                "libx264",
                "-g",
                "30",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::tests_support::fixture;
    use super::*;

    #[test]
    fn probe_and_index_agree_with_the_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };

        let probe = crate::probe::probe(&file).unwrap();
        let video = probe.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert!((video.fps() - 60.0).abs() < 0.01, "fps {}", video.fps());
        assert!((probe.duration_seconds - 2.0).abs() < 0.1);

        let index = build_frame_index(&file).unwrap();
        assert_eq!(index.frame_count(), 120);
        assert!(!index.vfr);
        assert!((index.fps_estimate() - 60.0).abs() < 0.01);
        // GOP 30: keyframes at 0, 30, 60, 90
        for (n, expect) in [(0, 0), (29, 0), (30, 30), (75, 60), (119, 90)] {
            assert_eq!(index.nearest_keyframe_at_or_before(n), expect, "frame {n}");
        }
        // pts strictly increasing
        assert!(index.entries.windows(2).all(|w| w[0].pts < w[1].pts));
    }

    #[test]
    fn index_cache_round_trips_and_validates_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        let cache = dir.path().join("index");
        let index = build_frame_index(&file).unwrap();
        index.save_to(&cache).unwrap();

        let fp = Fingerprint::of(&file).unwrap();
        let loaded = FrameIndex::load_cached(&cache, &fp).expect("cache hit");
        assert_eq!(loaded, index);

        // Modifying the file invalidates the cache by fingerprint mismatch.
        let mut bytes = std::fs::read(&file).unwrap();
        let len = bytes.len();
        bytes[len - 1] ^= 0xff;
        std::fs::write(&file, bytes).unwrap();
        let fp2 = Fingerprint::of(&file).unwrap();
        assert!(FrameIndex::load_cached(&cache, &fp2).is_none());
    }

    #[test]
    fn fingerprint_distinguishes_content() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, vec![1u8; 200_000]).unwrap();
        std::fs::write(&b, vec![2u8; 200_000]).unwrap();
        let fa = Fingerprint::of(&a).unwrap();
        let fb = Fingerprint::of(&b).unwrap();
        assert_ne!(fa.content_hash, fb.content_hash);
        assert_eq!(fa.size, fb.size);
    }
}
