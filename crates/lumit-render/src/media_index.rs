//! Where a frame index comes from (docs/impl/media-io.md §2).
//!
//! In plain terms: before Lumit can show frame 4,000 of a clip it needs the
//! file's frame table — which frame sits at which timestamp, and which of them
//! are the keyframes a decoder can start from. Working that out means walking
//! every packet in the file, which takes seconds on a long clip and depends on
//! nothing but the file's bytes. So it is worked out once and written to a
//! small sidecar file in Lumit's cache folder, named after a fingerprint of the
//! file's content; every later session reads it back instead. Change the file
//! and the fingerprint changes with it, so the stale table is ignored rather
//! than believed.
//!
//! Everything in the engine that opens a decoder comes through here — the probe
//! that fills the Project panel, the Viewer's decode, the decode-ahead thread,
//! the Project panel's thumbnails — so the cache one of them warms is the cache
//! the others read. It was not always so: the decode path used to scan the file
//! itself, which is why the first preview frame after opening a project used to
//! cost seconds that had already been paid.

use std::path::{Path, PathBuf};

/// The frame index for `path`: read from the sidecar cache when one matches the
/// file's current content, else built by a packet scan and written back.
///
/// The cache is a convenience, never a requirement — a machine with no home
/// directory, an unwritable cache folder or a corrupt sidecar all still decode,
/// they just scan. Only a file whose index cannot be built at all is an error.
pub fn load_or_build_index(
    path: &Path,
) -> Result<lumit_media::FrameIndex, lumit_media::MediaError> {
    lumit_media::index::load_or_build_index(path, cache_dir().as_deref())
}

/// The sidecar directory: Lumit's own media-index cache
/// (docs/10-FILE-FORMAT.md §3), global and shared across projects because the
/// index describes the file, not the project that uses it.
fn cache_dir() -> Option<PathBuf> {
    test_cache_dir().or_else(lumit_project::media_index_dir)
}

#[cfg(not(test))]
fn test_cache_dir() -> Option<PathBuf> {
    None
}

// Tests must never write into the user's real cache folder, so they point the
// sidecar directory at a temporary one for the duration of a call. Thread-local:
// the test binary runs its tests in parallel threads, and one test's override
// must not be another's.
#[cfg(test)]
thread_local! {
    static TEST_CACHE_DIR: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_cache_dir() -> Option<PathBuf> {
    TEST_CACHE_DIR.with(|dir| dir.borrow().clone())
}

/// Run `f` with the sidecar cache pointed at `dir` (tests only).
#[cfg(test)]
pub(crate) fn with_cache_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    TEST_CACHE_DIR.with(|slot| *slot.borrow_mut() = Some(dir.to_path_buf()));
    let out = f();
    TEST_CACHE_DIR.with(|slot| *slot.borrow_mut() = None);
    out
}
