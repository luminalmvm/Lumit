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

/// The disk tier. All operations are best-effort and silent about IO trouble
/// (a failed write just means the frame is re-rendered later); nothing here
/// panics. One instance is meant to be owned by a single IO thread.
pub struct DiskCache {
    root: PathBuf,
    cap_bytes: u64,
    /// Running total of stored bytes, seeded by a scan on construction and
    /// maintained by store/evict so the cap check never re-walks the folder.
    used_bytes: u64,
}

impl DiskCache {
    /// Open (or prepare) the cache under `root`, scanning any existing
    /// entries so the size accounting starts truthful.
    pub fn open(root: PathBuf, cap_bytes: u64) -> Self {
        let used_bytes = scan_bytes(&root.join("frames"));
        Self {
            root,
            cap_bytes,
            used_bytes,
        }
    }

    /// Bytes currently stored (approximate across crashes; re-seeded by scan).
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    /// Every hash currently present, from a folder walk — seeds the UI's
    /// "on disk" set for the cache bar's blue tier.
    pub fn known_hashes(&self) -> Vec<u128> {
        let mut out = Vec::new();
        let frames = self.root.join("frames");
        let Ok(shards) = fs::read_dir(&frames) else {
            return out;
        };
        for shard in shards.flatten() {
            let Ok(entries) = fs::read_dir(shard.path()) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(stem) = Path::new(&name).file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if let Ok(h) = u128::from_str_radix(stem, 16) {
                    out.push(h);
                }
            }
        }
        out
    }

    fn path_for(&self, hash: u128) -> PathBuf {
        let hex = format!("{hash:032x}");
        self.root
            .join("frames")
            .join(&hex[..2])
            .join(format!("{hex}.kfr"))
    }

    /// Whether a frame is present (fs metadata only; contents unverified —
    /// corruption is discovered and discarded at load).
    pub fn contains(&self, hash: u128) -> bool {
        self.path_for(hash).is_file()
    }

    /// Park a frame on disk (write-behind). Errors are swallowed: a frame
    /// that fails to store is simply re-rendered next time.
    pub fn store(&mut self, hash: u128, width: u32, height: u32, rgba: &[u8]) {
        if rgba.len() != (width as usize) * (height as usize) * 4 {
            return; // malformed input never reaches disk
        }
        let path = self.path_for(hash);
        if path.is_file() {
            return; // content-addressed: already present means identical
        }
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
                self.used_bytes = self.used_bytes.saturating_add(buf.len() as u64);
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
            return None;
        }
        let parsed = parse_kfr(&bytes);
        if parsed.is_none() {
            self.remove(hash);
        }
        parsed
    }

    /// Drop one entry (corruption discard, or external invalidation).
    pub fn remove(&mut self, hash: u128) {
        let path = self.path_for(hash);
        if let Ok(meta) = fs::metadata(&path) {
            if fs::remove_file(&path).is_ok() {
                self.used_bytes = self.used_bytes.saturating_sub(meta.len());
            }
        }
    }

    /// Evict oldest-modified entries until the running total fits the cap
    /// Change the byte cap, evicting oldest-first until within it (Settings →
    /// Performance sets the disk budget).
    pub fn set_cap(&mut self, cap_bytes: u64) {
        self.cap_bytes = cap_bytes;
        self.enforce_cap();
    }

    /// (the spec's cost-aware policy refines this once the index exists —
    /// recompute cost isn't tracked without it).
    fn enforce_cap(&mut self) {
        if self.used_bytes <= self.cap_bytes {
            return;
        }
        let mut entries: Vec<(std::time::SystemTime, PathBuf, u64)> = Vec::new();
        let frames = self.root.join("frames");
        let Ok(shards) = fs::read_dir(&frames) else {
            return;
        };
        for shard in shards.flatten() {
            let Ok(files) = fs::read_dir(shard.path()) else {
                continue;
            };
            for e in files.flatten() {
                if let Ok(meta) = e.metadata() {
                    let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    entries.push((mtime, e.path(), meta.len()));
                }
            }
        }
        entries.sort_by_key(|(mtime, ..)| *mtime);
        for (_, path, len) in entries {
            if self.used_bytes <= self.cap_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                self.used_bytes = self.used_bytes.saturating_sub(len);
            }
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

/// Total bytes under a folder (recursive, best-effort).
fn scan_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            total += scan_bytes(&p);
        } else if let Ok(meta) = e.metadata() {
            total += meta.len();
        }
    }
    total
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

    #[test]
    fn round_trips_a_frame_and_reports_presence() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = DiskCache::open(dir.path().to_path_buf(), u64::MAX);
        let rgba = frame(8, 4, 7);
        assert!(!c.contains(42));
        c.store(42, 8, 4, &rgba);
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
        c.store(7, 4, 4, &frame(4, 4, 1));
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

    #[test]
    fn cap_evicts_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        // A cap small enough for roughly two of the three frames.
        let one_size = {
            let mut probe = DiskCache::open(dir.path().join("probe"), u64::MAX);
            probe.store(1, 16, 16, &frame(16, 16, 3));
            probe.used_bytes()
        };
        let mut c = DiskCache::open(dir.path().join("real"), one_size * 2 + one_size / 2);
        c.store(1, 16, 16, &frame(16, 16, 1));
        // Distinct mtimes even on coarse filesystem clocks.
        std::thread::sleep(std::time::Duration::from_millis(30));
        c.store(2, 16, 16, &frame(16, 16, 2));
        std::thread::sleep(std::time::Duration::from_millis(30));
        c.store(3, 16, 16, &frame(16, 16, 3));
        assert!(c.used_bytes() <= one_size * 2 + one_size / 2);
        assert!(!c.contains(1), "oldest entry evicts first");
        assert!(c.contains(3), "newest entry survives");
    }

    #[test]
    fn set_cap_evicts_immediately_when_lowered() {
        let dir = tempfile::tempdir().unwrap();
        let one_size = {
            let mut probe = DiskCache::open(dir.path().join("probe"), u64::MAX);
            probe.store(1, 16, 16, &frame(16, 16, 3));
            probe.used_bytes()
        };
        let mut c = DiskCache::open(dir.path().join("real"), u64::MAX);
        c.store(1, 16, 16, &frame(16, 16, 1));
        std::thread::sleep(std::time::Duration::from_millis(30));
        c.store(2, 16, 16, &frame(16, 16, 2));
        assert!(c.contains(1) && c.contains(2));
        // Tightening the cap to hold one frame evicts the oldest at once.
        c.set_cap(one_size + one_size / 2);
        assert!(c.used_bytes() <= one_size + one_size / 2);
        assert!(!c.contains(1), "lowering the cap evicts the oldest");
        assert!(c.contains(2));
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
}
