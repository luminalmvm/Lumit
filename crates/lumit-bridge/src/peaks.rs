//! The session's waveform peak cache (docs/09-AUDIO.md §4).
//!
//! # In plain terms
//!
//! Drawing a waveform means summarising the sound, and summarising it means
//! decoding the whole track — seconds of work for a long file. That is fine
//! once, and unacceptable once per zoom step, so the summary
//! ([`lumit_audio::peaks::PeakPyramid`], which holds every zoom level at once)
//! is built the first time a lane asks for a file. It is then kept twice
//! over: in memory while Lumit runs, and as a small **peak file** beside the
//! file's frame index, so the next run, and this one after the memory has
//! moved on to other files, reads it back in a millisecond instead of
//! decoding the file again. A timeline showing three hundred sources is three
//! hundred small reads.
//!
//! The memory is keyed by **file path**, not by layer or by item: two layers
//! cut from the same music summarise it once between them, which is exactly
//! the case a music video hits on every cut. It is bounded by bytes, and the
//! least recently asked-for entry is dropped when a new one will not fit
//! (docs/14 §5, budgeted allocations).
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

/// How many sources' summaries to keep in memory at once. The byte ceiling is
/// the one that binds; this only bounds the list a look-up walks.
#[cfg(feature = "media")]
const MAX_ENTRIES: usize = 512;

/// The cache's memory ceiling.
///
/// A summary is small — about 0.2 bytes per sample, so a five-minute song costs
/// under 3 MB. What costs is the **mono sample copy** a short source keeps
/// beside it, which is what a fully zoomed lane draws from
/// (`lumit_audio::peaks::SAMPLE_KEEP_SECONDS`): 96 KB a second, so a
/// five-minute song is about 29 MB and the ten-minute ceiling is about 58 MB.
/// A pyramid holds that copy when it was built in this run, or once a lane
/// has zoomed right in on it; one read back from its peak file holds the
/// summary alone. This budget therefore holds two long songs with their
/// samples, or the summaries of an hour and a half of footage, and evicts the
/// least recently asked-for past that. It is deliberately a *byte* budget
/// rather than a count, because the count says nothing about the cost.
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

/// This file's summary: from memory, else from its peak file, else built by
/// decoding the file through once (a stretch at a time, never the whole of
/// it in hand) and written to a peak file for next time. `None` when the file
/// cannot be decoded (a missing path, a video with no audio stream), which a
/// lane draws as no waveform at all rather than as an error.
#[cfg(feature = "media")]
pub(crate) fn pyramid_for(path: &Path) -> Option<Arc<PeakPyramid>> {
    if let Ok(mut held) = cache().lock() {
        held.tick = held.tick.wrapping_add(1);
        let tick = held.tick;
        if let Some(entry) = held.entries.iter_mut().find(|e| e.path == path) {
            entry.used = tick;
            let pyramid = Arc::clone(&entry.pyramid);
            // A pyramid grows when a lane first zooms right in on it, so the
            // budget is held to on a hit as well as on a miss.
            held.trim(path);
            return Some(pyramid);
        }
    }

    // Nothing held while the file is read or FFmpeg runs. The peak files live
    // beside the frame indexes: both describe the file and not the project.
    let pyramid = Arc::new(lumit_audio::peaks::load_or_build(
        path,
        PEAK_RATE,
        lumit_project::media_index_dir().as_deref(),
    )?);

    if let Ok(mut held) = cache().lock() {
        held.tick = held.tick.wrapping_add(1);
        let tick = held.tick;
        held.entries.retain(|e| e.path != path);
        held.entries.push(PeakEntry {
            path: path.to_path_buf(),
            pyramid: Arc::clone(&pyramid),
            used: tick,
        });
        held.trim(path);
    }
    Some(pyramid)
}

#[cfg(feature = "media")]
impl PeakCache {
    /// Drop the stalest entries until both budgets are met again, never the
    /// one for `keep`: it was just asked for, and on a machine where one file
    /// alone breaks the budget, dropping it would mean fetching it again on
    /// the very next paint.
    fn trim(&mut self, keep: &Path) {
        while self.entries.len() > MAX_ENTRIES
            || self
                .entries
                .iter()
                .map(|e| e.pyramid.bytes())
                .sum::<usize>()
                > MAX_BYTES
        {
            let Some(stalest) = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.path != keep)
                .min_by_key(|(_, e)| e.used)
                .map(|(i, _)| i)
            else {
                break;
            };
            self.entries.remove(stalest);
        }
    }
}

// --- The spectrogram cache: the same bargain, for the other picture.
//
// Kept beside the peaks rather than folded into them because the two are
// asked for separately — a lane in spectral mode never wants peaks, and
// most lanes never want a spectrogram — and a combined entry would decode
// and hold both for every file either was asked about.
//
// ponytail: a file in both caches is decoded twice, once per structure;
// share one decode if the double cost ever shows in a profile.
//
// ponytail: unlike the peaks, a spectrogram is still built from the whole
// file decoded into memory, kept for four sources and written nowhere. Only a
// lane switched to the spectral view asks for one. If a long cut is ever
// worked in that view, build the grid a window at a time as the pyramid is
// built and give it a file beside the peak file.

/// How many sources' spectrograms to keep. The grid is a few kilobytes per
/// second of audio, so the byte ceiling below is the binding one.
#[cfg(feature = "media")]
const MAX_SPECTRA_ENTRIES: usize = 4;

/// The spectrogram cache's own byte ceiling: ~4.5 KB/s means a five-minute
/// song is under 2 MB, so this holds every file a session plausibly opens.
#[cfg(feature = "media")]
const MAX_SPECTRA_BYTES: usize = 32 * 1024 * 1024;

#[cfg(feature = "media")]
struct SpectraEntry {
    path: PathBuf,
    grid: Arc<lumit_audio::spectra::Spectrogram>,
    used: u64,
}

#[cfg(feature = "media")]
#[derive(Default)]
struct SpectraCache {
    entries: Vec<SpectraEntry>,
    tick: u64,
}

#[cfg(feature = "media")]
fn spectra_cache() -> &'static Mutex<SpectraCache> {
    static CACHE: OnceLock<Mutex<SpectraCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SpectraCache::default()))
}

/// This file's spectrogram, decoding and building it on the first ask —
/// the same look-up/decode/store discipline as [`pyramid_for`], with the
/// same rule about the lock: never held across the decode (docs/14 §3).
#[cfg(feature = "media")]
pub(crate) fn spectrogram_for(path: &Path) -> Option<Arc<lumit_audio::spectra::Spectrogram>> {
    if let Ok(mut held) = spectra_cache().lock() {
        held.tick = held.tick.wrapping_add(1);
        let tick = held.tick;
        if let Some(entry) = held.entries.iter_mut().find(|e| e.path == path) {
            entry.used = tick;
            return Some(Arc::clone(&entry.grid));
        }
    }

    // Nothing held while FFmpeg and the FFT run.
    let buffer = lumit_media::audio::decode_all(path, PEAK_RATE).ok()?;
    let grid = Arc::new(lumit_audio::spectra::Spectrogram::build(
        &buffer.samples,
        PEAK_RATE,
    ));
    drop(buffer);
    if grid.is_empty() {
        return None;
    }

    if let Ok(mut held) = spectra_cache().lock() {
        held.tick = held.tick.wrapping_add(1);
        let tick = held.tick;
        held.entries.retain(|e| e.path != path);
        held.entries.push(SpectraEntry {
            path: path.to_path_buf(),
            grid: Arc::clone(&grid),
            used: tick,
        });
        while held.entries.len() > MAX_SPECTRA_ENTRIES
            || held.entries.iter().map(|e| e.grid.bytes()).sum::<usize>() > MAX_SPECTRA_BYTES
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
            if held.entries.len() <= 1 {
                break;
            }
            held.entries.remove(stalest);
        }
    }
    Some(grid)
}

/// Forget everything summarised so far — peaks and spectrograms both. Called
/// when a project closes: the next project's files are different files, and a
/// stale entry is memory held for nothing.
pub(crate) fn clear() {
    #[cfg(feature = "media")]
    if let Ok(mut held) = cache().lock() {
        held.entries.clear();
    }
    #[cfg(feature = "media")]
    if let Ok(mut held) = spectra_cache().lock() {
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
