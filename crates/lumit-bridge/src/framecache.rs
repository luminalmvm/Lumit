//! The bridge-side rendered-frame cache — the RAM tier of the three-tier cache
//! (docs/06-RENDER-PIPELINE.md §5.1), and the cross-thread controls for the two
//! tiers the worker owns.
//!
//! # In plain terms
//!
//! Rendering a whole composited comp is the single most expensive thing the
//! Viewer does, so a frame already made should never be made twice. Three stores
//! hold finished frames, each cheaper to reach and smaller than the next:
//! textures still on the graphics card (the worker's renderer owns those), the
//! bytes in this module's map, and files on disk. This module is the middle rung
//! plus the switchboard: the RAM store itself, and the atomics and mirrors
//! through which the settings ops and the cache bar talk to the tiers living on
//! the worker thread.
//!
//! ## Named by content, which is the whole design (K-178, docs/06 §5.2)
//!
//! Every frame is filed under a **content hash** — a hash of everything that
//! went into it: each layer's evaluated transform, its effects, masks, blend and
//! switches, which source frame each footage layer reads, the parent chain it
//! inherits, and the preview resolution ([`lumit_render::cache::frame_key`]).
//! Nothing about *where* the frame sits enters the name.
//!
//! Three consequences, and they are the reason the cache feels the way it does:
//!
//! * **An edit that cannot change a pixel costs nothing.** Renaming a layer,
//!   nudging the work area, changing a hidden layer's opacity, adding sound to a
//!   layer, moving a marker: all of them produce the same names, so every held
//!   frame stays held and the cache bar stays green. This module used to empty
//!   itself on *every* commit, because the names were positional and an edit did
//!   not change them — so a rename retired the whole composition.
//! * **An undo is instantly valid again.** The restored document asks for the
//!   names it asked for before the edit, and if they have not been evicted they
//!   are still here. Nothing has to be re-rendered to get back to where you
//!   were, which is the After Effects Global Performance Cache lesson taken
//!   whole (docs/06 §5.2).
//! * **There is no invalidation machinery at all.** An edit changes values,
//!   values change hashes, and old entries simply stop being addressed and age
//!   out through eviction. No dirty flags, no dependency walk, nothing to get
//!   wrong.
//!
//! A frame is only nameable once its footage is probed; until then it renders
//! live and is banked nowhere, so a cache entry can never be a promise the
//! renderer did not keep.
//!
//! ## What fills this tier
//!
//! Two things. The Scopes path renders CPU pixels of its own (the zero-copy
//! Viewer keeps none, K-183) and files them. And the **demotion ladder**
//! (docs §5.3): a frame squeezed out of the VRAM cache is read back off the card
//! and lands here, then goes on to disk — and can be put straight back on the
//! card when it is wanted again, without compositing anything. That read-back is
//! started at eviction time and collected later, so an eviction never makes the
//! preview wait.
//!
//! ## Budget and eviction
//!
//! Bounded by a byte budget ([`DEFAULT_BUDGET_BYTES`], overridable from
//! Settings → Performance), least-recently-used first. Eviction scans for the
//! oldest entry (`O(n)` in the number of held frames — tens at 1080p under the
//! default budget, so the scan is cheap; a linked-hash-map would make it `O(1)`
//! if the count ever grows large, noted as future work).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The default RAM cap for rendered frames: 512 MiB. Sized so a comfortable run
/// of 1080p frames (~8 MiB each → ~64 frames) stays warm without the cache
/// growing without bound. Settings → Performance overrides it via
/// [`set_budget`].
pub(crate) const DEFAULT_BUDGET_BYTES: usize = 512 * 1024 * 1024;

/// One frame's cache identity: the content hash from
/// [`lumit_render::cache::frame_key`]. Every tier files a frame under the same
/// number, which is what lets a frame move up and down the ladder without anyone
/// having to know where it has been.
pub(crate) type FrameKey = u128;

/// Where a held frame sits in its composition, and at what preview scale — kept
/// beside the bytes but never part of the name (see [`best_frame`]).
pub(crate) type Provenance = lumit_render::FrameProvenance;

/// One cached frame: its dimensions, the tightly-packed display bytes, and the
/// bookkeeping the store and its consumers need.
struct Entry {
    width: u32,
    height: u32,
    /// The display bytes, in a shared handle. A frame is 8 MB at 1080p, and it
    /// goes up to the graphics card again each time playback comes past it. The
    /// handle makes that trip a counter increment in place of an 8 MB copy.
    bytes: Arc<Vec<u8>>,
    /// True when `bytes` are BGRA (the Windows and macOS zero-copy order) rather
    /// than RGBA. Frames come down off the card in the order they were
    /// composited in and go back up the same way, so no swizzle is paid on the
    /// render path; the one consumer that needs true RGBA ([`best_frame`], for
    /// the Scopes) converts its own copy.
    bgra: bool,
    /// What the frame cost to render, in milliseconds — carried so a frame put
    /// back on the card keeps its cost-aware eviction ranking there.
    cost_ms: u32,
    provenance: Provenance,
    last_used: u64,
}

/// The rendered-frame cache: an LRU of display-encoded frames under a byte
/// budget, each named by its content hash (see the module docs).
pub(crate) struct Cache {
    budget: usize,
    used: usize,
    map: HashMap<FrameKey, Entry>,
    /// Monotonic LRU clock; each access stamps an entry's `last_used`.
    clock: u64,
    hits: u64,
    misses: u64,
}

impl Cache {
    fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            map: HashMap::new(),
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Fetch a cached frame, stamping it most-recently-used. Counts one hit or
    /// one miss. The returned bytes are cloned (the caller owns them; the cache
    /// keeps its copy).
    fn get(&mut self, key: &FrameKey) -> Option<(u32, u32, Vec<u8>)> {
        self.clock += 1;
        let clock = self.clock;
        match self.map.get_mut(key) {
            Some(entry) => {
                entry.last_used = clock;
                self.hits += 1;
                Some((entry.width, entry.height, entry.bytes.as_ref().clone()))
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Store a rendered frame, evicting the least-recently-used entries first so
    /// the total stays within budget. A single frame larger than the whole
    /// budget is not cached (it would evict everything and still not fit).
    fn put(&mut self, key: FrameKey, entry: Entry) {
        let bytes = entry.bytes.len();
        if bytes == 0 || bytes > self.budget {
            return;
        }
        // Replacing an existing key: reclaim its bytes first.
        if let Some(old) = self.map.remove(&key) {
            self.used = self.used.saturating_sub(old.bytes.len());
        }
        self.evict_until_fits(bytes);
        self.clock += 1;
        let clock = self.clock;
        self.map.insert(
            key,
            Entry {
                last_used: clock,
                ..entry
            },
        );
        self.used += bytes;
    }

    /// Drop least-recently-used entries until `incoming` more bytes fit.
    fn evict_until_fits(&mut self, incoming: usize) {
        while !self.map.is_empty() && self.used + incoming > self.budget {
            // Find the oldest entry (smallest `last_used`).
            let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(e) = self.map.remove(&oldest) {
                self.used = self.used.saturating_sub(e.bytes.len());
            }
        }
    }

    /// Resize the budget, evicting down to it immediately.
    fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.evict_until_fits(0);
    }

    /// Throw away every cached frame. Keeps the configured budget and the
    /// lifetime hit/miss counters.
    fn clear(&mut self) {
        self.map.clear();
        self.used = 0;
    }

    /// `(used_bytes, budget_bytes, entries, hits, misses)`.
    fn stats(&self) -> (usize, usize, usize, u64, u64) {
        (
            self.used,
            self.budget,
            self.map.len(),
            self.hits,
            self.misses,
        )
    }
}

/// The process-wide cache instance, shared by the render path and the FFI
/// controls. One Flutter window, one cache.
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn with_cache<R>(f: impl FnOnce(&mut Cache) -> R) -> R {
    let mutex = CACHE.get_or_init(|| Mutex::new(Cache::new(DEFAULT_BUDGET_BYTES)));
    let mut guard = mutex.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut guard)
}

/// This frame's held RGBA bytes, if the memory tier has them.
///
/// A hit returns under the cache lock and the lock is let go before anything
/// else happens; the lock never wraps GPU or FFI work (docs/14 §"no locks
/// across GPU").
///
/// Reading and banking are two calls rather than one `get_or_render` because
/// the decision to bank cannot always be made until *after* the render: a frame
/// drawn while a Lens flare's bake was still being made is of the previous lens
/// (K-350), and only the render itself can say whether that happened. The key
/// names the content, so a superseded render for the same key simply overwrites
/// with identical pixels.
pub(crate) fn get(key: FrameKey) -> Option<(u32, u32, Vec<u8>)> {
    with_cache(|c| c.get(&key))
}

/// Bank a freshly rendered frame under its content name — the other half of
/// [`get`].
pub(crate) fn put_rendered(
    key: FrameKey,
    provenance: Provenance,
    width: u32,
    height: u32,
    bytes: &[u8],
) {
    with_cache(|c| {
        c.put(
            key,
            Entry {
                width,
                height,
                bytes: Arc::new(bytes.to_vec()),
                bgra: false,
                cost_ms: 1,
                provenance,
                last_used: 0,
            },
        );
    });
}

/// File a frame the demotion ladder brought down off the graphics card
/// (docs/06 §5.3). Its bytes stay in the channel order they were composited in,
/// so the trip back up needs no conversion.
pub(crate) fn put_demoted(key: FrameKey, frame: &lumit_render::DemotedFrame, bytes: Arc<Vec<u8>>) {
    with_cache(|c| {
        c.put(
            key,
            Entry {
                width: frame.width,
                height: frame.height,
                bytes,
                bgra: frame.bgra,
                cost_ms: frame.cost_ms,
                provenance: frame.provenance,
                last_used: 0,
            },
        );
    });
}

/// File a frame that has just come back OFF disk (docs/06 §5.1's way up). The
/// bytes are on their way to the graphics card in the same breath; keeping a
/// share here means the next pass over this frame is an upload from memory
/// rather than another file read — without it, a comp larger than the VRAM
/// budget re-read every frame from disk on every single pass, and the IO
/// thread's throughput became the playback rate.
pub(crate) fn put_loaded(
    key: FrameKey,
    width: u32,
    height: u32,
    bgra: bool,
    cost_ms: u32,
    provenance: Provenance,
    bytes: Arc<Vec<u8>>,
) {
    with_cache(|c| {
        c.put(
            key,
            Entry {
                width,
                height,
                bytes,
                bgra,
                cost_ms,
                provenance,
                last_used: 0,
            },
        );
    });
}

/// One held frame, ready to be put back on the graphics card.
pub(crate) struct HeldFrame {
    pub width: u32,
    pub height: u32,
    /// A share of the cached bytes, not a copy of them. The cache keeps its own
    /// share until the frame ages out.
    pub bytes: Arc<Vec<u8>>,
    pub bgra: bool,
    pub cost_ms: u32,
}

/// Take a held frame for promotion back into VRAM — the way up the ladder.
/// Counts as neither a hit nor a miss: those numbers describe how well the
/// Viewer's own lookups are served, and a promotion is the machinery underneath
/// them.
pub(crate) fn held(key: FrameKey) -> Option<HeldFrame> {
    with_cache(|c| {
        let entry = c.map.get_mut(&key)?;
        c.clock += 1;
        entry.last_used = c.clock;
        Some(HeldFrame {
            width: entry.width,
            height: entry.height,
            bytes: entry.bytes.clone(),
            bgra: entry.bgra,
            cost_ms: entry.cost_ms,
        })
    })
}

/// Whether a frame is held, without touching recency — what the cache bar and
/// the idle fill ask.
pub(crate) fn contains(key: FrameKey) -> bool {
    with_cache(|c| c.map.contains_key(&key))
}

/// The finest held picture of `comp` at `frame`, whatever scale it was made at,
/// as true RGBA and stamped most-recently-used.
///
/// For the Scopes, which need the *numbers* in a frame rather than a frame at
/// any particular size. They were compositing the whole composition again to get
/// them — a second full render of the frame the Viewer had just rendered, several
/// times a second, all through playback. Any resolution answers the question a
/// waveform or a vectorscope asks, so the one already in hand will do.
///
/// This is the one lookup that asks a *positional* question, which a content
/// hash cannot answer — hence the provenance kept beside each entry. It is
/// deliberately a best effort: two positions with identical pixels share one
/// entry filed under whichever asked first, so this can miss a frame it could
/// have served, and then the Scopes render their own.
///
/// **`still_current` is what keeps it from answering with the wrong picture**
/// (K-330). An entry's provenance says which position asked for it, and that
/// stays true for ever — but what the position *shows* does not. Edit the comp
/// and frame 12 renders to a new name, while the old entry sits in the map
/// still claiming frame 12; the finest of the two wins, which alternates as
/// tiers churn, and the Scopes flicker between the picture and the picture it
/// used to be. So each candidate is asked whether its name is still what this
/// position renders to at the quality it was made at, and a stale one is passed
/// over rather than served. The predicate runs under the cache lock, so — like
/// the dropper's reader below — it must stay bounded, pure CPU, and nowhere
/// near the GPU or the FFI boundary (docs/14 §"no locks across GPU").
///
/// Does not count as a hit or a miss: those numbers describe how well the Viewer
/// is being served, and mixing a second consumer into them would make the meter
/// mean nothing.
pub(crate) fn best_frame(
    comp: uuid::Uuid,
    frame: u64,
    still_current: impl Fn(FrameKey, lumit_render::Quality) -> bool,
) -> Option<(u32, u32, Vec<u8>)> {
    with_cache(|c| {
        let key = *c
            .map
            .iter()
            .filter(|(k, e)| {
                e.provenance.comp == comp
                    && e.provenance.frame == frame
                    && still_current(**k, e.provenance.quality)
            })
            // The finest one held.
            .max_by_key(|(_, e)| e.provenance.scale_q)
            .map(|(k, _)| k)?;
        let entry = c.map.get_mut(&key)?;
        c.clock += 1;
        entry.last_used = c.clock;
        let mut bytes = entry.bytes.as_ref().clone();
        if entry.bgra {
            // The Scopes bin R, G and B by name, so BGRA bytes would read as a
            // channel-swapped picture. Paid on this consumer's own copy (it is
            // throttled to a few traces a second) rather than on every frame
            // coming down the ladder.
            for px in bytes.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        Some((entry.width, entry.height, bytes))
    })
}

/// Read from the finest held picture of `comp` at `frame` **without copying
/// it**: `read` is given the pixels, the width, the height and the channel
/// order, and what it returns is the answer.
///
/// The dropper's reason for existing. [`best_frame`] clones the whole frame —
/// eight megabytes at 1080p — which is the correct shape for the Scopes, who
/// then bin every pixel of it, and exactly the wrong one for a reader that wants
/// a few hundred pixels out of the middle. `read` runs under the cache lock,
/// thus it must stay what the dropper's is: bounded, pure-CPU, and nowhere near
/// the GPU or the FFI boundary (docs/14 §"no locks across GPU").
///
/// **The channel order is given, not corrected.** A frame that came down off the
/// card on Windows or macOS is BGRA, and putting that right here would mean a
/// copy of the whole frame — which is the cost this function exists to avoid.
/// The reader takes its small window first and puts that right instead.
///
/// Stamps most-recently-used, like [`best_frame`], and counts as neither a hit
/// nor a miss for the same reason: those numbers describe how well the Viewer
/// is served.
pub(crate) fn with_best_frame<R>(
    comp: uuid::Uuid,
    frame: u64,
    still_current: impl Fn(FrameKey, lumit_render::Quality) -> bool,
    read: impl FnOnce(&[u8], u32, u32, bool) -> R,
) -> Option<R> {
    with_cache(|c| {
        let key = *c
            .map
            .iter()
            .filter(|(k, e)| {
                e.provenance.comp == comp
                    && e.provenance.frame == frame
                    && still_current(**k, e.provenance.quality)
            })
            .max_by_key(|(_, e)| e.provenance.scale_q)
            .map(|(k, _)| k)?;
        let entry = c.map.get_mut(&key)?;
        c.clock += 1;
        entry.last_used = c.clock;
        Some(read(&entry.bytes, entry.width, entry.height, entry.bgra))
    })
}

/// Resize the RAM budget (Settings → Performance).
pub(crate) fn set_budget(bytes: usize) {
    with_cache(|c| c.set_budget(bytes));
}

/// Empty the cache now (Settings → Clear cache).
pub(crate) fn clear() {
    with_cache(|c| c.clear());
}

/// `(used_bytes, budget_bytes, entries, hits, misses)`.
pub(crate) fn stats() -> (usize, usize, usize, u64, u64) {
    with_cache(|c| c.stats())
}

/// The worker renderer's comp-decode counter, mirrored each loop turn so
/// `cache_stats` can report it — a decode that should not have happened (a
/// drag that re-decoded, a cache that missed) is then visible in Settings
/// rather than merely slow (docs/TODO.md, Render pipeline).
static COMP_DECODES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn publish_comp_decodes(count: u64) {
    COMP_DECODES.store(count, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn comp_decodes() -> u64 {
    COMP_DECODES.load(std::sync::atomic::Ordering::Relaxed)
}

/// The VRAM final-frame cache's controls and mirror. The textures themselves
/// live inside the worker's renderer (they are GPU objects only that thread
/// touches); what crosses threads is three atomics the settings ops write and
/// the worker applies, plus the used/entries numbers the worker publishes for the
/// meter.
///
/// What no longer crosses is a list of held keys. The cache bar used to merge one
/// (the keys were positions, so a position could be read off them); under content
/// keying a hash says nothing about where its frame sits, so the bar is built by
/// the worker — which can name each frame — and published whole. See [`bar`].
pub(crate) mod vram {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// The budget the settings asked for; the worker applies it on its next
    /// turn.
    static BUDGET: AtomicUsize = AtomicUsize::new(lumit_render::DEFAULT_VRAM_CACHE_BYTES);
    /// Bumped by Clear cache; the worker clears when it sees it move.
    static CLEARS: AtomicU64 = AtomicU64::new(0);
    /// What the worker last reported holding.
    static USED: AtomicU64 = AtomicU64::new(0);
    static ENTRIES: AtomicU64 = AtomicU64::new(0);
    /// The budget the worker's cache is ACTUALLY holding to, as it last
    /// reported. [`budget`] above is the wish the settings wrote; this is what
    /// arrived, and they differ for one loop turn after a change — or for good,
    /// if the applying ever breaks. The meter shows this one, because a meter
    /// that draws a wish it cannot see being honoured is how "the cache stops
    /// at 512 MB while Settings says 8 GB" goes unnoticed.
    static APPLIED: AtomicUsize = AtomicUsize::new(lumit_render::DEFAULT_VRAM_CACHE_BYTES);

    pub(crate) fn publish_applied(bytes: usize) {
        APPLIED.store(bytes, Ordering::Relaxed);
    }

    pub(crate) fn applied() -> usize {
        APPLIED.load(Ordering::Relaxed)
    }

    pub(crate) fn set_budget(bytes: usize) {
        BUDGET.store(bytes, Ordering::Relaxed);
    }

    pub(crate) fn budget() -> usize {
        BUDGET.load(Ordering::Relaxed)
    }

    pub(crate) fn request_clear() {
        CLEARS.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn clears() -> u64 {
        CLEARS.load(Ordering::Relaxed)
    }

    /// The worker's report of what it holds.
    pub(crate) fn publish(used: u64, entries: u64) {
        USED.store(used, Ordering::Relaxed);
        ENTRIES.store(entries, Ordering::Relaxed);
    }

    /// `(used, entries)` as last published.
    pub(crate) fn stats() -> (u64, u64) {
        (
            USED.load(Ordering::Relaxed),
            ENTRIES.load(Ordering::Relaxed),
        )
    }
}

/// The disk tier's controls and mirror — the same shape as [`vram`], because the
/// tier lives on the worker's IO thread and the settings ops run on whichever
/// thread frb gave them.
/// The decoded-source-frame pool's numbers, as the worker last published them.
///
/// Published rather than asked, for the same reason [`vram`] is: the pool lives
/// on the worker's renderer, and a settings window must not reach across the
/// loop to read it (K-184's rule, on the other side of the bridge).
pub(crate) mod decode {
    use std::sync::atomic::{AtomicU64, Ordering};

    static USED: AtomicU64 = AtomicU64::new(0);
    static DECODERS: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn publish(used: u64, decoders: u64) {
        USED.store(used, Ordering::Relaxed);
        DECODERS.store(decoders, Ordering::Relaxed);
    }

    /// `(used_bytes, open_decoders)`.
    pub(crate) fn stats() -> (u64, u64) {
        (
            USED.load(Ordering::Relaxed),
            DECODERS.load(Ordering::Relaxed),
        )
    }
}

/// What the graphics driver holds for the worker's device, as it last
/// published — the layer under every tier in this file, and the one nothing
/// could see until now.
pub(crate) mod gpu {
    use std::sync::atomic::{AtomicU64, Ordering};

    static ALLOCATED: AtomicU64 = AtomicU64::new(0);
    static RESERVED: AtomicU64 = AtomicU64::new(0);
    static TEXTURES: AtomicU64 = AtomicU64::new(0);
    static BUFFERS: AtomicU64 = AtomicU64::new(0);
    /// Whether the card draws from this process's own memory.
    static UNIFIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub(crate) fn publish_unified(unified: bool) {
        UNIFIED.store(unified, Ordering::Relaxed);
    }

    pub(crate) fn unified() -> bool {
        UNIFIED.load(Ordering::Relaxed)
    }

    pub(crate) fn publish(allocated: u64, reserved: u64, textures: u64, buffers: u64) {
        ALLOCATED.store(allocated, Ordering::Relaxed);
        RESERVED.store(reserved, Ordering::Relaxed);
        TEXTURES.store(textures, Ordering::Relaxed);
        BUFFERS.store(buffers, Ordering::Relaxed);
    }

    /// `(allocated_bytes, reserved_bytes, textures, buffers)`. The two byte
    /// figures are 0 on a backend that keeps no allocator accounting (Metal);
    /// the two counts are kept by every backend.
    pub(crate) fn stats() -> (u64, u64, u64, u64) {
        (
            ALLOCATED.load(Ordering::Relaxed),
            RESERVED.load(Ordering::Relaxed),
            TEXTURES.load(Ordering::Relaxed),
            BUFFERS.load(Ordering::Relaxed),
        )
    }
}

pub(crate) mod disk {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    /// Where the parked frames go (docs/07-UI-SPEC.md §15, Settings →
    /// Performance → Cache).
    #[derive(Clone, PartialEq, Eq, Debug, Default)]
    pub(crate) enum Location {
        /// Under the application's own cache directory, keyed by document id —
        /// the default, and the only one that works before a project has ever
        /// been saved.
        #[default]
        AppData,
        /// In a `<project>.lum-cache/` folder beside the project file. Per
        /// project by construction: move the project, and its cache follows.
        /// An unsaved project has nowhere to put one, so it falls back to
        /// [`Self::AppData`] until the first save.
        BesideProject,
        /// Under a folder the user chose — to park the cache on a faster or
        /// roomier drive. Application-wide.
        Custom(PathBuf),
    }

    static BUDGET: AtomicU64 = AtomicU64::new(lumit_render::diskio::DEFAULT_CAP_BYTES);
    static CLEARS: AtomicU64 = AtomicU64::new(0);
    static USED: AtomicU64 = AtomicU64::new(0);
    static ENTRIES: AtomicU64 = AtomicU64::new(0);
    /// The wanted location, and a counter the worker watches so a change is
    /// noticed exactly once.
    /// How many frames are in the write-behind queue, as the worker last
    /// published — the depth K-277 bounded, in the memory report (K-294).
    static PENDING_PARKS: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn publish_pending_parks(n: u64) {
        PENDING_PARKS.store(n, Ordering::Relaxed);
    }

    pub(crate) fn pending_parks() -> u64 {
        PENDING_PARKS.load(Ordering::Relaxed)
    }

    static LOCATION: Mutex<Option<(u64, Location)>> = Mutex::new(None);
    static LOCATION_EPOCH: AtomicU64 = AtomicU64::new(0);
    /// The folder the tier actually resolved to, for Settings to show. `None`
    /// means the tier is off (no project open yet, or no home directory).
    static ROOT: Mutex<Option<String>> = Mutex::new(None);

    pub(crate) fn set_budget(bytes: u64) {
        BUDGET.store(bytes, Ordering::Relaxed);
    }

    pub(crate) fn budget() -> u64 {
        BUDGET.load(Ordering::Relaxed)
    }

    pub(crate) fn request_clear() {
        CLEARS.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn clears() -> u64 {
        CLEARS.load(Ordering::Relaxed)
    }

    /// Ask for a location. The worker re-opens the cache on its next turn.
    pub(crate) fn set_location(location: Location) {
        let epoch = LOCATION_EPOCH.fetch_add(1, Ordering::Relaxed) + 1;
        let mut guard = LOCATION.lock().unwrap_or_else(|p| p.into_inner());
        *guard = Some((epoch, location));
    }

    /// The wanted location and the epoch it was asked at, or `None` if nobody
    /// has ever chosen one (the worker then uses [`Location::default`]).
    pub(crate) fn location() -> (u64, Location) {
        let guard = LOCATION.lock().unwrap_or_else(|p| p.into_inner());
        guard.clone().unwrap_or((0, Location::default()))
    }

    pub(crate) fn publish(used: u64, entries: u64) {
        USED.store(used, Ordering::Relaxed);
        ENTRIES.store(entries, Ordering::Relaxed);
    }

    pub(crate) fn stats() -> (u64, u64) {
        (
            USED.load(Ordering::Relaxed),
            ENTRIES.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn publish_root(root: Option<String>) {
        let mut guard = ROOT.lock().unwrap_or_else(|p| p.into_inner());
        *guard = root;
    }

    pub(crate) fn root() -> Option<String> {
        let guard = ROOT.lock().unwrap_or_else(|p| p.into_inner());
        guard.clone()
    }
}

/// The cache bar's per-frame strip (docs/06 §5.6: "redrawn from a lock-free
/// bitmap snapshot; the UI thread never queries the cache itself").
///
/// **Why the worker builds it and the interface only reads it.** Under content
/// keying, knowing whether frame 12 is held means *naming* frame 12 — hashing
/// the whole composition at that time — and only the worker can do that: the
/// hash needs the renderer's probe results, and hashing a few hundred frames is
/// not work to do on the interface's thread, let alone per paint. So the bar
/// leaves a note saying which composition and scale it is drawing ([`want`]) and
/// reads whatever the worker last published for it ([`read`]).
///
/// One frame is one **byte, in two nibbles**: where the picture is kept, and
/// how big it is.
///
/// The low nibble is the *storage state*, and is the whole of what the strip
/// used to say:
///
/// * `0` — nothing held.
/// * `1` — held in memory or on the card, but only at a coarser preview
///   resolution than asked for (dimmed green).
/// * `2` — held at this resolution: plays now (green).
/// * `3` — on disk only, at a coarser resolution (dimmed blue).
/// * `4` — on disk only, at this resolution: promotable, not yet playable
///   (blue).
///
/// Playable beats promotable, so a frame both held and parked reads as held.
///
/// The high nibble is the *resolution tier* (K-441, docs/15-DESIGN.md §6.3):
/// the preview **divisor** the held picture was actually made at, relative to
/// the scale the bar asked about — `1` full, `2` half, `3` third, `4` quarter,
/// the same ladder [`crate::realtime::tier_scale`] names. It is `0` exactly
/// when the storage state is `0`, since a frame nobody holds has no size.
///
/// So the bar can say not just "cached" but "cached at what size". Two limits
/// on that, both honest rather than guessed: a frame held at some *other*
/// scale — one no adaptive tier ever renders at — is not found at all and
/// reads as nothing held; and on a composition long enough to be sampled, a
/// frame the sweep has not reached yet wears its sample's tier along with its
/// sample's storage state until the refinement pass names it (see
/// `publish_cache_bar`).
pub(crate) mod bar {
    use std::sync::Mutex;

    /// Build a strip byte from a storage state (`0`..=`4`) and a preview
    /// divisor (`1`..=`4`, or `0` for nothing held). The one place the two
    /// nibbles are put together — see the module docs for what they mean.
    pub(crate) const fn pack(storage: u8, divisor: u8) -> u8 {
        (divisor << 4) | (storage & 0x0F)
    }

    /// The storage state out of a strip byte. Its twin — the divisor — is
    /// `byte >> 4`.
    ///
    /// Test-only: the strip crosses whole and the bar's painter splits it, so
    /// neither half has a reader in Rust. It stays because the split is what
    /// the packing is proved against.
    #[cfg(test)]
    pub(crate) const fn storage_of(byte: u8) -> u8 {
        byte & 0x0F
    }

    /// What the bar last asked to draw: composition, frame count and preview
    /// scale in thousandths.
    static WANTED: Mutex<Option<(uuid::Uuid, u64, u16)>> = Mutex::new(None);
    /// The strips the worker has published, newest first. A handful of slots
    /// rather than one: the interface may draw a bar for more than one
    /// composition or scale in a session (a comp switch, a Viewer resize), and a
    /// single slot would make the two knock each other out — each paint would
    /// find the other's strip and read blank.
    static PUBLISHED: Mutex<Vec<(uuid::Uuid, u16, Vec<u8>)>> = Mutex::new(Vec::new());

    /// How many strips are kept. Four is two compositions at two scales; a
    /// 4000-frame strip is 4 kB, so the whole store is measured in kilobytes.
    const SLOTS: usize = 4;

    /// Record what the bar is drawing, so the worker knows what to compute.
    pub(crate) fn want(comp: uuid::Uuid, frames: u64, scale_q: u16) {
        let mut guard = WANTED.lock().unwrap_or_else(|p| p.into_inner());
        *guard = Some((comp, frames, scale_q));
    }

    /// What the bar is asking for, if it has asked at all.
    pub(crate) fn wanted() -> Option<(uuid::Uuid, u64, u16)> {
        let guard = WANTED.lock().unwrap_or_else(|p| p.into_inner());
        *guard
    }

    /// Publish a freshly computed strip, replacing any earlier one for the same
    /// composition and scale and pushing the oldest out.
    pub(crate) fn publish(comp: uuid::Uuid, scale_q: u16, tiers: Vec<u8>) {
        let mut guard = PUBLISHED.lock().unwrap_or_else(|p| p.into_inner());
        guard.retain(|(held, scale, _)| !(*held == comp && *scale == scale_q));
        guard.insert(0, (comp, scale_q, tiers));
        guard.truncate(SLOTS);
    }

    /// The **storage states** for `comp` at `scale_q` — the low nibble of each
    /// strip byte, `0`..=`4` — padded or trimmed to `frames`.
    ///
    /// All zeros when the worker has not published this composition at this
    /// scale yet — the honest answer, and one the next worker turn corrects.
    ///
    /// Test-only since K-441 put both nibbles across the seam: the bar reads
    /// [`read_packed`] and splits them itself, so nothing in the shipped
    /// library still asks for the storage half alone. It stays because it is
    /// what the masking is proved against.
    #[cfg(test)]
    pub(crate) fn read(comp: uuid::Uuid, frames: u64, scale_q: u16) -> Vec<u8> {
        let mut out = read_packed(comp, frames, scale_q);
        for byte in &mut out {
            *byte = storage_of(*byte);
        }
        out
    }

    /// The whole strip for `comp` at `scale_q`, both nibbles per frame (see the
    /// module docs), padded or trimmed to `frames`.
    pub(crate) fn read_packed(comp: uuid::Uuid, frames: u64, scale_q: u16) -> Vec<u8> {
        want(comp, frames, scale_q);
        let frames = frames as usize;
        let guard = PUBLISHED.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = vec![0u8; frames];
        if let Some((_, _, tiers)) = guard
            .iter()
            .find(|(held, scale, _)| *held == comp && *scale == scale_q)
        {
            let n = tiers.len().min(frames);
            out[..n].copy_from_slice(&tiers[..n]);
        }
        out
    }

    /// Forget every published strip — after a Clear cache, so the bar does not
    /// keep drawing frames that have just been thrown away until the worker's
    /// next turn.
    pub(crate) fn invalidate() {
        let mut guard = PUBLISHED.lock().unwrap_or_else(|p| p.into_inner());
        guard.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Frame names in these tests are arbitrary `u128`s: the cache only ever
    /// compares them. What the name MEANS — and the guarantee that an edit which
    /// cannot change a pixel produces the same name — is `lumit-render`'s to
    /// prove, and it does (`cache::tests`, `headless::tests`).
    /// One test at a time through the process-wide cache.
    ///
    /// These tests share the one `CACHE` this module owns — they `clear()` it,
    /// put frames in it and read them back — and cargo runs them in parallel
    /// threads of one process. Two of them interleaving is a test clearing
    /// another's frames out from under it, which shows up as one unrelated case
    /// failing every so often and passing on a re-run: the worst kind, because
    /// it teaches everybody to re-run rather than to look. Caught on this
    /// branch's own suite (K-294), pre-existing rather than new.
    ///
    /// The lock is taken for the body of every test that touches the global; a
    /// poisoned lock is recovered rather than propagated, so one genuine
    /// failure does not cascade into "all the others failed too".
    static CACHE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn cache_test_guard() -> std::sync::MutexGuard<'static, ()> {
        CACHE_TESTS.lock().unwrap_or_else(|e| e.into_inner())
    }

    const A: FrameKey = 1;
    const B: FrameKey = 2;
    const C: FrameKey = 3;

    fn at(comp: uuid::Uuid, frame: u64, scale_q: u16) -> Provenance {
        Provenance {
            comp,
            frame,
            scale_q,
            quality: lumit_render::Quality::default(),
        }
    }

    /// A positional lookup that accepts every candidate — for the tests that
    /// are about the lookup itself rather than about staleness.
    fn anything(_key: FrameKey, _quality: lumit_render::Quality) -> bool {
        true
    }

    /// One entry of `bytes` bytes. The dimensions are nominal — this store only
    /// ever compares names and counts bytes — except where a test hands the same
    /// frame back out, which is why they are given rather than assumed.
    fn sized(width: u32, height: u32, bytes: usize, provenance: Provenance) -> Entry {
        Entry {
            width,
            height,
            bytes: Arc::new(vec![7u8; bytes]),
            bgra: false,
            cost_ms: 1,
            provenance,
            last_used: 0,
        }
    }

    fn entry(bytes: usize, provenance: Provenance) -> Entry {
        sized(2, 2, bytes, provenance)
    }

    /// A cached frame is served on the second identical request without invoking
    /// the renderer — the scrub guarantee, proven with a render counter on a
    /// local cache (deterministic, no GPU, no shared global).
    #[test]
    fn a_cached_frame_is_served_without_re_rendering() {
        let _guard = cache_test_guard();
        let mut cache = Cache::new(DEFAULT_BUDGET_BYTES);
        let comp = uuid::Uuid::now_v7();
        let renders = std::cell::Cell::new(0u32);

        let once = |cache: &mut Cache| -> (u32, u32, Vec<u8>) {
            if let Some(hit) = cache.get(&A) {
                return hit;
            }
            renders.set(renders.get() + 1);
            cache.put(A, sized(4, 4, 4 * 4 * 4, at(comp, 0, 1000)));
            (4, 4, vec![7u8; 4 * 4 * 4])
        };

        let first = once(&mut cache);
        assert_eq!(renders.get(), 1, "first request renders");
        let second = once(&mut cache);
        assert_eq!(
            renders.get(),
            1,
            "second identical request is served from the cache"
        );
        assert_eq!(first, second, "the cached bytes match the rendered ones");
    }

    /// An edit that changes the picture changes the frame's name, so the render
    /// path simply misses and renders afresh — there is no invalidation step to
    /// get wrong. The flip side matters just as much: an edit that changes no
    /// pixel produces the same name and hits, which is why the cache no longer
    /// empties itself on every commit.
    #[test]
    fn a_changed_frame_name_misses_and_an_unchanged_one_hits() {
        let _guard = cache_test_guard();
        let mut cache = Cache::new(DEFAULT_BUDGET_BYTES);
        let comp = uuid::Uuid::now_v7();
        cache.put(A, entry(16, at(comp, 0, 1000)));

        assert!(
            cache.get(&B).is_none(),
            "a picture-changing edit renames the frame, so it misses"
        );
        assert!(
            cache.get(&A).is_some(),
            "an edit that cannot change a pixel keeps the name, so it hits"
        );
    }

    /// The byte budget evicts the least-recently-used frame first.
    #[test]
    fn the_budget_evicts_least_recently_used() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        // Budget holds exactly two 16-byte frames.
        let mut cache = Cache::new(32);
        cache.put(A, entry(16, at(comp, 0, 1000)));
        cache.put(B, entry(16, at(comp, 1, 1000)));
        // Touch A so B is now the least-recently-used.
        assert!(cache.get(&A).is_some());
        cache.put(C, entry(16, at(comp, 2, 1000)));

        assert!(cache.get(&A).is_some(), "recently used survives");
        assert!(cache.get(&C).is_some(), "the new frame is present");
        assert!(cache.get(&B).is_none(), "the LRU frame was evicted");
        let (used, budget, entries, _, _) = cache.stats();
        assert_eq!(budget, 32);
        assert_eq!(entries, 2);
        assert_eq!(used, 32);
    }

    /// Shrinking the budget evicts immediately; clearing empties the cache.
    #[test]
    fn resizing_and_clearing_free_frames() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        let mut cache = Cache::new(64);
        cache.put(A, entry(16, at(comp, 0, 1000)));
        cache.put(B, entry(16, at(comp, 1, 1000)));
        assert_eq!(cache.stats().2, 2);

        cache.set_budget(16); // room for one
        assert_eq!(cache.stats().2, 1, "shrinking the budget evicts");

        cache.clear();
        assert_eq!(cache.stats().0, 0);
        assert_eq!(cache.stats().2, 0);
    }

    /// A frame larger than the whole budget is refused rather than thrashing.
    #[test]
    fn an_oversized_frame_is_not_cached() {
        let _guard = cache_test_guard();
        let mut cache = Cache::new(16);
        cache.put(A, entry(64, at(uuid::Uuid::now_v7(), 0, 1000)));
        assert_eq!(cache.stats().2, 0, "oversized frame skipped");
    }

    /// The global FFI-facing controls round-trip: clear, set budget, stats.
    #[test]
    fn global_controls_round_trip() {
        let _guard = cache_test_guard();
        clear();
        set_budget(123 * 1024 * 1024);
        let (used, budget, _entries, _hits, _misses) = stats();
        assert_eq!(budget, 123 * 1024 * 1024);
        assert_eq!(used, 0);
        // Restore the default so other tests see a sane budget.
        set_budget(DEFAULT_BUDGET_BYTES);
    }

    /// The Scopes read the values in a frame, so any resolution answers their
    /// question — and the frame the Viewer just rendered is right there. They
    /// were compositing the composition a second time to get it, several times a
    /// second, for as long as playback ran with the panel open. A content hash
    /// cannot answer "any picture of frame 5", which is what the provenance kept
    /// beside each entry is for.
    #[test]
    fn the_finest_held_picture_of_a_frame_is_reusable() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        let other = uuid::Uuid::now_v7();
        clear();
        with_cache(|c| {
            c.put(1, entry(64, at(comp, 5, 250)));
            c.put(2, entry(256, at(comp, 5, 500)));
            c.put(3, entry(1024, at(comp, 6, 1000)));
            c.put(4, entry(4096, at(other, 5, 1000)));
        });

        // The finest one held for frame 5 of this comp is the 500-thousandths
        // entry, not the 250 one and not another comp's.
        let (_, _, bytes) = best_frame(comp, 5, anything).expect("frame 5 is held");
        assert_eq!(bytes.len(), 256, "the finest one held, not just any");

        assert!(
            best_frame(comp, 7, anything).is_none(),
            "nothing held for frame 7"
        );
        let (_, _, others) = best_frame(other, 5, anything).expect("the other comp has its own");
        assert_eq!(others.len(), 4096, "never another composition's picture");
        clear();
    }

    /// **The Scopes were showing the picture a frame used to be.** Reported on
    /// 0.2.0: retime a footage layer and the scope jumps and flickers and
    /// matches nothing in the Viewer.
    ///
    /// The edit renames every frame it touches, so the comp renders frame 5 to
    /// a new name — and the entry made before it is still in the map, still
    /// saying it is a picture of frame 5, because provenance records where a
    /// frame *came from* and that never stops being true. Whichever of the two
    /// was made at the finer scale won, and which one that was flipped as the
    /// tiers churned under playback: hence the flicker, and hence a scope that
    /// disagreed with the picture beside it. A candidate whose name is no
    /// longer this position's name is now passed over (K-330).
    #[test]
    fn a_frame_the_edit_orphaned_is_not_served_positionally() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        clear();
        // 1 is what frame 5 shows now; 2 is what it showed before the retime,
        // and it is the finer of the two — so it wins on scale alone.
        with_cache(|c| {
            c.put(1, entry(64, at(comp, 5, 250)));
            c.put(2, entry(256, at(comp, 5, 1000)));
        });
        let current = |key: FrameKey, _q: lumit_render::Quality| key == 1;

        let (_, _, bytes) = best_frame(comp, 5, current).expect("the current frame is held");
        assert_eq!(
            bytes.len(),
            64,
            "the picture frame 5 shows now, not the finer one it used to show"
        );

        // The stale entry is passed over, not evicted: its name is still valid
        // content, so an undo that brings it back must find it there.
        assert!(contains(2), "a stale position does not retire the frame");

        // And with nothing current held, the caller renders its own rather than
        // being handed the old picture.
        with_cache(|c| {
            c.map.remove(&1);
        });
        assert!(
            best_frame(comp, 5, current).is_none(),
            "no current picture is answered with none, never with the old one"
        );
        clear();
    }

    /// A frame that came down off the card in BGRA is handed to the Scopes as
    /// RGBA — they bin channels by name, so the swap would show as a
    /// blue-and-red-swapped picture. The bytes themselves stay in the order they
    /// arrived, so the trip back up the ladder is still conversion-free.
    #[test]
    fn a_demoted_bgra_frame_reaches_the_scopes_as_rgba() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        clear();
        with_cache(|c| {
            c.put(
                A,
                Entry {
                    width: 1,
                    height: 1,
                    bytes: Arc::new(vec![1, 2, 3, 4]),
                    bgra: true,
                    cost_ms: 9,
                    provenance: at(comp, 0, 1000),
                    last_used: 0,
                },
            );
        });

        let (_, _, bytes) = best_frame(comp, 0, anything).unwrap();
        assert_eq!(bytes, vec![3, 2, 1, 4], "given to the Scopes as RGBA");
        let up = held(A).expect("still held for promotion");
        assert_eq!(*up.bytes, vec![1, 2, 3, 4], "and kept as it came down");
        assert!(up.bgra);
        assert_eq!(up.cost_ms, 9, "with the cost that earned it its place");
        clear();
    }

    /// A frame read back off disk is held in memory as well as uploaded — so
    /// the NEXT pass over it is an upload from here, not another file read.
    /// Without this, a comp larger than the VRAM budget re-read every frame
    /// from disk on every pass, and the IO thread's rate became the playback
    /// rate.
    #[test]
    fn a_disk_load_is_banked_in_memory_for_the_next_pass() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        clear();
        let bytes = Arc::new(vec![9u8; 16]);
        put_loaded(A, 2, 2, true, 16, at(comp, 3, 1000), bytes);
        let up = held(A).expect("held for the next promotion");
        assert_eq!(*up.bytes, vec![9u8; 16]);
        assert!(up.bgra, "in the order it will go up in");
        assert_eq!(up.cost_ms, 16, "dear enough to keep");
        clear();
    }

    /// The bar is a mirror: it says what it is drawing, and reads what the
    /// worker published for exactly that composition and scale. A strip for
    /// another composition — or the same one at another scale — must never be
    /// handed over, or the bar would promise frames that do not exist.
    #[test]
    fn the_bar_reads_only_the_strip_it_asked_for() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        let other = uuid::Uuid::now_v7();
        bar::publish(comp, 1000, vec![2, 2, 1, 4, 0]);

        assert_eq!(bar::read(comp, 5, 1000), vec![2, 2, 1, 4, 0]);
        assert_eq!(
            bar::read(comp, 3, 1000),
            vec![2, 2, 1],
            "trimmed to the frames asked for"
        );
        assert_eq!(
            bar::read(comp, 7, 1000),
            vec![2, 2, 1, 4, 0, 0, 0],
            "and padded past what was published"
        );
        assert_eq!(
            bar::read(comp, 5, 500),
            vec![0; 5],
            "another scale is not this strip"
        );
        assert_eq!(
            bar::read(other, 5, 1000),
            vec![0; 5],
            "and neither is another composition"
        );

        // Two compositions drawn in turn must not knock each other out: a
        // single slot meant each paint found the other's strip and read blank.
        bar::publish(other, 1000, vec![4, 4, 4]);
        assert_eq!(bar::read(other, 3, 1000), vec![4, 4, 4]);
        assert_eq!(
            bar::read(comp, 5, 1000),
            vec![2, 2, 1, 4, 0],
            "the first composition's strip is still there"
        );

        // Reading records what to compute next, so the worker follows the bar.
        assert_eq!(bar::read(other, 4, 250).len(), 4);
        assert_eq!(bar::wanted(), Some((other, 4, 250)));

        bar::invalidate();
        assert_eq!(bar::read(comp, 5, 1000), vec![0; 5], "cleared means blank");
    }

    /// K-441, docs/15-DESIGN.md §6.3: a strip byte says *where* a frame is
    /// kept and *how big* it is, and the two must stay separable — the bar
    /// draws its storage states from one nibble and its hue from the other.
    ///
    /// [`bar::storage_of`] is the split, and it keeps answering `0`..=`4`
    /// however large the divisor grows. A packed byte read as a storage state
    /// unmasked draws a frame held at quarter as some state nobody has ever
    /// defined, which is what the painter would do given the wrong half.
    #[test]
    fn a_strip_byte_carries_the_storage_state_and_the_resolution_tier() {
        let _guard = cache_test_guard();
        let comp = uuid::Uuid::now_v7();
        // Held at the asked scale, held at half, parked at quarter, nothing.
        bar::publish(
            comp,
            1000,
            vec![
                bar::pack(2, 1),
                bar::pack(1, 2),
                bar::pack(3, 4),
                bar::pack(0, 0),
            ],
        );

        assert_eq!(
            bar::read(comp, 4, 1000),
            vec![2, 1, 3, 0],
            "the storage half is exactly what the strip always said"
        );
        assert_eq!(
            bar::read_packed(comp, 4, 1000),
            vec![0x12, 0x21, 0x43, 0x00],
            "and the whole byte carries the divisor above it"
        );
        // Nothing held has no size, so a zero byte stays a zero byte and the
        // sampler's "is this frame held at all?" test keeps working.
        assert_eq!(bar::pack(0, 0), 0);
        assert_eq!(bar::storage_of(bar::pack(4, 3)), 4);

        bar::invalidate();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod transport_cost {
    /// A stopwatch for the pixel path's serialisation, run by hand:
    /// `cargo test -p lumit_bridge --release -- --ignored --nocapture encode_cost`
    ///
    /// It reproduces the generated `SseEncode for Vec<u8>` exactly — a per-byte
    /// `write_u8` loop (`frb_generated.rs`, `impl SseEncode for Vec<u8>`) —
    /// against the bulk copy the same bytes could have had. The generated code
    /// itself is not callable from a test (the trait is private to the generated
    /// module), so this measures the identical loop rather than the code.
    #[test]
    #[ignore = "timing, not correctness"]
    fn encode_cost() {
        use flutter_rust_bridge::for_generated::byteorder::WriteBytesExt;

        for (label, w, h) in [("800x450", 800u32, 450u32), ("1920x1080", 1920, 1080)] {
            let bytes = (w * h * 4) as usize;
            let frame = vec![7u8; bytes];
            let n = 20;

            let started = std::time::Instant::now();
            for _ in 0..n {
                let mut out: Vec<u8> = Vec::new();
                for item in &frame {
                    out.write_u8(*item).unwrap();
                }
                std::hint::black_box(out);
            }
            let per_byte = started.elapsed().as_secs_f64() * 1000.0 / f64::from(n);

            let started = std::time::Instant::now();
            for _ in 0..n {
                let mut out: Vec<u8> = Vec::new();
                out.extend_from_slice(&frame);
                std::hint::black_box(out);
            }
            let bulk = started.elapsed().as_secs_f64() * 1000.0 / f64::from(n);

            println!(
                "ENCODE {label:>10} {:>5.1} MB  per-byte {per_byte:>7.2} ms  bulk {bulk:>7.2} ms",
                bytes as f64 / 1e6
            );
        }
    }
}
