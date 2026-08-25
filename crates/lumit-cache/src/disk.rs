//! The disk cache tier (docs/06-RENDER-PIPELINE.md §5.4): rendered frames in
//! the project's sidecar folder, deletable at any time with no correctness
//! effect.
//!
//! In plain terms: frames the RAM cache would forget are parked on disk in a
//! `<project>.lum-cache/` folder next to the project file. Each frame is one
//! small file named by its content hash, so looking one up is "does this file
//! exist" — no database needed (the spec's `index.db` is a later speed-up;
//! the layout here is exactly the one it would index). Anything unreadable —
//! corrupt, truncated, from a future version — is silently deleted and simply
//! re-rendered: the cache can never make a frame wrong, only faster.
//!
//! Layout per the spec: `frames/<first two hex chars>/<hash>.kfr`, each file
//! a small header (magic, version, dimensions, pixel format, colourspace)
//! followed by LZ4-compressed pixels. The first pixel format is RGBA8 (what
//! the preview compositor produces today); fp16 planes join as a new format
//! tag when the working format reaches the CPU, which is why the header
//! carries a format field at all.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// File magic + container version ("KFR1").
const MAGIC: [u8; 4] = *b"KFR1";
/// Pixel format tag: 8-bit RGBA, display-referred sRGB.
const FORMAT_RGBA8: u32 = 1;
/// Colourspace tag: sRGB display space.
const COLOURSPACE_SRGB: u32 = 1;
/// Header: magic + format + colourspace + width + height (5 × 4 bytes).
const HEADER_LEN: usize = 20;

/// One frame on its way to disk, with what the index needs to rank it later.
pub struct Parked<'a> {
    /// The frame's content hash — its name in every tier.
    pub hash: u128,
    pub width: u32,
    pub height: u32,
    /// Tightly-packed RGBA8, exactly `width * height * 4` bytes.
    pub rgba: &'a [u8],
    /// What the frame cost to render, in milliseconds. Feeds the
    /// cheap-to-remake half of the eviction score.
    pub cost_ms: u32,
    /// The preview scale it was made at, in thousandths.
    pub scale_q: u16,
}

/// One frame loaded back from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The sidecar cache folder for a project file: `<project>.lum-cache/`
/// beside it (docs/10-FILE-FORMAT.md). None when the path has no file name.
pub fn sidecar_root(project_path: &Path) -> Option<PathBuf> {
    let name = project_path.file_name()?.to_str()?;
    Some(project_path.with_file_name(format!("{name}-cache")))
}

/// Where a project's disk cache should live, honouring the user's Settings →
/// Performance → Cache "cache root folder" override (docs/07-UI-SPEC.md §15).
///
/// In plain terms: by default the cache sits next to the project file, same
/// as always (`override_root` is `None`, delegates straight to
/// [`sidecar_root`]). When the user picks a folder instead — to park the
/// cache on a faster drive — this returns a folder under *that* root, named
/// after the project's file stem plus a short hash of its full canonical
/// path, e.g. `<override_root>/comp1-9f3a21bc-cache`. The hash exists purely
/// to keep two different projects that happen to share a file name (e.g.
/// `comp1.lum` in two different folders) from colliding on one cache folder;
/// the stem stays in the name so the folder is still recognisable by eye.
pub fn cache_root_for(project_path: &Path, override_root: Option<&Path>) -> Option<PathBuf> {
    let Some(root) = override_root else {
        return sidecar_root(project_path);
    };
    let stem = project_path.file_stem()?.to_str()?;
    // Canonicalize so the hash is stable regardless of how the path was
    // spelled (relative vs absolute, `.`/`..` segments, case on Windows);
    // fall back to the given path unchanged if that fails (e.g. the file
    // doesn't exist yet) rather than panicking or losing the entry.
    let canonical = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());
    // A stable FNV-1a hash over the path bytes, NOT `DefaultHasher`: SipHash's
    // algorithm and seeding are not guaranteed stable across Rust releases, so a
    // toolchain bump would silently rename every override cache folder — cold
    // caches and orphaned `-cache` dirs. FNV is fixed for all time.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canonical.to_string_lossy().as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(root.join(format!("{stem}-{:08x}-cache", hash as u32)))
}

/// The disk tier. All operations are best-effort and silent about IO trouble
/// (a failed write just means the frame is re-rendered later); nothing here
/// panics. One instance is meant to be owned by a single IO thread.
pub struct DiskCache {
    root: PathBuf,
    cap_bytes: u64,
    /// What is here, how big, how dear, and when it was last wanted — so
    /// presence, the byte total and the eviction order are all answered without
    /// walking the folder (docs/06 §5.4).
    index: crate::index::FrameIndex,
}

impl DiskCache {
    /// Open (or prepare) the cache under `root`.
    ///
    /// The index is read if it is there; if it is missing or unreadable — a first
    /// run, a cache from a build that had none, a half-written file — the folder
    /// is walked once and the index rebuilt from it, which is the spec's
    /// "rebuilt by scan" (docs/06 §5.4). A folder with frames in it and an empty
    /// index counts as needing a rebuild, so an index that loses everything can
    /// never quietly orphan the files it forgot.
    pub fn open(root: PathBuf, cap_bytes: u64) -> Self {
        let mut index = crate::index::FrameIndex::open(root.clone());
        let frames = root.join("frames");
        if index.is_empty() && frames.is_dir() {
            index.rebuild_from_scan(&frames);
        }
        Self {
            root,
            cap_bytes,
            index,
        }
    }

    /// Bytes currently stored, as the index accounts them.
    pub fn used_bytes(&self) -> u64 {
        self.index.used_bytes()
    }

    /// How many frames are parked.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether nothing is parked.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Every hash currently present — seeds the UI's "on disk" set for the cache
    /// bar's blue tier. Read from the index, so no folder walk.
    pub fn known_hashes(&self) -> Vec<u128> {
        self.index.hashes().collect()
    }

    /// Write the index's snapshot out — at a quiet moment, or before closing.
    /// Everything since the last one is already in its log, so this is a
    /// tidying-up rather than a durability requirement.
    pub fn flush_index(&mut self) {
        self.index.write_snapshot();
    }

    fn path_for(&self, hash: u128) -> PathBuf {
        let hex = format!("{hash:032x}");
        self.root
            .join("frames")
            .join(&hex[..2])
            .join(format!("{hex}.kfr"))
    }

    /// Whether a frame is parked, from the index rather than the filesystem.
    /// Contents unverified — corruption is discovered and discarded at load, and
    /// a file deleted behind the cache's back is dropped from the index then too.
    pub fn contains(&self, hash: u128) -> bool {
        self.index.contains(hash)
    }

    /// Park a frame on disk (write-behind). Errors are swallowed: a frame
    /// that fails to store is simply re-rendered next time.
    pub fn store(&mut self, frame: Parked<'_>) {
        let Parked {
            hash,
            width,
            height,
            rgba,
            cost_ms,
            scale_q,
        } = frame;
        if rgba.len() != (width as usize) * (height as usize) * 4 {
            return; // malformed input never reaches disk
        }
        if self.index.contains(hash) {
            // Content-addressed: already present means identical. Count the ask
            // as use, so a frame kept alive by being wanted is not evicted as
            // though it were forgotten.
            self.index.touch(hash, crate::index::now_secs());
            return;
        }
        let path = self.path_for(hash);
        let Some(dir) = path.parent() else { return };
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let mut buf = Vec::with_capacity(HEADER_LEN + rgba.len() / 2);
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_RGBA8.to_le_bytes());
        buf.extend_from_slice(&COLOURSPACE_SRGB.to_le_bytes());
        buf.extend_from_slice(&width.to_le_bytes());
        buf.extend_from_slice(&height.to_le_bytes());
        buf.extend_from_slice(&lz4_flex::compress_prepend_size(rgba));
        // Write to a sibling temp name then rename, so a torn write can never
        // look like a valid entry.
        let tmp = path.with_extension("kfr.tmp");
        let write = fs::File::create(&tmp)
            .and_then(|mut f| f.write_all(&buf))
            .and_then(|()| fs::rename(&tmp, &path));
        match write {
            Ok(()) => {
                self.index.put(
                    hash,
                    crate::index::IndexEntry {
                        bytes: buf.len() as u64,
                        cost_ms: cost_ms.max(1),
                        scale_q,
                        last_used: crate::index::now_secs(),
                    },
                );
                self.enforce_cap();
            }
            Err(_) => {
                let _ = fs::remove_file(&tmp);
            }
        }
    }

    /// Load a frame back, or None. Anything unreadable — bad magic, unknown
    /// format, truncation, failed decompression, wrong pixel count — deletes
    /// the entry and returns None (the spec's silent discard).
    pub fn load(&mut self, hash: u128) -> Option<DiskFrame> {
        let path = self.path_for(hash);
        let mut bytes = Vec::new();
        if fs::File::open(&path)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .is_err()
        {
            // Not there, or unreadable. Either way the index must stop claiming
            // it: a folder emptied by hand, or reclaimed by the operating
            // system, would otherwise leave every entry promising a frame that
            // cannot be served.
            self.remove(hash);
            return None;
        }
        let parsed = parse_kfr(&bytes);
        match parsed {
            // Wanted, so no longer stale: this is what keeps a frame the user
            // keeps returning to from being evicted ahead of one nobody asks for.
            Some(_) => self.index.touch(hash, crate::index::now_secs()),
            None => self.remove(hash),
        }
        parsed
    }

    /// Drop one entry (corruption discard, or external invalidation). The index
    /// forgets it whether or not the file was there to delete — a file removed
    /// behind the cache's back must not stay in the index promising a frame.
    pub fn remove(&mut self, hash: u128) {
        let _ = fs::remove_file(self.path_for(hash));
        self.index.remove(hash);
    }

    /// Delete every entry (Settings → Clear cache). The folder itself stays, so
    /// the tier keeps working; only the frames go. Best-effort like everything
    /// here: a file that will not delete is left, and the next scan counts it.
    pub fn clear(&mut self) {
        let frames = self.root.join("frames");
        if let Ok(shards) = fs::read_dir(&frames) {
            for shard in shards.flatten() {
                let _ = fs::remove_dir_all(shard.path());
            }
        }
        // Whatever survived deletion is what the index should now describe, so
        // the totals cannot drift from the folder on a partial clear.
        self.index.rebuild_from_scan(&frames);
    }

    /// Change the byte cap, evicting until within it (Settings → Performance
    /// sets the disk budget).
    pub fn set_cap(&mut self, cap_bytes: u64) {
        self.cap_bytes = cap_bytes;
        self.enforce_cap();
    }

    /// Evict until the total fits the cap, worst-scoring entry first — **stale ×
    /// large ÷ cheap-to-remake**, the same policy the tiers above use (docs/06
    /// §5.3), which is what the index exists to make possible. Before it, this
    /// walked the folder and could only sort by modification time.
    fn enforce_cap(&mut self) {
        let now = crate::index::now_secs();
        for hash in self.index.victims(self.cap_bytes, now) {
            self.remove(hash);
        }
    }
}

/// Decode one `.kfr` byte stream, or None if it is not exactly a well-formed
/// frame of a format this build understands.
fn parse_kfr(bytes: &[u8]) -> Option<DiskFrame> {
    if bytes.len() < HEADER_LEN || bytes[..4] != MAGIC {
        return None;
    }
    let word = |i: usize| -> u32 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&bytes[i..i + 4]);
        u32::from_le_bytes(b)
    };
    if word(4) != FORMAT_RGBA8 || word(8) != COLOURSPACE_SRGB {
        return None; // a future format: not ours to read, silently ignore
    }
    let (width, height) = (word(12), word(16));
    let rgba = lz4_flex::decompress_size_prepended(&bytes[HEADER_LEN..]).ok()?;
    if rgba.len()
        != (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?
    {
        return None;
    }
    Some(DiskFrame {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, seed: u8) -> Vec<u8> {
        (0..(w * h * 4))
            .map(|i| (i as u8).wrapping_add(seed))
            .collect()
    }

    /// One frame to park, at a stated cost. `cost_ms` is what the eviction order
    /// weighs, so a test that cares about it says so; the rest pass 8, which is
    /// what an ordinary comp frame measures.
    fn parked(hash: u128, w: u32, h: u32, rgba: &[u8], cost_ms: u32) -> Parked<'_> {
        Parked {
            hash,
            width: w,
            height: h,
            rgba,
            cost_ms,
            scale_q: 1000,
        }
    }

    #[test]
    fn round_trips_a_frame_and_reports_presence() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        let rgba = frame(8, 4, 7);
        assert!(!c.contains(42));
        c.store(parked(42, 8, 4, &rgba, 8));
        assert!(c.contains(42));
        assert!(c.used_bytes() > 0);
        let f = c.load(42).unwrap();
        assert_eq!((f.width, f.height), (8, 4));
        assert_eq!(f.rgba, rgba);
        assert!(c.known_hashes().contains(&42));
    }

    #[test]
    fn corrupt_or_foreign_entries_are_silently_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        c.store(parked(7, 4, 4, &frame(4, 4, 1), 8));
        // Truncate the file behind the cache's back.
        let hex = format!("{:032x}", 7u128);
        let path = dir
            .path()
            .join("frames")
            .join(&hex[..2])
            .join(format!("{hex}.kfr"));
        fs::write(&path, b"KFR1 garbage").unwrap();
        assert!(c.load(7).is_none());
        assert!(!path.exists(), "corrupt entry must be deleted");
        // A future-format file is left unreadable but never a wrong frame.
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut future = Vec::new();
        future.extend_from_slice(b"KFR1");
        future.extend_from_slice(&99u32.to_le_bytes()); // unknown format
        future.extend_from_slice(&[0u8; 12]);
        fs::write(&path, &future).unwrap();
        assert!(c.load(7).is_none());
    }

    /// **The cap now evicts by the spec's policy, not by modification time.**
    /// Three frames of equal size, one of them far dearer to render: the cap
    /// takes the cheap ones and keeps the dear one, which the old
    /// oldest-modified-first walk could not do — it would have taken the dear
    /// frame purely for being written first (docs/06 §5.3).
    #[test]
    fn the_cap_evicts_the_cheap_frame_and_keeps_the_dear_one() {
        let dir = tempfile::tempdir().unwrap();
        let one_size = {
            let mut probe = DiskCache::open(dir.path().join("probe"), u64::MAX);
            probe.store(parked(1, 16, 16, &frame(16, 16, 3), 8));
            probe.used_bytes()
        };
        let cap = one_size * 2 + one_size / 2;
        let mut c = DiskCache::open(dir.path().join("real"), cap);
        // The dear one first, so being oldest is not what saves it.
        c.store(parked(1, 16, 16, &frame(16, 16, 1), 500));
        c.store(parked(2, 16, 16, &frame(16, 16, 2), 1));
        c.store(parked(3, 16, 16, &frame(16, 16, 3), 1));

        assert!(c.used_bytes() <= cap);
        assert!(
            c.contains(1),
            "the frame that cost 500 ms to make is the one worth keeping"
        );
        assert_eq!(c.len(), 2, "and the cap was actually enforced");
    }

    /// Lowering the cap evicts at once rather than waiting for the next store,
    /// so the space the user just asked to reclaim is really gone.
    #[test]
    fn set_cap_evicts_immediately_when_lowered() {
        let dir = tempfile::tempdir().unwrap();
        let one_size = {
            let mut probe = DiskCache::open(dir.path().join("probe"), u64::MAX);
            probe.store(parked(1, 16, 16, &frame(16, 16, 3), 8));
            probe.used_bytes()
        };
        let mut c = DiskCache::open(dir.path().join("real"), u64::MAX);
        c.store(parked(1, 16, 16, &frame(16, 16, 1), 1));
        c.store(parked(2, 16, 16, &frame(16, 16, 2), 500));
        assert!(c.contains(1) && c.contains(2));

        c.set_cap(one_size + one_size / 2);
        assert!(c.used_bytes() <= one_size + one_size / 2);
        assert!(c.contains(2), "the dear frame survives the squeeze");
        assert!(!c.contains(1));
    }

    /// **The index is what makes the tier cheap to open**, and it must agree with
    /// the folder across a close and a reopen — otherwise the cache either
    /// forgets frames that are taking up room, or promises frames that are gone.
    #[test]
    fn a_reopened_cache_knows_what_it_holds_without_a_walk() {
        let dir = tempfile::tempdir().unwrap();
        let (used, hashes) = {
            let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
            c.store(parked(1, 8, 8, &frame(8, 8, 1), 20));
            c.store(parked(2, 8, 8, &frame(8, 8, 2), 20));
            c.flush_index();
            let mut hashes = c.known_hashes();
            hashes.sort_unstable();
            (c.used_bytes(), hashes)
        };

        let reopened = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        assert_eq!(reopened.used_bytes(), used);
        let mut again = reopened.known_hashes();
        again.sort_unstable();
        assert_eq!(again, hashes);
        assert!(reopened.contains(1) && reopened.contains(2));
    }

    /// A cache written by a build with no index — or one whose index file was
    /// deleted — is rebuilt by walking the folder, so nothing is orphaned
    /// (docs/06 §5.4's "rebuilt by scan if missing or corrupt").
    #[test]
    fn a_missing_index_is_rebuilt_from_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
            c.store(parked(1, 8, 8, &frame(8, 8, 1), 20));
            c.flush_index();
        }
        fs::remove_file(dir.path().join("index.bin")).unwrap();
        let _ = fs::remove_file(dir.path().join("index.log"));

        let mut rebuilt = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        assert!(rebuilt.contains(1), "the frame on disk was found again");
        assert!(rebuilt.used_bytes() > 0);
        assert_eq!(rebuilt.load(1).map(|f| (f.width, f.height)), Some((8, 8)));
    }

    /// A file deleted behind the cache's back (a user emptying the folder, the
    /// operating system reclaiming a cache directory) must not leave the index
    /// promising a frame that is not there: the failed load drops it.
    #[test]
    fn a_frame_deleted_underneath_is_dropped_from_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        c.store(parked(5, 8, 8, &frame(8, 8, 1), 20));
        let hex = format!("{:032x}", 5u128);
        fs::remove_file(
            dir.path()
                .join("frames")
                .join(&hex[..2])
                .join(format!("{hex}.kfr")),
        )
        .unwrap();

        assert!(c.load(5).is_none(), "there is nothing to load");
        assert!(!c.contains(5), "and the index no longer claims otherwise");
        assert_eq!(c.used_bytes(), 0);
    }

    /// Storing the same frame twice is not a second copy — the name is the
    /// content — but it does count as the frame being wanted again.
    #[test]
    fn storing_a_held_frame_again_is_a_use_not_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        c.store(parked(9, 8, 8, &frame(8, 8, 1), 20));
        let used = c.used_bytes();
        c.store(parked(9, 8, 8, &frame(8, 8, 1), 20));
        assert_eq!(c.used_bytes(), used, "content-addressed: one file");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn sidecar_root_sits_beside_the_project() {
        let p = Path::new("D:/edits/montage.lum");
        assert_eq!(
            sidecar_root(p).unwrap(),
            Path::new("D:/edits/montage.lum-cache")
        );
        assert!(sidecar_root(Path::new("/")).is_none());
    }

    #[test]
    fn cache_root_for_with_no_override_matches_sidecar_root() {
        let p = Path::new("D:/edits/montage.lum");
        assert_eq!(cache_root_for(p, None), sidecar_root(p));
        assert_eq!(
            cache_root_for(p, None).unwrap(),
            Path::new("D:/edits/montage.lum-cache")
        );
    }

    #[test]
    fn cache_root_for_is_deterministic() {
        let p = Path::new("D:/edits/montage.lum");
        let over = Path::new("E:/lumit-cache");
        let a = cache_root_for(p, Some(over));
        let b = cache_root_for(p, Some(over));
        assert!(a.is_some());
        assert_eq!(a, b);
    }

    #[test]
    fn cache_root_for_hash_is_stable_across_toolchains() {
        // FNV-1a is a fixed algorithm; this pins the exact folder name for a
        // known path so any change to the hashing (e.g. a slip back to
        // DefaultHasher) is caught here rather than silently orphaning every
        // user's override cache on a Rust toolchain bump.
        let over = Path::new("E:/lumit-cache");
        let root = cache_root_for(Path::new("D:/edits/montage.lum"), Some(over)).unwrap();
        assert_eq!(root, over.join("montage-6fe0182f-cache"));
    }

    #[test]
    fn cache_root_for_does_not_collide_on_same_file_name() {
        let over = Path::new("E:/lumit-cache");
        let a = cache_root_for(Path::new("D:/edits/montage.lum"), Some(over)).unwrap();
        let b = cache_root_for(Path::new("D:/archive/montage.lum"), Some(over)).unwrap();
        assert_ne!(a, b, "same file name in different folders must not collide");
        // Both still sit under the chosen override root, and stay
        // recognisable by the project's file stem.
        assert!(a.starts_with(over));
        assert!(b.starts_with(over));
        assert!(a
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("montage-"));
        assert!(b
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("montage-"));
    }
}
