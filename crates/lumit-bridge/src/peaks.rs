//! The session's waveform peak cache (docs/09-AUDIO.md §4).
//!
//! # In plain terms
//!
//! Drawing a waveform means summarising the sound, and summarising it means
//! decoding the whole track — seconds of work for a long file. That is fine
//! once, and unacceptable once per zoom step, so the summary
//! ([`lumit_audio::peaks::PeakPyramid`], which holds every zoom level at once)
//! is built the first time a lane asks for a file and kept for the session.
//!
//! The cache is keyed by **file path**, not by layer or by item: two layers cut
//! from the same music decode it once between them, which is exactly the case a
//! music video hits on every cut. It is bounded in both directions — a few
//! entries, a few tens of megabytes — and the least recently asked-for entry is
//! dropped when a new one will not fit (docs/14 §5, budgeted allocations).
//!
//! **The lock is never held across the decode.** Look-up takes the lock, clones
//! an `Arc` and lets go; a miss decodes with nothing held and takes the lock
//! back only to store the result. Two lanes racing on the same cold file each
//! decode it once and the second simply overwrites the first with an equal
//! answer — cheaper than making either wait behind a lock held across FFmpeg
//! (docs/14 §3).

#[cfg(feature = "media")]
use std::path::{Path, PathBuf};
#[cfg(feature = "media")]
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(feature = "media")]
use lumit_audio::peaks::PeakPyramid;

/// The rate every source is summarised at. The peaks are a picture, not a
/// signal path, so one rate for all of them keeps the buckets comparable
/// between a 44.1 kHz song and a 48 kHz camera track.
#[cfg(feature = "media")]
pub(crate) const PEAK_RATE: u32 = 48_000;

/// How many sources' summaries to keep at once. Four covers the music plus a
/// handful of camera tracks an edit is usually cut against.
#[cfg(feature = "media")]
const MAX_ENTRIES: usize = 4;

/// The cache's memory ceiling.
///
/// A summary is small — about 0.2 bytes per sample, so a five-minute song costs
/// under 3 MB. What costs is the **mono sample copy** a short source keeps
/// beside it, which is what a fully zoomed lane draws from
/// (`lumit_audio::peaks::SAMPLE_KEEP_SECONDS`): 96 KB a second, so a
/// five-minute song is about 29 MB and the ten-minute ceiling is about 58 MB.
/// This budget therefore holds two long songs, or four ordinary ones, and
/// evicts the least recently asked-for past that. It is deliberately a *byte*
/// budget rather than a count, because the count says nothing about the cost.
#[cfg(feature = "media")]
const MAX_BYTES: usize = 96 * 1024 * 1024;

#[cfg(feature = "media")]
struct PeakEntry {
    path: PathBuf,
    pyramid: Arc<PeakPyramid>,
    /// When this entry was last asked for, on the cache's own counter.
    used: u64,
}

#[cfg(feature = "media")]
#[derive(Default)]
struct PeakCache {
    entries: Vec<PeakEntry>,
    tick: u64,
}

#[cfg(feature = "media")]
fn cache() -> &'static Mutex<PeakCache> {
    static CACHE: OnceLock<Mutex<PeakCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(PeakCache::default()))
}

/// This file's summary, decoding and building it if this is the first ask.
/// `None` when the file cannot be decoded — a missing path, a video with no
/// audio stream — which a lane draws as no waveform at all rather than as an
/// error.
#[cfg(feature = "media")]
pub(crate) fn pyramid_for(path: &Path) -> Option<Arc<PeakPyramid>> {
    if let Ok(mut held) = cache().lock() {
        held.tick = held.tick.wrapping_add(1);
        let tick = held.tick;
        if let Some(entry) = held.entries.iter_mut().find(|e| e.path == path) {
            entry.used = tick;
            return Some(Arc::clone(&entry.pyramid));
        }
    }

    // Nothing held while FFmpeg runs.
    let buffer = lumit_media::audio::decode_all(path, PEAK_RATE).ok()?;
    let pyramid = Arc::new(PeakPyramid::build(&buffer.samples, PEAK_RATE));
    drop(buffer);
    if pyramid.is_empty() {
        return None;
    }

    if let Ok(mut held) = cache().lock() {
        held.tick = held.tick.wrapping_add(1);
        let tick = held.tick;
        held.entries.retain(|e| e.path != path);
        held.entries.push(PeakEntry {
            path: path.to_path_buf(),
            pyramid: Arc::clone(&pyramid),
            used: tick,
        });
        // Drop the stalest until both budgets are met again.
        while held.entries.len() > MAX_ENTRIES
            || held
                .entries
                .iter()
                .map(|e| e.pyramid.bytes())
                .sum::<usize>()
                > MAX_BYTES
        {
            let Some(stalest) = held
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.used)
                .map(|(i, _)| i)
            else {
                break;
            };
            // Never evict the entry just stored: on a machine where one file
            // alone breaks the budget, evicting it would mean decoding it
            // again on the very next paint.
            if held.entries.len() <= 1 {
                break;
            }
            held.entries.remove(stalest);
        }
    }
    Some(pyramid)
}

/// Forget everything summarised so far. Called when a project closes: the next
/// project's files are different files, and a stale entry is memory held for
/// nothing.
pub(crate) fn clear() {
    #[cfg(feature = "media")]
    if let Ok(mut held) = cache().lock() {
        held.entries.clear();
    }
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A file that is not one answers "no peaks" rather than panicking, and
    /// nothing is cached for it — the next ask must be free to try again once
    /// the media is relinked.
    #[test]
    fn an_unreadable_file_caches_nothing() {
        clear();
        assert!(pyramid_for(Path::new("/definitely/not/a/file.wav")).is_none());
        let held = cache().lock().unwrap();
        assert!(held.entries.is_empty());
    }
}
