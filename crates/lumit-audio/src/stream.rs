//! A file's sound held in blocks, a few of them at a time (docs/09-AUDIO.md
//! §2).
//!
//! # In plain terms
//!
//! Decoded sound is big: a minute of stereo is 23 MB, so the ten hours of
//! footage a long cut names would be 14 GB if every file were decoded whole,
//! which is what used to happen. Nearly all of that is sound nobody is about
//! to hear. So a file's sound is cut into two-second **blocks**, a block is
//! decoded when something is about to need it, and the blocks of every file
//! share one byte budget: when it is full the block that has gone longest
//! unwanted is dropped.
//!
//! Three kinds of caller read a [`Source`], and they differ in one thing,
//! which is whether they may wait:
//!
//! - **The audio callback** may not. It reads the blocks that are there
//!   ([`Source::read_runs`]) and is told how many frames were not, which it
//!   plays as silence. It takes no lock it could wait on, allocates nothing
//!   and never frees a block: a block is only ever dropped by the thread that
//!   decided to drop it.
//! - **The thread that fills ahead** ([`crate::mix::MixPlan::fill_step`])
//!   keeps the blocks around the playhead decoded, nearest first.
//! - **Everything else** (an export, a rack being baked, a scrub) asks for a
//!   stretch and waits for it ([`Source::read`]).
//!
//! A [`Source`] can also be a whole buffer already in memory, which is what a
//! baked rack's output is and what the tests mix. It reads the same way and
//! is never dropped.

use lumit_media::audio::AudioReader;
use lumit_media::AudioBuffer;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};

/// How long one block is, in seconds. Whole seconds, because a reader lands
/// on a whole second (see [`AudioReader`]) and a block that starts on one is
/// decoded with no walk to it. Two of them is 768 KB at 48 kHz: small enough
/// that the budget is spent near the playhead, big enough that a playing clip
/// asks for a block every two seconds and not every few milliseconds.
pub const BLOCK_SECONDS: usize = 2;

/// What every file's decoded blocks may hold between them, by default: about
/// twenty-two minutes of sound.
pub const DEFAULT_BUDGET_BYTES: usize = 512 * 1024 * 1024;

/// Blocks to a table, and tables to a source. A source's tables are made as
/// its blocks are first asked for, so a short file costs one.
const TABLE_BLOCKS: usize = 256;
// ponytail: 64 tables of 256 two-second blocks is a little over nine hours of
// one file. Sound past that is silence. Raise the count if a single source
// ever runs longer; a table is ten kilobytes and only made when reached.
const TABLES: usize = 64;

/// How many files are kept open for decoding at once. A row plays one file at
/// a time and a crossfade two, so this covers a handful of rows without
/// opening a file again for every block.
const OPEN_READERS: usize = 12;

/// How many sources the pool remembers before it forgets the ones holding
/// nothing.
const KEPT_SOURCES: usize = 1024;

/// One block of one file: interleaved stereo, as long as a block is except at
/// the end of the file, and empty past it.
pub struct Block {
    pub samples: Vec<f32>,
    /// Whether these are the very samples a decode of the whole file holds
    /// here, or the same sound a landing decoded (see [`AudioReader::read`]).
    pub exact: bool,
}

struct Slot {
    held: RwLock<Option<Arc<Block>>>,
    /// The same answer as `held.is_some()`, readable without the lock.
    there: AtomicBool,
    /// Until when this block is wanted, on [`now`]'s clock.
    used: AtomicU64,
}

impl Slot {
    fn new() -> Self {
        Self {
            held: RwLock::new(None),
            there: AtomicBool::new(false),
            used: AtomicU64::new(0),
        }
    }
}

/// A file's blocks.
struct File {
    path: PathBuf,
    pool: Weak<Pool>,
    /// Names this source among the pool's open readers.
    id: u64,
    block_frames: usize,
    tables: [OnceLock<Box<[Slot]>>; TABLES],
    /// The file would not decode. It is silence, and nothing waits for it.
    failed: AtomicBool,
    /// The file's length in frames once a decode has found it, `u64::MAX`
    /// until then.
    total: AtomicU64,
}

impl File {
    /// The slot for `block` if its table has been made. Lock-free.
    fn slot(&self, block: usize) -> Option<&Slot> {
        self.tables
            .get(block / TABLE_BLOCKS)?
            .get()?
            .get(block % TABLE_BLOCKS)
    }

    /// The slot for `block`, its table made if this is the first ask. `None`
    /// past the last table.
    fn slot_made(&self, block: usize) -> Option<&Slot> {
        self.tables
            .get(block / TABLE_BLOCKS)?
            .get_or_init(|| (0..TABLE_BLOCKS).map(|_| Slot::new()).collect())
            .get(block % TABLE_BLOCKS)
    }

    fn total(&self) -> Option<usize> {
        let total = self.total.load(Ordering::Relaxed);
        (total != u64::MAX).then_some(total as usize)
    }

    /// Whether there is nothing to wait for at `block`: it is decoded, or it
    /// never will be.
    fn settled(&self, block: usize) -> bool {
        self.failed.load(Ordering::Relaxed)
            || block / TABLE_BLOCKS >= TABLES
            || self
                .total()
                .is_some_and(|total| block * self.block_frames >= total)
            || self
                .slot(block)
                .is_some_and(|s| s.there.load(Ordering::Relaxed))
    }
}

enum Kind {
    Whole(Arc<AudioBuffer>),
    File(Box<File>),
}

/// One stereo source at one rate: a whole buffer, or a file read in blocks.
pub struct Source {
    rate: u32,
    kind: Kind,
}

impl Source {
    /// A buffer already in memory, read as it stands.
    #[must_use]
    pub fn whole(buffer: Arc<AudioBuffer>) -> Arc<Source> {
        Arc::new(Source {
            rate: buffer.rate,
            kind: Kind::Whole(buffer),
        })
    }

    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Whether this is a file read in blocks, and not a buffer in memory.
    #[must_use]
    pub fn is_file(&self) -> bool {
        matches!(self.kind, Kind::File(_))
    }

    /// The file this source reads, for one that reads a file.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.kind {
            Kind::File(file) => Some(&file.path),
            Kind::Whole(_) => None,
        }
    }

    /// The source's length in frames, where it is known without decoding: a
    /// buffer's always is, a file's once something has decoded to its end.
    #[must_use]
    pub fn frames(&self) -> Option<usize> {
        match &self.kind {
            Kind::Whole(buffer) => Some(buffer.samples.len() / 2),
            Kind::File(file) => file.total(),
        }
    }

    /// The source's length in frames, decoding the end of a file to find it.
    /// Waits. Nought for a file that will not decode.
    #[must_use]
    pub fn frames_exactly(&self) -> usize {
        if let Some(frames) = self.frames() {
            return frames;
        }
        let Kind::File(file) = &self.kind else {
            return 0;
        };
        let Some(pool) = file.pool.upgrade() else {
            return 0;
        };
        let Some(mut reader) = pool.reader(file, self.rate, 0, false) else {
            return 0;
        };
        let total = match reader.length() {
            Ok(total) => total,
            Err(_) => {
                file.failed.store(true, Ordering::Relaxed);
                0
            }
        };
        file.total.store(total, Ordering::Relaxed);
        pool.put_back(file.id, reader);
        total as usize
    }

    /// **The callback's read.** Hands `f` each run of interleaved samples that
    /// is in memory inside `[start, start + frames)`, with how many frames
    /// into the stretch the run begins, and answers how many frames were due
    /// and not there. Never waits, allocates or frees.
    pub fn read_runs(
        &self,
        start: usize,
        frames: usize,
        mut f: impl FnMut(usize, &[f32]),
    ) -> usize {
        match &self.kind {
            Kind::Whole(buffer) => {
                let len = buffer.samples.len() & !1;
                let from = start.saturating_mul(2).min(len);
                let to = start.saturating_add(frames).saturating_mul(2).min(len);
                if to > from {
                    f(0, &buffer.samples[from..to]);
                }
                0
            }
            Kind::File(file) => {
                let end = match file.total() {
                    Some(total) => start.saturating_add(frames).min(total),
                    None => start.saturating_add(frames),
                };
                if file.failed.load(Ordering::Relaxed) || file.block_frames == 0 {
                    return 0;
                }
                let (mut at, mut missed) = (start, 0usize);
                while at < end {
                    let block = at / file.block_frames;
                    let within = at % file.block_frames;
                    let n = (file.block_frames - within).min(end - at);
                    if block / TABLE_BLOCKS >= TABLES {
                        break;
                    }
                    // `try_read` cannot wait. It misses only while a block is
                    // being put in or taken out, and a block being put in was
                    // not there a moment ago either.
                    let guard = file.slot(block).and_then(|s| s.held.try_read());
                    match guard.as_ref().and_then(|g| g.as_ref()) {
                        Some(held) => {
                            let len = held.samples.len() & !1;
                            let from = (within * 2).min(len);
                            let to = ((within + n) * 2).min(len);
                            if to > from {
                                f(at - start, &held.samples[from..to]);
                            }
                        }
                        None => missed += n,
                    }
                    at += n;
                }
                missed
            }
        }
    }

    /// One frame, or `None` where there is none in memory. Not for the
    /// callback: it waits for a block that is being put in.
    #[must_use]
    pub fn frame(&self, index: usize) -> Option<(f32, f32)> {
        match &self.kind {
            Kind::Whole(buffer) => Some((
                *buffer.samples.get(index * 2)?,
                *buffer.samples.get(index * 2 + 1)?,
            )),
            Kind::File(file) => {
                if file.block_frames == 0 {
                    return None;
                }
                let held = file.slot(index / file.block_frames)?.held.read();
                let samples = &held.as_ref()?.samples;
                let at = (index % file.block_frames) * 2;
                Some((*samples.get(at)?, *samples.get(at + 1)?))
            }
        }
    }

    /// **The waiting read.** `[start, start + frames)` as interleaved samples,
    /// shorter where the source ends first. Blocks that are not in memory are
    /// decoded on this thread.
    ///
    /// `exact` is an export's ask: every sample the one a decode of the whole
    /// file holds there, whatever that costs. Without it a block a landing
    /// decoded will do (see [`AudioReader::read`]).
    #[must_use]
    pub fn read(self: &Arc<Self>, start: usize, frames: usize, exact: bool) -> Vec<f32> {
        match &self.kind {
            Kind::Whole(buffer) => {
                let len = buffer.samples.len() & !1;
                let from = start.saturating_mul(2).min(len);
                let to = start.saturating_add(frames).saturating_mul(2).min(len);
                buffer.samples[from..to].to_vec()
            }
            Kind::File(file) => {
                let mut out = Vec::with_capacity(frames.min(1 << 24) * 2);
                let stamp = now();
                let end = start.saturating_add(frames);
                let mut at = start;
                while at < end && file.block_frames > 0 {
                    let block = at / file.block_frames;
                    let within = at % file.block_frames;
                    let n = (file.block_frames - within).min(end - at);
                    let Some(held) = self.block(block, exact, stamp) else {
                        break;
                    };
                    let len = held.samples.len() & !1;
                    let from = (within * 2).min(len);
                    let to = ((within + n) * 2).min(len);
                    out.extend_from_slice(&held.samples[from..to]);
                    if to - from < n * 2 {
                        break; // the file ended inside this block
                    }
                    at += n;
                }
                out
            }
        }
    }

    /// The blocks `[start, start + frames)` touches, for a file. `None` for a
    /// whole buffer, which has none to fetch.
    fn blocks(&self, start: usize, frames: usize) -> Option<std::ops::Range<usize>> {
        let Kind::File(file) = &self.kind else {
            return None;
        };
        if frames == 0 || file.block_frames == 0 {
            return None;
        }
        Some(start / file.block_frames..(start + frames - 1) / file.block_frames + 1)
    }

    /// Mark the blocks under `[start, start + frames)` as wanted at `stamp`
    /// and answer the first of them that is not in memory, if any, with how
    /// many frames into the stretch it begins.
    pub(crate) fn want(&self, start: usize, frames: usize, stamp: u64) -> Option<(usize, usize)> {
        let Kind::File(file) = &self.kind else {
            return None;
        };
        let mut first = None;
        for block in self.blocks(start, frames)? {
            if let Some(slot) = file.slot_made(block) {
                slot.used.store(stamp, Ordering::Relaxed);
            }
            if first.is_none() && !file.settled(block) {
                first = Some((block, (block * file.block_frames).saturating_sub(start)));
            }
        }
        first
    }

    /// Block `index` of a file, decoded now if it is not in memory. Waits.
    /// `None` for a buffer, a file that will not decode, or a block past the
    /// last table.
    pub(crate) fn block(
        self: &Arc<Self>,
        index: usize,
        exact: bool,
        stamp: u64,
    ) -> Option<Arc<Block>> {
        let Kind::File(file) = &self.kind else {
            return None;
        };
        let slot = file.slot_made(index)?;
        slot.used.store(stamp, Ordering::Relaxed);
        if let Some(held) = slot.held.read().as_ref() {
            if held.exact || !exact {
                return Some(Arc::clone(held));
            }
        }
        if file.failed.load(Ordering::Relaxed) {
            return None;
        }
        let Some(pool) = file.pool.upgrade() else {
            // The pool has gone, so nothing will ever decode this: silence,
            // and nobody left waiting for it.
            file.failed.store(true, Ordering::Relaxed);
            return None;
        };
        let start = (index * file.block_frames) as u64;
        let mut reader = pool.reader(file, self.rate, start, exact)?;
        let block = match reader.read(start, file.block_frames, exact) {
            Ok((samples, exact)) => Arc::new(Block { samples, exact }),
            Err(_) => {
                // A file that fails part way is not asked again: the rest of
                // it is silence, as a file that would not open is.
                file.failed.store(true, Ordering::Relaxed);
                return None;
            }
        };
        if let Some(total) = reader.total() {
            file.total.store(total, Ordering::Relaxed);
        }
        pool.put_back(file.id, reader);
        pool.keep(self, index, Arc::clone(&block), stamp);
        Some(block)
    }
}

/// A stamp for "wanted now": milliseconds on a clock that starts with the
/// process. A block is stamped with the moment it is wanted until, and the
/// pool drops the block whose moment is furthest past.
///
/// A clock and not a counter, so that the thread filling ahead of the
/// playhead can want its blocks a little into the future
/// ([`crate::mix::MixPlan::fill_step`]) and an export reading through the same
/// pool at full speed does not push them out between two of its passes. It
/// decides only which block is dropped, never what a block holds, so nothing
/// that is rendered depends on it.
#[must_use]
pub fn now() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX - 1) + 1
}

/// The process's own pool: every file the mixer, the export and the previews
/// read shares its budget.
#[must_use]
pub fn pool() -> &'static Arc<Pool> {
    static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(DEFAULT_BUDGET_BYTES))
}

struct OpenReader {
    source: u64,
    reader: AudioReader,
    used: u64,
}

#[derive(Default)]
struct PoolState {
    /// Every file anybody has asked for, by path and rate. Compacted when it
    /// passes [`KEPT_SOURCES`]: a source nothing else holds and that holds no
    /// block is forgotten, and made again for nothing if it is asked for.
    sources: HashMap<(PathBuf, u32), Arc<Source>>,
    /// The blocks in memory, for choosing which to drop.
    held: Vec<(Arc<Source>, usize)>,
    readers: Vec<OpenReader>,
    next_id: u64,
}

/// The decoded blocks of every file, under one byte budget.
pub struct Pool {
    budget: usize,
    bytes: AtomicUsize,
    state: Mutex<PoolState>,
}

impl Pool {
    #[must_use]
    pub fn new(budget_bytes: usize) -> Arc<Pool> {
        Arc::new(Pool {
            budget: budget_bytes,
            bytes: AtomicUsize::new(0),
            state: Mutex::new(PoolState::default()),
        })
    }

    /// The sound of `path` at `rate`, the same [`Source`] for every asker so
    /// two clips of one file share its blocks. Opens nothing: a file that is
    /// missing or silent is found out when a block of it is first wanted.
    ///
    /// ponytail: keyed by path, so a file replaced on disk under a running
    /// Lumit keeps the blocks already decoded from the old one until they are
    /// dropped. Key by the file's fingerprint if overwriting a clip in place
    /// has to be heard at once.
    #[must_use]
    pub fn source(self: &Arc<Self>, path: &Path, rate: u32) -> Arc<Source> {
        let mut state = self.state.lock();
        if let Some(source) = state.sources.get(&(path.to_path_buf(), rate)) {
            return Arc::clone(source);
        }
        if state.sources.len() >= KEPT_SOURCES {
            let holding: std::collections::HashSet<*const Source> =
                state.held.iter().map(|(s, _)| Arc::as_ptr(s)).collect();
            state
                .sources
                .retain(|_, s| Arc::strong_count(s) > 1 || holding.contains(&Arc::as_ptr(s)));
        }
        state.next_id += 1;
        let source = Arc::new(Source {
            rate,
            kind: Kind::File(Box::new(File {
                path: path.to_path_buf(),
                pool: Arc::downgrade(self),
                id: state.next_id,
                block_frames: BLOCK_SECONDS * rate as usize,
                tables: std::array::from_fn(|_| OnceLock::new()),
                failed: AtomicBool::new(false),
                total: AtomicU64::new(u64::MAX),
            })),
        });
        state
            .sources
            .insert((path.to_path_buf(), rate), Arc::clone(&source));
        source
    }

    /// How many bytes of decoded blocks are in memory now.
    #[must_use]
    pub fn resident_bytes(&self) -> usize {
        self.bytes.load(Ordering::Relaxed)
    }

    /// An open reader for `file`: one standing at `start` if there is one,
    /// else one that reaches it by decoding on, else any when a landing will
    /// do, else a fresh one. `None` when the file will not open, which marks
    /// it failed.
    fn reader(&self, file: &File, rate: u32, start: u64, exact: bool) -> Option<AudioReader> {
        let taken = {
            let mut state = self.state.lock();
            let of_file = |r: &OpenReader| r.source == file.id;
            let at = state
                .readers
                .iter()
                .position(|r| of_file(r) && r.reader.reaches(start, exact) == Some(0))
                .or_else(|| {
                    state
                        .readers
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| of_file(r))
                        .filter_map(|(i, r)| Some((r.reader.reaches(start, exact)?, i)))
                        .min()
                        .map(|(_, i)| i)
                })
                .or_else(|| {
                    // A landing does not care where the reader stood. An exact
                    // read does: one that cannot walk there starts again from
                    // the top, and would take the playing clip's reader with
                    // it.
                    (!exact)
                        .then(|| state.readers.iter().position(of_file))
                        .flatten()
                });
            at.map(|i| state.readers.swap_remove(i).reader)
        };
        if taken.is_some() {
            return taken;
        }
        // Opened with nothing held: it is a file access and an FFmpeg call.
        match AudioReader::open(&file.path, rate) {
            Ok(mut reader) => {
                if let Some(total) = file.total() {
                    reader.set_total(total as u64);
                }
                Some(reader)
            }
            Err(_) => {
                file.failed.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    fn put_back(&self, source: u64, reader: AudioReader) {
        let mut state = self.state.lock();
        state.readers.push(OpenReader {
            source,
            reader,
            used: now(),
        });
        if state.readers.len() > OPEN_READERS {
            if let Some(stalest) = state
                .readers
                .iter()
                .enumerate()
                .min_by_key(|(_, r)| r.used)
                .map(|(i, _)| i)
            {
                state.readers.swap_remove(stalest);
            }
        }
    }

    /// Put a decoded block in its slot and, if that takes the pool over its
    /// budget, drop the blocks that have gone longest unwanted. A block wanted
    /// until `stamp` or later is never dropped to make room: the caller is
    /// about to read those, and so is whoever stamped the later ones.
    fn keep(&self, source: &Arc<Source>, index: usize, block: Arc<Block>, stamp: u64) {
        let Kind::File(file) = &source.kind else {
            return;
        };
        let Some(slot) = file.slot_made(index) else {
            return;
        };
        let bytes = block.samples.len() * std::mem::size_of::<f32>();
        let mut state = self.state.lock();
        let was = slot.held.write().replace(block);
        slot.there.store(true, Ordering::Relaxed);
        match was {
            Some(old) => {
                self.bytes.fetch_sub(
                    old.samples.len() * std::mem::size_of::<f32>(),
                    Ordering::Relaxed,
                );
            }
            None => state.held.push((Arc::clone(source), index)),
        }
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        if self.bytes.load(Ordering::Relaxed) <= self.budget {
            return;
        }
        // Over: drop the stalest down to seven eighths, so the next block in
        // does not have to do this again.
        let used = |(s, b): &(Arc<Source>, usize)| match &s.kind {
            Kind::File(f) => f
                .slot(*b)
                .map_or(0, |slot| slot.used.load(Ordering::Relaxed)),
            Kind::Whole(_) => 0,
        };
        state.held.sort_by_key(used);
        let floor = self.budget / 8 * 7;
        let mut kept = Vec::with_capacity(state.held.len());
        for entry in std::mem::take(&mut state.held) {
            if self.bytes.load(Ordering::Relaxed) > floor && used(&entry) < stamp {
                self.drop_block(&entry.0, entry.1);
            } else {
                kept.push(entry);
            }
        }
        state.held = kept;
    }

    /// Take one block out of memory. The write lock waits for a callback that
    /// is reading it, so the samples are freed here and never there.
    fn drop_block(&self, source: &Source, index: usize) {
        let Kind::File(file) = &source.kind else {
            return;
        };
        let Some(slot) = file.slot(index) else {
            return;
        };
        slot.there.store(false, Ordering::Relaxed);
        if let Some(old) = slot.held.write().take() {
            self.bytes.fetch_sub(
                old.samples.len() * std::mem::size_of::<f32>(),
                Ordering::Relaxed,
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;

    /// A folder of this test's own under the system's temporary one, removed
    /// when the guard goes.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("lumit-audio-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A WAV of `seconds` of a ramp that never repeats, so a block out of
    /// place cannot pass for the right one: frame `n` holds `n` on the left
    /// and its negative on the right, scaled into range.
    pub(crate) fn ramp_wav(dir: &Path, name: &str, rate: u32, seconds: usize) -> PathBuf {
        let frames = rate as usize * seconds;
        let mut bytes = Vec::with_capacity(44 + frames * 4);
        let data = (frames * 4) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&2u16.to_le_bytes()); // stereo
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for n in 0..frames {
            let v = ((n * 7919) % 30_000) as i16 - 15_000;
            bytes.extend_from_slice(&v.to_le_bytes());
            bytes.extend_from_slice(&(-v).to_le_bytes());
        }
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// **The budget holds and nothing is lost by it**: blocks past the budget
    /// push the stalest out, a block that was dropped decodes again to the
    /// same samples, and the waiting read is the whole-file decode whatever
    /// was in memory when it was asked.
    #[test]
    fn the_pool_drops_the_stalest_block_and_reads_the_same_sound_back() {
        const RATE: u32 = 8_000;
        let dir = Scratch::new("pool");
        let path = ramp_wav(&dir.0, "ramp.wav", RATE, 21);
        let whole = lumit_media::audio::decode_all(&path, RATE).unwrap();
        let block_bytes = BLOCK_SECONDS * RATE as usize * 2 * 4;

        // Room for three blocks of the eleven the file has.
        let pool = Pool::new(block_bytes * 3);
        let source = pool.source(&path, RATE);
        assert!(Arc::ptr_eq(&source, &pool.source(&path, RATE)));
        assert_eq!(source.frames(), None, "nothing is opened to make a source");

        // The callback's read of a block that is not there: all of it missed,
        // none of it sounded, and nothing decoded on its behalf.
        let mut heard = 0;
        let missed = source.read_runs(0, 100, |_, run| heard += run.len());
        assert_eq!((missed, heard), (100, 0));
        assert_eq!(pool.resident_bytes(), 0);

        for block in 0..6 {
            // Each wanted a moment after the one before.
            assert!(source.block(block, false, block as u64 + 1).is_some());
            assert!(pool.resident_bytes() <= block_bytes * 3, "block {block}");
        }
        // The first blocks went to make room, and the newest are there.
        let there = |b: usize| source.read_runs(b * 2 * RATE as usize, 1, |_, _| {}) == 0;
        assert!(!there(0) && !there(1), "the stalest were dropped");
        assert!(there(5), "the newest is in memory");

        // A stretch across dropped and kept blocks, and off the end.
        let from = 2 * RATE as usize - 5;
        let got = source.read(from, 9 * 2 * RATE as usize, true);
        assert!(got == whole.samples[from * 2..(from + 9 * 2 * RATE as usize) * 2]);
        let tail = source.read(20 * RATE as usize, 5 * RATE as usize, true);
        assert!(tail == whole.samples[20 * RATE as usize * 2..]);
        assert_eq!(source.frames(), Some(whole.frames()));
        assert_eq!(source.frames_exactly(), whole.frames());

        // The callback's read of what is there now is those same samples, and
        // a stretch that runs off the end of the file misses nothing.
        let at = 20 * RATE as usize;
        let mut run_of = Vec::new();
        let missed = source.read_runs(at, 4 * RATE as usize, |off, run| {
            assert_eq!(off, 0);
            run_of.extend_from_slice(run);
        });
        assert_eq!(missed, 0, "past the end is silence, not sound that is late");
        assert!(run_of == whole.samples[at * 2..]);

        // A file that is not one is silence that nothing waits for.
        let gone = pool.source(Path::new("/definitely/not/a/file.wav"), RATE);
        assert!(gone.block(0, false, now()).is_none());
        assert_eq!(gone.read_runs(0, 100, |_, _| {}), 0);
        assert!(gone.read(0, 100, true).is_empty());
    }
}
