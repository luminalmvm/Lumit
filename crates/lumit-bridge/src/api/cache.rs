//! The rendered-frame cache's readouts and controls.
//!
//! # In plain terms
//!
//! Rendering a frame is expensive, so the engine keeps the ones it has already
//! made and hands them back if you return to that moment. This is the window
//! onto that store: how much of it is used, how often it saved a render, and the
//! two buttons that resize or empty it.
//!
//! It is deliberately a readout rather than something the interface has to keep
//! in step. Nothing here changes the document, so none of it is undoable and
//! none of it goes through an op — asking is always safe.

use flutter_rust_bridge::frb;

/// What the cache currently holds, for the Timeline's cache bar and the
/// Settings window's budget control.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeCacheStats {
    pub used_bytes: u64,
    pub budget_bytes: u64,
    pub entries: u64,
    /// Frames served from the cache rather than rendered.
    pub hits: u64,
    /// Frames that had to be rendered.
    pub misses: u64,
    /// Comp frames the worker's renderer has genuinely DECODED this session,
    /// as last mirrored. The drag fast path's meter: a value drag must not
    /// move it, so a decode that should not have happened is visible here
    /// rather than merely slow.
    pub comp_decodes: u64,
}

#[frb(ignore)]
fn read() -> BridgeCacheStats {
    let (used, budget, entries, hits, misses) = crate::framecache::stats();
    BridgeCacheStats {
        used_bytes: used as u64,
        budget_bytes: budget as u64,
        entries: entries as u64,
        hits,
        misses,
        comp_decodes: crate::framecache::comp_decodes(),
    }
}

/// The cache's live numbers.
///
/// Cheap enough to poll on the interface's own cadence — it reads five counters
/// behind one lock and renders nothing. It must never be called *per paint*
/// even so: the lock it takes is the one a render holds.
#[frb(sync)]
pub fn cache_stats() -> BridgeCacheStats {
    read()
}

/// Where this process's memory has gone: what each tier admits to holding, and
/// what the operating system says the process holds.
///
/// **The field that matters is [`Self::unaccounted_bytes`].** Every tier here
/// is byte-budgeted and evicts to stay inside its budget, so a report where the
/// tiers add up to their budgets and the process is a hundred times larger is
/// not a cache problem at all — it is memory nobody in this list is counting,
/// which is a different search entirely. Lumit has twice been reported holding
/// tens of gigabytes, and both times that question took days to answer from the
/// outside. It is one call from the inside.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeMemoryReport {
    /// What the operating system says this process holds — Activity Monitor's
    /// **Memory** on macOS, the working set on Windows, `VmRSS` on Linux. 0
    /// where the platform will not say ([`crate::api::system::resident_memory_bytes`]).
    pub process_bytes: u64,
    /// Finished frames held in ordinary memory.
    pub frame_cache_bytes: u64,
    /// Frames held on the graphics card.
    pub vram_cache_bytes: u64,
    /// Whether the card's memory *is* this process's memory — true on every
    /// Apple Silicon Mac and on any integrated adapter.
    ///
    /// When it is, `vram_cache_bytes` is counted against `process_bytes` like
    /// any other tier; when it is not, the card's frames are somewhere else
    /// entirely and counting them would understate what is unaccounted for.
    /// Getting this backwards is how a cache doing exactly its job read as
    /// gigabytes nobody could explain.
    pub unified_memory: bool,
    /// Decoded source frames held for the compositor.
    pub decode_cache_bytes: u64,
    /// How many media decoders are open. Counted, not weighed: what a decoder
    /// holds is FFmpeg's and the driver's business, and a made-up number of
    /// bytes would be worse than an honest count.
    pub open_decoders: u64,
    /// How many frames are waiting to be written to disk — the write-behind
    /// queue, bounded at eight. A count rather than bytes on purpose: each
    /// waiting frame shares its allocation with the frame cache above (one
    /// `Arc`, both tiers), so charging it twice would make the report lie in
    /// the one direction that matters.
    pub park_queue_frames: u64,
    /// What the graphics driver holds for the render device, in use. On
    /// unified memory (every Apple Silicon Mac) this is inside
    /// `process_bytes`; on a discrete card it is not.
    pub gpu_allocated_bytes: u64,
    /// What the graphics driver has **reserved** — the blocks those
    /// allocations were carved from, including the free room inside them.
    ///
    /// The gap between this and `gpu_allocated_bytes` is memory that has been
    /// released by the engine and not handed back by the driver: free, and
    /// still ours.
    ///
    /// **Both byte figures are 0 on macOS**, where Metal keeps no such
    /// accounting and wgpu therefore reports none — which is the platform the
    /// question was asked on. Read the two counts below there.
    pub gpu_reserved_bytes: u64,
    /// How many textures the driver is holding for the render device right now.
    ///
    /// **This is the figure that works everywhere, Metal included.** The engine
    /// holds a handful at rest — the frames in the card's cache, the pooled
    /// upload textures, the shared present targets, and each frame's
    /// intermediates while it is being made. Thousands of them means frames the
    /// engine dropped were never destroyed, which is a different fault from any
    /// cache being too large and is not visible in any other number here.
    pub gpu_textures: u64,
    /// How many buffers the driver is holding, for the same reason.
    pub gpu_buffers: u64,
    /// `process_bytes` less everything above that lives in ordinary memory.
    /// Saturating at zero, since the platform's number and ours are read a
    /// moment apart and a small negative is meaningless.
    pub unaccounted_bytes: u64,
}

/// Read the memory report. Cheap: five atomics, one lock and one syscall.
#[frb(sync)]
#[must_use]
pub fn memory_report() -> BridgeMemoryReport {
    let (frame_cache, _, _, _, _) = crate::framecache::stats();
    let (vram, _) = crate::framecache::vram::stats();
    let (decode, decoders) = crate::framecache::decode::stats();
    let park = crate::framecache::disk::pending_parks();
    let (gpu_allocated, gpu_reserved, gpu_textures, gpu_buffers) = crate::framecache::gpu::stats();
    let process = crate::api::system::resident_memory_bytes();
    // On unified memory the card's frames are inside this process, so they are
    // accounted like any other tier; on a discrete card they are not in it at
    // all and must not be. The adapter says which, rather than the platform:
    // a Mac with an external card would answer differently, and so should the
    // report.
    let unified = crate::framecache::gpu::unified();
    let accounted = (frame_cache as u64)
        .saturating_add(decode)
        .saturating_add(if unified { vram } else { 0 });
    BridgeMemoryReport {
        process_bytes: process,
        frame_cache_bytes: frame_cache as u64,
        vram_cache_bytes: vram,
        unified_memory: unified,
        decode_cache_bytes: decode,
        open_decoders: decoders,
        park_queue_frames: park,
        gpu_allocated_bytes: gpu_allocated,
        gpu_reserved_bytes: gpu_reserved,
        gpu_textures,
        gpu_buffers,
        unaccounted_bytes: process.saturating_sub(accounted),
    }
}

/// Resize the cache, returning what it holds afterwards.
///
/// Shrinking evicts oldest-first straight away rather than waiting for the next
/// render, so the memory the user just asked to reclaim is actually gone when
/// the number on screen changes.
#[frb(sync)]
pub fn set_cache_budget(bytes: u64) -> BridgeCacheStats {
    crate::framecache::set_budget(usize::try_from(bytes).unwrap_or(usize::MAX));
    read()
}

/// Empty the cache. Every frame is re-rendered from here on, so this is a
/// deliberate act rather than something to do on a timer.
#[frb(sync)]
pub fn clear_cache() -> BridgeCacheStats {
    crate::framecache::clear();
    crate::framecache::bar::invalidate();
    read()
}

impl crate::api::composition::CompositionReference {
    /// Which frames of this composition are held, one byte each — **two
    /// nibbles**, because the bar says not just "cached" but "cached at what
    /// size" (docs/15 §6.3).
    ///
    /// The **low** nibble (`byte & 0x0F`) is the storage state the bar has
    /// always drawn:
    ///
    /// * `0` — nothing held.
    /// * `1` — held in memory or on the graphics card, but only at a coarser
    ///   preview resolution than `scale`.
    /// * `2` — held at `scale` or finer: plays now.
    /// * `3` — parked on disk only, at a coarser resolution.
    /// * `4` — parked on disk only, at this resolution: promotable, not yet
    ///   playable.
    ///
    /// The **high** nibble (`byte >> 4`) is the preview **divisor** the picture
    /// that was found was actually made at, relative to `scale`: `1` full, `2`
    /// half, `3` third, `4` quarter, and `0` when nothing is held. The tiers are
    /// probed finest first, so the answer is the best picture there is of that
    /// frame and does not depend on the order the tiers filled.
    ///
    /// Two truths to carry over rather than paper over: a frame held at a scale
    /// no adaptive tier renders at reads as **nothing held**, and on a sampled
    /// composition a frame the refinement sweep has not reached wears its
    /// sample's tier.
    ///
    /// This is what the Timeline's cache bar draws (docs/07-UI-SPEC.md §3.2,
    /// docs/06-RENDER-PIPELINE.md §5.6): mint for the memory pair, steel blue
    /// for the disk pair, stepped down as the divisor coarsens.
    ///
    /// **A mirror read, not a query.** Frames are named by a hash of their
    /// content (docs/06 §5.2), so answering "is frame 12 held?" means *naming*
    /// frame 12 — hashing the whole composition at that time, which needs the
    /// renderer's probe results. So the worker builds this strip and publishes
    /// it, and this reads what was published: §5.6's lock-free snapshot, where
    /// the interface never touches a cache itself. Asking also tells the worker
    /// what to compute, so a bar that starts drawing a different composition is
    /// answered within a turn or two — all zeros until then, which is the honest
    /// answer rather than another composition's frames.
    #[frb(sync)]
    pub fn cached_frames(&self, frames: u64, scale: f32) -> Vec<u8> {
        let scale_q = lumit_render::preview_scale_q(crate::render::quality_for(scale));
        crate::framecache::bar::read_packed(self.id, frames, scale_q)
    }
}

/// What the VRAM final-frame cache holds — the tier that makes a revisited
/// frame free on the zero-copy Viewer ("cache on the card", docs/06 §5.1).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeVramCacheStats {
    pub used_bytes: u64,
    pub budget_bytes: u64,
    pub entries: u64,
}

/// The VRAM cache's live numbers. `used`/`entries` are as the worker last
/// published (it holds the textures; nothing here touches the GPU); the
/// budget is the asked-for value, applied on the worker's next turn.
#[frb(sync)]
pub fn vram_cache_stats() -> BridgeVramCacheStats {
    let (used, entries) = crate::framecache::vram::stats();
    BridgeVramCacheStats {
        used_bytes: used,
        // The applied budget, not the asked-for one: they agree a loop turn
        // after a change, and a lasting disagreement is a bug the meter should
        // show rather than hide.
        budget_bytes: crate::framecache::vram::applied() as u64,
        entries,
    }
}

/// Resize the VRAM cache. The worker applies it on its next turn, evicting
/// down immediately then; the returned stats show the new budget with the
/// holdings as last published.
#[frb(sync)]
pub fn set_vram_cache_budget(bytes: u64) -> BridgeVramCacheStats {
    crate::framecache::vram::set_budget(usize::try_from(bytes).unwrap_or(usize::MAX));
    vram_cache_stats()
}

/// Empty the VRAM cache on the worker's next turn.
#[frb(sync)]
pub fn clear_vram_cache() -> BridgeVramCacheStats {
    crate::framecache::vram::request_clear();
    crate::framecache::bar::invalidate();
    vram_cache_stats()
}

/// What the disk tier holds — the bottom of the three-tier cache
/// (docs/06-RENDER-PIPELINE.md §5.4), and the only one that outlives the session.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDiskCacheStats {
    pub used_bytes: u64,
    pub budget_bytes: u64,
    pub entries: u64,
    /// The folder the frames are actually going to, for Settings to show. Empty
    /// when the tier is off — which, since an unsaved project falls back to the
    /// application's own cache folder, means only a platform with no home
    /// directory at all.
    pub root: String,
}

/// The disk tier's live numbers. `used`/`entries` are as the IO thread last
/// accounted them; the budget is the asked-for value, applied on the worker's
/// next turn.
#[frb(sync)]
pub fn disk_cache_stats() -> BridgeDiskCacheStats {
    let (used, entries) = crate::framecache::disk::stats();
    BridgeDiskCacheStats {
        used_bytes: used,
        budget_bytes: crate::framecache::disk::budget(),
        entries,
        root: crate::framecache::disk::root().unwrap_or_default(),
    }
}

/// Resize the disk tier. Shrinking evicts oldest-first on the IO thread.
#[frb(sync)]
pub fn set_disk_cache_budget(bytes: u64) -> BridgeDiskCacheStats {
    crate::framecache::disk::set_budget(bytes);
    disk_cache_stats()
}

/// Where the disk tier keeps its frames (docs/07-UI-SPEC.md §15).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgeCacheLocation {
    /// Under the application's own cache folder, keyed by document id — the
    /// default, and the only one that works before a project has been saved.
    AppData,
    /// In a `<project>.lum-cache/` folder beside the project file, so a project
    /// carries its cache with it. An unsaved project has nowhere to put one and
    /// falls back to [`Self::AppData`] until it is saved.
    BesideProject,
    /// Under a folder the user picked — to park the cache on a faster or roomier
    /// drive. Application-wide.
    Custom,
}

/// Choose where parked frames live. `folder` is used only for
/// [`BridgeCacheLocation::Custom`] and ignored otherwise.
///
/// The worker re-opens the cache on its next turn: frames already parked
/// elsewhere are not moved or deleted — they are simply not addressed from the
/// new folder, and the old one can be deleted by hand at any time with no
/// correctness effect.
#[frb(sync)]
pub fn set_disk_cache_location(
    location: BridgeCacheLocation,
    folder: String,
) -> BridgeDiskCacheStats {
    use crate::framecache::disk::Location;
    let location = match location {
        BridgeCacheLocation::AppData => Location::AppData,
        BridgeCacheLocation::BesideProject => Location::BesideProject,
        BridgeCacheLocation::Custom if !folder.is_empty() => {
            Location::Custom(std::path::PathBuf::from(folder))
        }
        // A custom location with no folder chosen is not a location; keeping the
        // default is better than pointing the tier at nothing.
        BridgeCacheLocation::Custom => Location::AppData,
    };
    crate::framecache::disk::set_location(location);
    disk_cache_stats()
}

/// A cache location as a pair: which of the three, and the folder that goes with
/// `Custom` (empty for the other two).
///
/// The same shape whether it is being set for the application
/// ([`set_disk_cache_location`]) or for one project
/// (`ProjectReference::set_cache_location`), so the interface has one control and
/// one type for both, and the only difference is where the answer is stored.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeProjectCacheLocation {
    pub location: BridgeCacheLocation,
    pub folder: String,
}

/// Delete every parked frame, on the IO thread. Unlike the other two tiers this
/// destroys files that may represent hours of rendering and cannot be undone, so
/// the interface asks first (docs/07-UI-SPEC.md §15).
#[frb(sync)]
pub fn clear_disk_cache() -> BridgeDiskCacheStats {
    crate::framecache::disk::request_clear();
    // The bar's published strip promises frames that are about to go; drop it so
    // the stripe does not draw them until the worker republishes.
    crate::framecache::bar::invalidate();
    disk_cache_stats()
}

/// Which route a rendered frame takes from the engine to the Viewer.
///
/// Worth reporting rather than assuming: the zero-copy paths are build features,
/// and a build without one silently falls back. That fallback is four trips for
/// a picture that never needed to leave the graphics card — composite, copy down
/// to ordinary memory, serialise a byte at a time across the boundary, upload
/// back to the card to draw — so "which one am I on?" is the first question to
/// ask when playback feels heavy, and it should not need a rebuild to answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgeViewerTransport {
    /// Windows: a D3D12 texture shared by handle. No pixels cross.
    SharedTexture,
    /// Linux: a DMA-BUF shared by file descriptor. No pixels cross.
    DmaBuf,
    /// Every pixel copied down, serialised, and uploaded again.
    ReadBack,
}

/// What this build compiles to. It reports the *build*, not the run — a machine
/// that cannot provide a shared texture still falls back at runtime.
#[frb(sync)]
pub fn viewer_transport() -> BridgeViewerTransport {
    #[cfg(all(windows, feature = "shared-texture"))]
    {
        BridgeViewerTransport::SharedTexture
    }
    #[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
    {
        BridgeViewerTransport::DmaBuf
    }
    #[cfg(not(any(
        all(windows, feature = "shared-texture"),
        all(target_os = "linux", feature = "shared-texture-linux")
    )))]
    {
        BridgeViewerTransport::ReadBack
    }
}

/// Ask the render worker to measure what each frame costs, per layer and per
/// effect (docs/13 §7.1), or to stop.
///
/// **Not free, which is why it is a switch.** A measured frame waits for the
/// graphics card at every node, so a millisecond on a Timeline row means the
/// work rather than the paperwork — and that wait is exactly the overlap a
/// brisk preview lives on. So the numbers are collected while something is
/// showing them and not otherwise, and playback is never measured whatever
/// this says.
///
/// Takes effect from the next frame; frames already in flight finish as they
/// started.
#[frb(sync)]
pub fn set_render_profiling(on: bool) {
    crate::profiling::set_wanted(on);
}
