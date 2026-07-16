//! Media probing and the frame index — docs/impl/media-io.md §2, slice 4.
//!
//! In plain terms: when footage is imported, Luminal reads the file's vital
//! statistics (resolution, frame rate, duration — the *probe*) and then scans
//! every packet without decoding to build the *frame index*: an exact map of
//! frame number → timestamp → nearest keyframe. The index is what makes
//! scrubbing land on exactly the right frame in slice 5, and it is cached on
//! disk keyed by a content *fingerprint* so it is built once per file.

pub mod audio;
pub mod decode;
pub mod encode;
pub mod index;
pub mod probe;

use std::path::Path;

pub use audio::AudioBuffer;
pub use decode::{DecodedFrame, VideoDecoder};
pub use encode::Encoder;
pub use index::{FrameIndex, IndexEntry};
pub use probe::{AudioInfo, MediaProbe, VideoInfo};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg: {0}")]
    Ffmpeg(String),
    #[error("path is not valid unicode")]
    BadPath,
    #[error("no streams found")]
    NoStreams,
    #[error("index cache: {0}")]
    IndexCache(String),
}

impl From<rsmpeg::error::RsmpegError> for MediaError {
    fn from(e: rsmpeg::error::RsmpegError) -> Self {
        MediaError::Ffmpeg(e.to_string())
    }
}

/// The linked FFmpeg (libavformat) version, for the boot log (K-008).
pub fn ffmpeg_version() -> String {
    format!(
        "{}.{}.{}",
        rsmpeg::ffi::LIBAVFORMAT_VERSION_MAJOR,
        rsmpeg::ffi::LIBAVFORMAT_VERSION_MINOR,
        rsmpeg::ffi::LIBAVFORMAT_VERSION_MICRO
    )
}

/// Content fingerprint for relinking and index-cache keys
/// (docs/03-DATA-MODEL.md §3): size + mtime + hash of head and tail.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_unix: i64,
    pub content_hash: String, // blake3 of first + last 64 KiB, hex
}

impl Fingerprint {
    pub fn of(path: &Path) -> Result<Self, MediaError> {
        use std::io::{Read, Seek, SeekFrom};
        let meta = std::fs::metadata(path)?;
        let size = meta.len();
        let mtime_unix = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);

        let mut file = std::fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let chunk = 64 * 1024;
        let mut buf = vec![0u8; chunk];
        let read = file.read(&mut buf)?;
        hasher.update(&buf[..read]);
        if size > (2 * chunk) as u64 {
            file.seek(SeekFrom::End(-(chunk as i64)))?;
            let read = file.read(&mut buf)?;
            hasher.update(&buf[..read]);
        }
        Ok(Self {
            size,
            mtime_unix,
            content_hash: hasher.finalize().to_hex().to_string(),
        })
    }

    /// Stable key for cache filenames.
    ///
    /// `content_hash` is always a 64-character blake3 hex digest when built
    /// via [`Fingerprint::of`], but this type is publicly constructible
    /// (e.g. round-tripped through a corrupted sidecar cache file), so a
    /// shorter string must not panic here — `str::get` returns `None`
    /// instead of indexing out of bounds.
    pub fn cache_key(&self) -> String {
        let prefix = self.content_hash.get(..32).unwrap_or(&self.content_hash);
        format!("{prefix}-{}", self.size)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Regression: `cache_key` used to slice `content_hash[..32]` directly,
    /// which panics for any `Fingerprint` whose hash is shorter than 32
    /// bytes — reachable since `Fingerprint`'s fields are all `pub` and it
    /// derives `Deserialize` (a corrupted index cache file can produce
    /// one).
    #[test]
    fn cache_key_does_not_panic_on_a_short_hash() {
        let fp = Fingerprint {
            size: 10,
            mtime_unix: 0,
            content_hash: "ab".to_string(),
        };
        assert_eq!(fp.cache_key(), "ab-10");
    }

    #[test]
    fn cache_key_does_not_panic_on_an_empty_hash() {
        let fp = Fingerprint {
            size: 0,
            mtime_unix: 0,
            content_hash: String::new(),
        };
        assert_eq!(fp.cache_key(), "-0");
    }

    #[test]
    fn fingerprint_of_zero_byte_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.bin");
        std::fs::write(&path, []).unwrap();
        let fp = Fingerprint::of(&path).unwrap();
        assert_eq!(fp.size, 0);
        assert_eq!(fp.content_hash.len(), 64);
        // Must not panic when used for a cache key either.
        assert!(fp.cache_key().ends_with("-0"));
    }
}
