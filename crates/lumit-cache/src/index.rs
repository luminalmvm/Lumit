//! The disk tier's index (docs/06-RENDER-PIPELINE.md §5.4): what is parked, how
//! big it is, what it cost to make, and when it was last wanted.
//!
//! # In plain terms
//!
//! Without an index the disk cache can only answer questions by walking the
//! folder: "is this frame here" is a file-exists check, "how much space am I
//! using" is a recursive walk at start-up, and "what should I delete" can only
//! sort by the one thing the filesystem remembers — the modification time. So the
//! bottom tier evicted oldest-first while the two tiers above it evicted by the
//! spec's actual policy: prefer whatever is *stale, large and cheap to make
//! again*. This is the small amount of bookkeeping that lets the disk tier use
//! the same rule.
//!
//! ## Why not SQLite, which the spec names
//!
//! docs/06 §5.4 says `index.db`, SQLite. This is a flat map of a few tens of
//! thousands of fixed-size rows that is read once at start-up and otherwise lives
//! in memory; SQLite would bring a C dependency into an engine crate to store it,
//! and the media frame index (docs/10 §3) already sets the house precedent of a
//! plain binary sidecar. Recorded as a deviation rather than a silent choice.
//!
//! ## Crash safety, which is the only interesting part
//!
//! A snapshot rewritten on every change would rewrite megabytes per frame; a
//! snapshot rewritten only occasionally loses whatever happened since — and the
//! frames it loses are *worse than forgotten*, because the files are still on
//! disk taking up room that nothing knows to reclaim. So this is the usual
//! log-and-snapshot arrangement:
//!
//! - `index.bin` — the snapshot: every entry, rewritten atomically now and then.
//! - `index.log` — one fixed-size record appended per change since that snapshot.
//!
//! Opening reads the snapshot and replays the log over it. A record half-written
//! by a crash is a partial trailing record, which is discarded by length — that
//! is the whole point of fixed-size records. If either file is missing or
//! unreadable the index reports itself empty, and the cache rebuilds it by
//! walking the folder, exactly as the spec requires ("rebuilt by scan if missing
//! or corrupt").

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Snapshot magic and version.
const SNAPSHOT_MAGIC: [u8; 4] = *b"KIX1";
/// One entry as stored in the snapshot: hash, bytes, cost, quality, last use.
const ENTRY_LEN: usize = 16 + 8 + 4 + 2 + 8;
/// One log record: a kind byte, then an entry.
const RECORD_LEN: usize = 1 + ENTRY_LEN;
/// Log record kinds.
const KIND_PUT: u8 = 1;
const KIND_REMOVE: u8 = 2;
const KIND_TOUCH: u8 = 3;

/// How many log records may accumulate before the snapshot is rewritten and the
/// log truncated. At 41 bytes a record this bounds the log to a few hundred
/// kilobytes, and bounds replay at open to a few thousand records.
const LOG_COMPACT_AFTER: u64 = 8192;

/// What the index knows about one parked frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    /// The file's size on disk (compressed, header included).
    pub bytes: u64,
    /// What the frame cost to render, in milliseconds — the "cheap to recompute"
    /// half of the eviction policy. Never zero, so the score cannot divide by it.
    pub cost_ms: u32,
    /// The preview scale it was made at, in thousandths (the spec's "quality"
    /// column). Not part of the frame's name — the name already covers quality —
    /// but useful to a human reading the index, and cheap to carry.
    pub scale_q: u16,
    /// When it was last stored or served, in seconds since the Unix epoch.
    pub last_used: u64,
}

/// Seconds since the Unix epoch, or 0 on a machine whose clock predates it.
/// Never panics: an engine crate does not unwrap a clock.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The disk tier's index. Held by [`crate::disk::DiskCache`] on its IO thread.
pub struct FrameIndex {
    root: PathBuf,
    entries: HashMap<u128, IndexEntry>,
    /// Records appended since the last snapshot.
    logged: u64,
    /// Total bytes across [`Self::entries`], maintained rather than summed.
    used_bytes: u64,
}

impl FrameIndex {
    /// Open the index under `root`, replaying anything logged since its last
    /// snapshot. Reports itself empty when there is nothing readable — the
    /// caller then rebuilds by scan.
    pub fn open(root: PathBuf) -> Self {
        let mut index = Self {
            root,
            entries: HashMap::new(),
            logged: 0,
            used_bytes: 0,
        };
        index.load_snapshot();
        index.replay_log();
        index
    }

    fn snapshot_path(&self) -> PathBuf {
        self.root.join("index.bin")
    }

    fn log_path(&self) -> PathBuf {
        self.root.join("index.log")
    }

    fn load_snapshot(&mut self) {
        let mut bytes = Vec::new();
        if File::open(self.snapshot_path())
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .is_err()
        {
            return;
        }
        if bytes.len() < 12 || bytes[..4] != SNAPSHOT_MAGIC {
            return; // not ours, or truncated past use: rebuild by scan
        }
        let count = u64::from_le_bytes(word8(&bytes[4..12]));
        let mut at = 12;
        for _ in 0..count {
            let Some(chunk) = bytes.get(at..at + ENTRY_LEN) else {
                break; // truncated tail: keep what parsed, the scan fills the rest
            };
            let (hash, entry) = decode_entry(chunk);
            self.admit(hash, entry);
            at += ENTRY_LEN;
        }
    }

    fn replay_log(&mut self) {
        let mut bytes = Vec::new();
        if File::open(self.log_path())
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .is_err()
        {
            return;
        }
        // Whole records only: a record half-written when the process stopped is
        // the trailing remainder, and is dropped.
        for record in bytes.chunks_exact(RECORD_LEN) {
            let (hash, entry) = decode_entry(&record[1..]);
            match record[0] {
                KIND_PUT => self.admit(hash, entry),
                KIND_REMOVE => self.forget(hash),
                KIND_TOUCH => {
                    if let Some(held) = self.entries.get_mut(&hash) {
                        held.last_used = entry.last_used;
                    }
                }
                _ => {}
            }
            self.logged += 1;
        }
    }

    /// Insert into the map, keeping the byte total straight.
    fn admit(&mut self, hash: u128, entry: IndexEntry) {
        if let Some(old) = self.entries.insert(hash, entry) {
            self.used_bytes = self.used_bytes.saturating_sub(old.bytes);
        }
        self.used_bytes = self.used_bytes.saturating_add(entry.bytes);
    }

    fn forget(&mut self, hash: u128) {
        if let Some(old) = self.entries.remove(&hash) {
            self.used_bytes = self.used_bytes.saturating_sub(old.bytes);
        }
    }

    /// Append one record. Best-effort: a log that cannot be written costs the
    /// index its crash safety, not its correctness, and the next snapshot fixes
    /// it. Compacts once the log has grown past [`LOG_COMPACT_AFTER`].
    fn log(&mut self, kind: u8, hash: u128, entry: IndexEntry) {
        let mut record = [0u8; RECORD_LEN];
        record[0] = kind;
        encode_entry(&mut record[1..], hash, &entry);
        let appended = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
            .and_then(|mut f| f.write_all(&record));
        if appended.is_ok() {
            self.logged += 1;
        }
        if self.logged >= LOG_COMPACT_AFTER {
            self.write_snapshot();
        }
    }

    /// Record a newly parked frame.
    pub fn put(&mut self, hash: u128, entry: IndexEntry) {
        self.admit(hash, entry);
        self.log(KIND_PUT, hash, entry);
    }

    /// Record that a frame is gone.
    pub fn remove(&mut self, hash: u128) {
        let entry = self.entries.get(&hash).copied();
        self.forget(hash);
        if let Some(entry) = entry {
            self.log(KIND_REMOVE, hash, entry);
        }
    }

    /// Record that a frame was wanted, so it stops looking stale.
    ///
    /// Only written when the recorded time actually moves by a useful amount: a
    /// scrub can ask for the same frame many times a second, and logging each of
    /// those would compact the snapshot repeatedly to record nothing anyone can
    /// use — the eviction score works in seconds.
    pub fn touch(&mut self, hash: u128, now: u64) {
        let Some(entry) = self.entries.get_mut(&hash) else {
            return;
        };
        if now.saturating_sub(entry.last_used) < TOUCH_RESOLUTION_SECS {
            return;
        }
        entry.last_used = now;
        let entry = *entry;
        self.log(KIND_TOUCH, hash, entry);
    }

    #[must_use]
    pub fn contains(&self, hash: u128) -> bool {
        self.entries.contains_key(&hash)
    }

    /// Test inspection hook; production code goes through [`Self::contains`]
    /// and the eviction paths.
    #[cfg(test)]
    fn get(&self, hash: u128) -> Option<IndexEntry> {
        self.entries.get(&hash).copied()
    }

    #[must_use]
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every parked hash — what the cache bar's "on disk" answer is built from.
    pub fn hashes(&self) -> impl Iterator<Item = u128> + '_ {
        self.entries.keys().copied()
    }

    /// Which entries to delete, worst-scoring first, to bring the total from
    /// where it is down to `cap`. Empty when it already fits.
    ///
    /// The score is **stale × large ÷ cheap-to-remake**, the same shape
    /// [`crate::ByteLru`] uses for the tiers above, so the ladder evicts by one
    /// rule from top to bottom (docs/06 §5.3). Higher means evict sooner.
    ///
    /// A full sort rather than a heap: this runs when the cache is over its cap,
    /// which is once per store at worst and never on a render thread, and a sort
    /// of a few tens of thousands of rows is microseconds.
    #[must_use]
    pub fn victims(&self, cap: u64, now: u64) -> Vec<u128> {
        if self.used_bytes <= cap {
            return Vec::new();
        }
        let score = |entry: &IndexEntry| {
            let staleness = now.saturating_sub(entry.last_used) as f64;
            // A frame stored this very second is not worth zero: without the
            // floor every fresh entry would score 0 and sort identically, so a
            // cache filled in one burst would evict in map order rather than by
            // size and cost.
            (staleness + 1.0) * entry.bytes as f64 / f64::from(entry.cost_ms.max(1))
        };
        let mut ranked: Vec<(f64, u128, u64)> = self
            .entries
            .iter()
            .map(|(hash, entry)| (score(entry), *hash, entry.bytes))
            .collect();
        // Worst (highest-scoring) first. `total_cmp` rather than `partial_cmp`:
        // no NaN can reach here, and a comparator that cannot fail needs no
        // fallback branch that tests would never cover.
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut freed = 0u64;
        let over = self.used_bytes - cap;
        let mut out = Vec::new();
        for (_, hash, bytes) in ranked {
            if freed >= over {
                break;
            }
            freed = freed.saturating_add(bytes);
            out.push(hash);
        }
        out
    }

    /// Replace the index wholesale, from a walk of the folder — the "rebuilt by
    /// scan" path (docs/06 §5.4), for a first run, a cache written by an older
    /// build, or an index that would not parse. The scan cannot know what a frame
    /// cost to make, so every entry starts at [`SCANNED_COST_MS`].
    pub fn rebuild_from_scan(&mut self, frames_dir: &Path) {
        self.entries.clear();
        self.used_bytes = 0;
        let Ok(shards) = fs::read_dir(frames_dir) else {
            self.write_snapshot();
            return;
        };
        for shard in shards.flatten() {
            let Ok(files) = fs::read_dir(shard.path()) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name();
                let Some(stem) = Path::new(&name).file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(hash) = u128::from_str_radix(stem, 16) else {
                    continue;
                };
                let Ok(meta) = file.metadata() else { continue };
                let last_used = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                self.admit(
                    hash,
                    IndexEntry {
                        bytes: meta.len(),
                        cost_ms: SCANNED_COST_MS,
                        scale_q: 0,
                        last_used,
                    },
                );
            }
        }
        self.write_snapshot();
    }

    /// Write the snapshot and drop the log. Atomic (temp file then rename), so a
    /// crash mid-write leaves the previous snapshot and its log intact.
    pub fn write_snapshot(&mut self) {
        let path = self.snapshot_path();
        let Some(dir) = path.parent() else { return };
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let mut buf = Vec::with_capacity(12 + self.entries.len() * ENTRY_LEN);
        buf.extend_from_slice(&SNAPSHOT_MAGIC);
        buf.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        for (hash, entry) in &self.entries {
            let mut chunk = [0u8; ENTRY_LEN];
            encode_entry(&mut chunk, *hash, entry);
            buf.extend_from_slice(&chunk);
        }
        let tmp = path.with_extension("bin.tmp");
        let written = File::create(&tmp)
            .and_then(|mut f| f.write_all(&buf))
            .and_then(|()| fs::rename(&tmp, &path));
        match written {
            Ok(()) => {
                // The log's records are all in the snapshot now.
                let _ = fs::remove_file(self.log_path());
                self.logged = 0;
            }
            Err(_) => {
                let _ = fs::remove_file(&tmp);
            }
        }
    }
}

/// The cost credited to a frame found by a folder scan. The scan cannot know
/// what it cost to render — the render happened in another session — so this is
/// stated rather than guessed: dear enough that a scanned entry is not thrown out
/// ahead of a trivial one, and no dearer than a frame with real work in it.
pub const SCANNED_COST_MS: u32 = 16;

/// How coarsely a frame's last-use time is recorded. The eviction score works in
/// seconds, so recording finer would log records nobody can use.
const TOUCH_RESOLUTION_SECS: u64 = 30;

fn word8(bytes: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    let n = bytes.len().min(8);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

fn encode_entry(out: &mut [u8], hash: u128, entry: &IndexEntry) {
    if out.len() < ENTRY_LEN {
        return;
    }
    out[..16].copy_from_slice(&hash.to_le_bytes());
    out[16..24].copy_from_slice(&entry.bytes.to_le_bytes());
    out[24..28].copy_from_slice(&entry.cost_ms.to_le_bytes());
    out[28..30].copy_from_slice(&entry.scale_q.to_le_bytes());
    out[30..38].copy_from_slice(&entry.last_used.to_le_bytes());
}

fn decode_entry(bytes: &[u8]) -> (u128, IndexEntry) {
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&bytes[..16]);
    let mut cost = [0u8; 4];
    cost.copy_from_slice(&bytes[24..28]);
    let mut scale = [0u8; 2];
    scale.copy_from_slice(&bytes[28..30]);
    (
        u128::from_le_bytes(hash),
        IndexEntry {
            bytes: u64::from_le_bytes(word8(&bytes[16..24])),
            cost_ms: u32::from_le_bytes(cost),
            scale_q: u16::from_le_bytes(scale),
            last_used: u64::from_le_bytes(word8(&bytes[30..38])),
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn entry(bytes: u64, cost_ms: u32, last_used: u64) -> IndexEntry {
        IndexEntry {
            bytes,
            cost_ms,
            scale_q: 1000,
            last_used,
        }
    }

    /// The index survives being closed and opened again, which is the whole
    /// reason it exists: the alternative is walking the folder at every start-up.
    #[test]
    fn a_snapshot_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = FrameIndex::open(dir.path().to_path_buf());
        assert!(index.is_empty());
        index.put(7, entry(1000, 20, 100));
        index.put(9, entry(2000, 5, 200));
        assert_eq!(index.used_bytes(), 3000);
        index.write_snapshot();

        let reopened = FrameIndex::open(dir.path().to_path_buf());
        assert_eq!(reopened.len(), 2);
        assert_eq!(reopened.used_bytes(), 3000);
        assert_eq!(reopened.get(7), Some(entry(1000, 20, 100)));
        let mut hashes: Vec<u128> = reopened.hashes().collect();
        hashes.sort_unstable();
        assert_eq!(hashes, vec![7, 9]);
    }

    /// **The crash case, which is the only interesting one.** Frames parked
    /// since the last snapshot are recorded in the log, so a session that ends
    /// without a clean close does not leave files on disk that nothing knows
    /// about — space nothing would ever reclaim.
    #[test]
    fn the_log_carries_what_the_snapshot_missed() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut index = FrameIndex::open(dir.path().to_path_buf());
            index.put(1, entry(100, 8, 10));
            index.write_snapshot(); // a clean point
            index.put(2, entry(200, 8, 20));
            index.remove(1);
            // and now the process stops: no second snapshot.
        }

        let recovered = FrameIndex::open(dir.path().to_path_buf());
        assert!(recovered.contains(2), "the parked frame was replayed");
        assert!(!recovered.contains(1), "and the deleted one stayed deleted");
        assert_eq!(recovered.used_bytes(), 200);
    }

    /// A record half-written when the power went is a partial trailing record.
    /// Fixed-size records make that detectable by length alone, so the whole
    /// ones before it still count.
    #[test]
    fn a_torn_final_record_is_discarded_not_believed() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut index = FrameIndex::open(dir.path().to_path_buf());
            index.put(1, entry(100, 8, 10));
            index.put(2, entry(200, 8, 20));
        }
        // Chop the last record in half, as a torn write would.
        let log = dir.path().join("index.log");
        let bytes = fs::read(&log).unwrap();
        fs::write(&log, &bytes[..bytes.len() - RECORD_LEN / 2]).unwrap();

        let recovered = FrameIndex::open(dir.path().to_path_buf());
        assert!(recovered.contains(1));
        assert!(
            !recovered.contains(2),
            "the torn record is not half-believed"
        );
        assert_eq!(recovered.used_bytes(), 100);
    }

    /// An unreadable or foreign snapshot leaves the index empty rather than
    /// wrong, so the cache rebuilds it by scan (docs/06 §5.4).
    #[test]
    fn a_corrupt_snapshot_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("index.bin"), b"not an index at all").unwrap();
        assert!(FrameIndex::open(dir.path().to_path_buf()).is_empty());

        // A future version's magic is not ours to read either.
        fs::write(dir.path().join("index.bin"), b"KIX9\0\0\0\0\0\0\0\0").unwrap();
        assert!(FrameIndex::open(dir.path().to_path_buf()).is_empty());
    }

    /// The point of the whole exercise: the disk tier evicts by the same rule as
    /// the tiers above it — stale × large ÷ cheap-to-remake — rather than by
    /// modification time, which is all the filesystem could tell it.
    #[test]
    fn eviction_prefers_the_stale_large_cheap_frame() {
        let now = 10_000;
        // A fresh folder per case: an index remembers, which is the point of it,
        // so reusing one would carry the previous case's entries into the next.
        let fresh = || {
            let dir = tempfile::tempdir().unwrap();
            let index = FrameIndex::open(dir.path().to_path_buf());
            (dir, index)
        };

        // Same size and staleness, different cost: the cheap one goes.
        let (_d, mut index) = fresh();
        index.put(1, entry(1000, 1, now - 500)); // cheap
        index.put(2, entry(1000, 100, now - 500)); // dear
        assert_eq!(
            index.victims(1000, now),
            vec![1],
            "the cheap one is the bargain to lose"
        );

        // Same cost and staleness, different size: the big one frees more.
        let (_d, mut index) = fresh();
        index.put(3, entry(200, 10, now - 500));
        index.put(4, entry(2000, 10, now - 500));
        assert_eq!(index.victims(2000, now), vec![4]);

        // Same size and cost, different staleness: the stale one goes.
        let (_d, mut index) = fresh();
        index.put(5, entry(1000, 10, now - 5));
        index.put(6, entry(1000, 10, now - 5000));
        assert_eq!(index.victims(1000, now), vec![6]);

        // Nothing to do when it already fits.
        assert!(index.victims(u64::MAX, now).is_empty());
    }

    /// Enough victims to actually get under the cap, not just one.
    #[test]
    fn eviction_names_as_many_victims_as_the_overage_needs() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = FrameIndex::open(dir.path().to_path_buf());
        for i in 0..10u128 {
            index.put(i, entry(100, 10, 1_000 + i as u64));
        }
        assert_eq!(index.used_bytes(), 1000);
        let victims = index.victims(550, 10_000);
        assert!(
            victims.len() >= 5 && victims.len() <= 6,
            "freeing 450 of 100-byte frames takes five, got {}",
            victims.len()
        );
    }

    /// A frame asked for again stops looking stale — but a scrub asking for it
    /// sixty times a second must not write sixty records to say so.
    #[test]
    fn touching_records_use_but_not_every_ask() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = FrameIndex::open(dir.path().to_path_buf());
        index.put(1, entry(100, 10, 1_000));
        let logged = index.logged;
        for _ in 0..50 {
            index.touch(1, 1_001); // a second later: not worth recording
        }
        assert_eq!(index.logged, logged, "a burst of asks logs nothing");
        assert_eq!(index.get(1).unwrap().last_used, 1_000);

        index.touch(1, 1_000 + TOUCH_RESOLUTION_SECS);
        assert_eq!(
            index.get(1).unwrap().last_used,
            1_000 + TOUCH_RESOLUTION_SECS,
            "a real gap does move it"
        );
        assert!(index.logged > logged);

        // Touching something absent is a no-op, not an insertion.
        index.touch(999, 2_000);
        assert!(!index.contains(999));
    }

    /// The scan path, for a first run or a cache from a build that had no index.
    /// It cannot know what a frame cost, so it says so with one stated number
    /// rather than pretending to measure.
    #[test]
    fn a_scan_rebuilds_the_index_from_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        fs::create_dir_all(frames.join("0a")).unwrap();
        let hex = format!("{:032x}", 0x0a_u128);
        fs::write(frames.join("0a").join(format!("{hex}.kfr")), vec![0u8; 512]).unwrap();
        // Something that is not a frame at all is ignored rather than indexed.
        fs::write(frames.join("0a").join("notes.txt"), b"hello").unwrap();

        let mut index = FrameIndex::open(dir.path().to_path_buf());
        index.rebuild_from_scan(&frames);
        assert_eq!(index.len(), 1);
        assert_eq!(index.used_bytes(), 512);
        let entry = index.get(0x0a).unwrap();
        assert_eq!(entry.bytes, 512);
        assert_eq!(entry.cost_ms, SCANNED_COST_MS);

        // And the rebuild is itself snapshotted, so the next open is cheap.
        assert_eq!(FrameIndex::open(dir.path().to_path_buf()).len(), 1);
    }
}
