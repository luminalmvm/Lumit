//! Rendering single comp frames without a window — what a frontend that draws
//! its own pixels (Flutter, over `lumit-bridge`) holds and drives.
//!
//! # In plain terms
//!
//! A [`HeadlessRenderer`] owns the expensive things that must survive between
//! frames: the GPU context (whose adapter is acquired once), the compiled
//! shaders, the open video decoders, and the probe results. Hold one per
//! session and ask it for frames.
//!
//! It offers two ways to render, and the difference matters:
//!
//! - [`HeadlessRenderer::render_preview`] is the **interactive** path. It plans
//!   the decode, reuses the pixels it decoded last time whenever the plan has
//!   not changed, builds a draw list and composites. Dragging an effect value
//!   changes what is *done* with the pixels, never *which* pixels are wanted,
//!   so a drag re-composites and decodes nothing at all. It also honours the
//!   preview resolution, so footage is decoded at the size it will be shown
//!   rather than at full size and thrown away. This is the path a Viewer wants.
//! - [`HeadlessRenderer::render_rgba`] is the **export** framing of the same
//!   walk: full decode quality, comp resolution.
//!
//! There is ONE comp walk (K-031): `build_comp_draws` + `Realiser::realise`.
//! The export encode loop drives it too, on its own renderer, so preview ==
//! export == the written file by construction — gated by the bit-identity
//! matrix in this file's tests.

use crate::decode::{CompFrame, CompJob, DecodePool};
use crate::export::{AudioJob, ItemInfo};
use crate::plan::{plan_comp_frame, Quality};
use crate::source::{SourceProbe, SourceProbes};
use lumit_core::model::{Composition, Document, FootageItem, LayerKind, ProjectItem};
// The one preview-size rounding, shared with the compositor's scaled render
// target so the composite and the final blit can never disagree about what
// "half size" is (it moved into lumit-gpu when the composite itself started
// running at the preview scale; the rounding is unchanged).
use lumit_gpu::scaled_size;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// The persistent GPU engines a render needs, held between calls so shaders
/// compile once. Taken out for the duration of one render and put back
/// afterwards, so a failed frame never discards the compiled pipelines.
struct Parts {
    colour: lumit_gpu::ColourEngine,
    compositor: lumit_gpu::Compositor,
    fx: lumit_gpu::fx::FxEngine,
    lut_cache: std::cell::RefCell<crate::fxops::LutCache>,
    /// The per-effect intermediate cache (K-421), VRAM only. Lives here
    /// rather than beside `frame_textures` because its entries are textures
    /// the effect engine made, and they go with it.
    fx_cache: std::cell::RefCell<crate::fxops::FxCache>,
}

/// One footage item's probe result, cached so a scrub does not re-probe. Slate
/// sizing is deliberately *not* stored here — the missing/failed slate is sized
/// to the comp being rendered at call time, since the same item can appear in
/// comps of different dimensions.
enum Probe {
    /// Decodable video: its exact rate, native size and frame count (the
    /// `frame_pick` and decode-width inputs).
    Ok {
        fps: f64,
        frames: usize,
        width: u32,
        height: u32,
    },
    /// A readable file with no video stream (audio-only). Not an error, so it
    /// must never draw the missing-footage slate — the layer simply
    /// contributes no picture, exactly as `item_infos` (export) and
    /// `collect_comp_jobs` (the live preview) already treat it.
    NoVideo,
    /// Not on disk, or present-but-unreadable: render the colour-bars slate,
    /// exactly as export's `item_infos` carries a `Missing` item (docs/07 §3.3).
    Slate,
}

/// A reusable, window-free renderer that turns `(Document, comp, frame)` into an
/// RGBA8 buffer through the export compositor. Hold one per frontend session.
///
/// The GPU adapter is acquired in [`HeadlessRenderer::new`]; a machine with no
/// adapter fails there, and the caller (the bridge) never constructs a second —
/// it returns its calm "no frame" state instead of retrying every call.
pub struct HeadlessRenderer {
    gpu: lumit_gpu::GpuContext,
    /// `Some` except for the instant a render borrows the engines. A render that
    /// unwinds (never expected — engine crates forbid panics) leaves this `None`,
    /// and further calls answer a calm error rather than crashing.
    parts: Option<Parts>,
    /// The GPU scope pass (K-096 v1). Held directly rather than in [`Parts`]
    /// because a scope trace runs *from a finished frame*, not during a
    /// composite, so it is never lent to the `Renderer` — it borrows `&self.gpu`
    /// on its own. Compiled once with the other engines.
    scope: lumit_gpu::scope::ScopeEngine,
    /// The `ItemInfo` map the renderer reads, rebuilt each call (cheap — it only
    /// reads `probe_cache`) so a missing item's slate matches the current comp.
    items: HashMap<Uuid, ItemInfo>,
    /// Probe results by footage id, so each file is probed at most once.
    probe_cache: HashMap<Uuid, Probe>,
    /// The audio-jobs walk with its has-audio probe cache, so building the
    /// export audio jobs probes each file at most once (export path only).
    audio_jobs: AudioJobsBuilder,
    /// The open decoders and the decoded-source-frame cache every render uses
    /// (K-031: the export drives this same path on its own renderer).
    pool: DecodePool,
    /// The last interactive frame's decoded per-layer pixels, kept with the
    /// plan that produced them — what makes a live value drag cost no decoding
    /// at all. Replaced whenever a render genuinely needs different pixels.
    retained: Option<Retained>,
    /// The VRAM final-frame cache (docs/06 §5.1's top tier, "cache on the
    /// card"): finished display textures keyed by their **content hash**
    /// ([`crate::cache::frame_key`]) and channel order. This is what makes a
    /// revisited frame free on the zero-copy Viewer, which keeps no CPU bytes to
    /// cache (K-183).
    ///
    /// Content-keyed, not keyed by position (docs/06 §5.2, K-178). That is what
    /// lets an edit which cannot change a pixel — a rename, a work-area nudge,
    /// an opacity keyframe on a hidden layer — keep every held frame, and what
    /// makes an undo instantly valid again: the restored document asks for the
    /// names it asked for before, and they are still here. A provisional drag
    /// render still passes `cacheable: false`, because its values were never
    /// committed and its pixels must not be filed under a name the document does
    /// not describe.
    frame_textures: lumit_cache::ByteLru<FrameTextureKey, FrameTexture>,
    /// Read-backs of evicted frames still in flight — the VRAM→RAM rung of the
    /// demotion ladder (docs/06 §5.3). Bounded by
    /// [`MAX_DEMOTIONS_IN_FLIGHT`]; drained by [`Self::poll_demotions`].
    demotions: Vec<Demotion>,
    /// Display textures that left the cache and can hold the next promoted
    /// frame — see [`Self::upload_frame_texture`]. Bounded by
    /// [`MAX_POOLED_TEXTURES`].
    upload_pool: Vec<std::sync::Arc<wgpu::Texture>>,
    /// How many `render_prepared` calls were served from [`Self::frame_textures`].
    frame_texture_hits: u64,
    /// Bumped whenever the held set changes — see [`Self::frame_texture_version`].
    frame_texture_version: u64,
    /// Where "this frame is such-and-such far along" reports go, when the owner
    /// has asked for them ([`Self::watch_frames`]). `None` — the default — is a
    /// renderer that reports nothing, which is what playback wants: a frame due
    /// in 16 ms has no use for a progress bar and no time to describe itself.
    progress: Option<crate::profile::ProgressSink>,
    /// Where a finished frame's per-layer and per-effect timings go, when they
    /// have been asked for ([`Self::measure_frames`]). Separate from `progress`
    /// because measuring costs real time (it fences the graphics card at each
    /// node) while reporting progress does not — so the Timeline's render-time
    /// column turns this on, and turning it off costs nothing to have had.
    profile: Option<crate::profile::ProfileSink>,
    /// Whether the *next* frame is watched, and whether it is measured. Set per
    /// render by the owner, because the same renderer serves both a scrub (a
    /// frame worth describing) and playback (a frame that must not be slowed).
    watching: bool,
    measuring: bool,
    /// The Viewer's own exposure and tone map (K-314) — a way of *looking* at
    /// the composite, never part of it.
    ///
    /// **This is how "it can never reach an export" is kept true.** It defaults
    /// to neutral and only [`Self::set_display_view`] moves it, and an export
    /// builds its own renderer (`export::run`) which nobody calls that on. So
    /// the promise is a property of the code's shape rather than a rule anyone
    /// has to remember — and `an_export_ignores_the_viewer_view` pins it.
    view: lumit_gpu::DisplayParams,
    /// Whether the fronted comp's own background colour is left out of the
    /// composite, so pixels nothing covers stay transparent and the Viewer's
    /// transparency grid shows through them (K-352). The Viewer sets it to
    /// follow its grid button; like [`Self::view`] it is a way of *looking*,
    /// and the export renderer — which nobody calls this on — always draws
    /// the backdrop.
    transparent_background: bool,
    /// The Viewer's region of interest as comp fractions (K-362), or `None`
    /// for the whole frame. Preview-only: the export renderer is never given
    /// one, the same construction that keeps the preview scale out of files.
    region: Option<[f32; 4]>,
    /// The Windows zero-copy Viewer targets (K-177): **one per size, kept and
    /// reused**, most recently used last.
    ///
    /// This was a single texture re-created whenever the size changed, and that
    /// is a handle churn the frontend cannot survive. Dart registers a texture
    /// with the platform runner and identifies it by its handle, so a new handle
    /// means a new registration and a round trip during which the outgoing
    /// texture is still on screen. One size change is fine. The case that is not
    /// is **alternation** — and creating a comp inside an existing project
    /// produces exactly that, because renders for the outgoing comp are still in
    /// flight while the new one starts, so present is called alternately at two
    /// sizes and a re-created texture hands out a fresh handle every frame. The
    /// registrations pile up, the compositor is asked to bind handles faster
    /// than it can, and it dies with "Binding D3D surface failed". An empty
    /// project has no outgoing comp, so no alternation and no crash — which is
    /// exactly the difference the bug report drew.
    ///
    /// Held per size, alternation costs nothing after the first frame at each:
    /// the same handle comes back, and Dart recognises it and does not
    /// re-register at all. It also means a texture is never freed under a
    /// compositor still drawing it, which is a second way the old shape could
    /// fail and this one cannot.
    ///
    /// Bounded and least-recently-used, because sizes are unbounded in
    /// principle: dragging the Viewer walks through a great many.
    #[cfg(all(windows, feature = "shared-texture"))]
    shared: Vec<lumit_gpu::shared::SharedTexture>,
    /// The Linux DMA-BUF sibling of [`Self::shared`], same reasoning — one Dart
    /// controller serves all three platforms.
    #[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
    shared_dmabuf: Vec<lumit_gpu::shared_linux::SharedDmabuf>,
    /// The macOS IOSurface sibling of [`Self::shared`] (K-195).
    #[cfg(all(target_os = "macos", feature = "shared-texture-macos"))]
    shared_iosurface: Vec<lumit_gpu::shared_metal::SharedIoSurface>,
}

/// How many differently-sized Viewer targets to keep alive at once.
///
/// Enough that the sizes actually in play — the outgoing comp, the incoming
/// one, and a resolution tier either side — all stay resident, so switching
/// between them re-uses handles instead of minting them. Small enough that a
/// slow drag through many sizes does not accumulate: each is roughly two
/// textures' worth of video memory.
#[cfg(any(
    all(windows, feature = "shared-texture"),
    all(target_os = "linux", feature = "shared-texture-linux"),
    all(target_os = "macos", feature = "shared-texture-macos")
))]
const SHARED_TARGET_POOL: usize = 4;

/// One frame's decoded per-layer pixels, kept alongside the decode plan that
/// asked for them, so the next render can tell at a glance whether it needs new
/// ones ([`crate::plan::same_decode`]).
struct Retained {
    comp: Uuid,
    frame: u64,
    jobs: Vec<CompJob>,
    pixels: CompFrame,
}

/// A rendered frame that stayed on the GPU: the number naming the surface it
/// lives in, plus its dimensions and format (K-177, K-195). Handed across the
/// bridge so the runner can register the texture with Flutter without any pixel
/// copy. The number stays valid across frames (the same texture is re-used) and
/// only changes when the comp is resized.
///
/// Windows and macOS share this shape because their payloads are genuinely the
/// same: one opaque integer naming a piece of graphics memory, plus its size.
/// Only what the integer *is* differs — an NT handle there, an `IOSurfaceID`
/// here — and neither side does anything with it but pass it on. Linux needs
/// more (stride, offset, DRM format) and has its own [`SharedFrameInfoLinux`].
#[cfg(any(
    all(windows, feature = "shared-texture"),
    all(target_os = "macos", feature = "shared-texture-macos")
))]
pub struct SharedFrameInfo {
    /// The NT `HANDLE` value of the shared texture on Windows (a
    /// `kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle` surface); the
    /// `IOSurfaceID` on macOS, which the runner passes to `IOSurfaceLookup`.
    pub handle: u64,
    pub width: u32,
    pub height: u32,
    /// Always RGBA8888 (sRGB-encoded bytes in a BGRA-ordered surface), the
    /// identical pixels every other path produces.
    pub format: &'static str,
}

/// A rendered frame that stayed on the GPU as a DMA-BUF (the Linux zero-copy
/// Viewer path, K-177): the exported file descriptor plus the dimensions, stride,
/// offset and DRM format/modifier the GTK embedder needs to import it as an
/// `EGLImage`. The Linux sibling of [`SharedFrameInfo`]. The fd stays valid
/// across frames (the same texture is re-used) and only changes when the comp is
/// resized.
#[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
pub struct SharedFrameInfoLinux {
    pub fd: i32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub offset: u32,
    /// The DRM fourcc (`DRM_FORMAT_ABGR8888`, memory order R,G,B,A).
    pub drm_fourcc: u32,
    /// The DRM modifier (`DRM_FORMAT_MOD_LINEAR` = 0 on the linear-tiling path).
    pub modifier: u64,
}

/// The inputs one export needs beyond the document itself: the comp's audio
/// jobs, mixed exactly as playback mixes them. The exporter builds its own
/// renderer and drives the same walk the Viewer does (K-031), so nothing else
/// crosses.
pub struct ExportInputs {
    pub audio: Vec<AudioJob>,
}

/// The VRAM cache's default byte budget: 512 MiB (~60 full-res 1080p display
/// textures, proportionally more at any preview scale). Settings →
/// Performance overrides it through the bridge.
pub const DEFAULT_VRAM_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// The preview scale as a small integer: thousandths of the scale the composite
/// actually ran at. Not part of any cache key any more (the content hash covers
/// quality) — it travels with a frame as *provenance*, so a consumer that thinks
/// in positions rather than hashes can still say "the finest held picture of
/// this frame" (the Scopes panel, which needs the numbers in a frame at any
/// size).
#[must_use]
pub fn preview_scale_q(quality: Quality) -> u16 {
    (composite_scale(quality) * 1000.0)
        .round()
        .clamp(0.0, 65535.0) as u16
}

/// Where a cached frame came from: the position and preview scale it was made
/// for. Deliberately NOT part of its name — two positions with identical content
/// share one entry, and this then records whichever asked for it first — but kept
/// because a hash alone cannot answer "is there any picture of frame 12?".
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameProvenance {
    pub comp: Uuid,
    pub frame: u64,
    /// Preview scale in thousandths — see [`preview_scale_q`].
    pub scale_q: u16,
    /// The quality the frame was made at, which is what makes the position
    /// answerable *and* checkable: a positional consumer can recompute the
    /// content name this position has now at this quality, and so tell the
    /// picture of frame 12 from a picture frame 12 used to show (K-330).
    pub quality: Quality,
}

/// A frame's name in the VRAM cache: its content hash, plus the channel order
/// the display encode ran in (a BGRA frame cannot stand in for an RGBA one, and
/// one build uses one order, so this only ever holds one value in practice).
type FrameTextureKey = (u128, bool);

/// How many demotion read-backs may be in flight at once. Each holds a staging
/// buffer the size of one frame, so this is the ladder's memory ceiling (four
/// 1080p frames ≈ 32 MB). A burst of evictions beyond it simply drops the extra
/// frames, which costs a re-render and nothing else.
///
/// **This cap, rather than a cost threshold, is what bounds the ladder's
/// traffic — a deliberate deviation from docs/06 §5.3.** The spec says to demote
/// only when a frame's recompute cost exceeds the cost of reading it back, and
/// that is the right idea; the trouble is that the number available to compare is
/// not the frame's cost. A composite is *submitted* to the graphics card and the
/// call returns, so the wall-clock the renderer can measure around it is the
/// submit, not the work — a frame that takes the card 8 ms can measure under one.
/// A threshold on that would gate the ladder on noise. The read-back costs the
/// worker no waiting at all now (it is encoded and collected later), so the real
/// cost is bus traffic, and a hard ceiling on how much of it can be in flight
/// bounds that directly and honestly. The cost hint is still measured and still
/// used — for eviction *ordering*, which is comparative and where a noisy number
/// is good enough.
const MAX_DEMOTIONS_IN_FLIGHT: usize = 4;

/// How many display textures are kept for re-use after they leave the VRAM
/// cache ([`HeadlessRenderer::upload_frame_texture`]).
///
/// Four, because these textures are not counted against the VRAM budget: they
/// are memory on the card that the meter does not show. Four frames is 32 MB at
/// 1080p, which is small beside the default budget, and it is more than enough
/// for the promotions of one playback pass — a pass promotes one frame at a
/// time, and the texture of the frame before it is usually free again.
const MAX_POOLED_TEXTURES: usize = 4;

/// One frame on its way out of VRAM and down to the tiers below: the read-back
/// is already running on the card and nobody is waiting for it.
struct Demotion {
    key: u128,
    bgra: bool,
    /// The cost that earned it the trip, carried on so the tier below can rank
    /// it against its own contents.
    cost_ms: u32,
    provenance: FrameProvenance,
    pending: lumit_gpu::PendingReadback,
    /// True when the frame is still on the card and this is a *copy* for the
    /// tiers below, not a frame on its way out ([`HeadlessRenderer::start_backup`]).
    /// The frame is then marked as held below when the copy lands, so a later
    /// eviction does not read the same pixels a second time.
    backup: bool,
}

/// A frame that has finished coming down off the graphics card — the payload the
/// owner files into the RAM tier and parks on disk (docs/06 §5.1's ladder).
pub struct DemotedFrame {
    /// The frame's content hash: the same name every tier files it under, which
    /// is what lets a frame come back up without anyone knowing where it went.
    pub key: u128,
    pub width: u32,
    pub height: u32,
    /// Display-encoded bytes in the channel order they were composited in — see
    /// [`Self::bgra`].
    pub rgba: Vec<u8>,
    /// True when `rgba` is really BGRA (the Windows/macOS zero-copy order). The
    /// bytes are kept in the order they came down so the trip back up needs no
    /// swizzle; anything that wants one canonical order (the disk tier's file
    /// format) converts on its own thread.
    pub bgra: bool,
    /// What the frame cost to render, in milliseconds.
    pub cost_ms: u32,
    /// The position and scale it was made for, so the tier below can be asked
    /// positional questions (see [`FrameProvenance`]).
    pub provenance: FrameProvenance,
}

/// One frame on its way back UP the ladder: bytes held below, and everything
/// needed to file the texture they become
/// ([`HeadlessRenderer::upload_frame_texture`]).
pub struct Promotion<'a> {
    /// The frame's content hash — the name every tier files it under.
    pub key: u128,
    /// The channel order `bytes` are in (see [`DemotedFrame::bgra`]).
    pub bgra: bool,
    pub width: u32,
    pub height: u32,
    /// Display-encoded bytes, exactly `width * height * 4` of them.
    pub bytes: &'a [u8],
    /// What the frame cost to make, so it keeps its place in the cost-aware
    /// eviction order up here.
    pub cost_ms: u32,
    pub provenance: FrameProvenance,
}

impl DemotedFrame {
    /// This frame as a promotion, for putting it straight back on the card.
    #[must_use]
    pub fn promotion(&self) -> Promotion<'_> {
        Promotion {
            key: self.key,
            bgra: self.bgra,
            width: self.width,
            height: self.height,
            bytes: &self.rgba,
            cost_ms: self.cost_ms,
            provenance: self.provenance,
        }
    }
}

/// One cached display texture. Costed by its pixel footprint — display
/// textures are 4 bytes per pixel in either channel order.
struct FrameTexture {
    texture: std::sync::Arc<wgpu::Texture>,
    provenance: FrameProvenance,
    /// True when this frame is known to be held below as well — because it
    /// arrived by being promoted UP the ladder
    /// ([`HeadlessRenderer::upload_frame_texture`]), or because the idle backup
    /// has since copied it down ([`HeadlessRenderer::start_backup`]). Evicting
    /// one of these needs no read-back, which is what stops a scrub over a span
    /// larger than the cache from reading the same frames off the card again
    /// and again.
    from_lower_tier: bool,
    /// What the frame cost to make, in milliseconds — kept beside the texture so
    /// a copy made for the tiers below can carry the same ranking the store
    /// evicts by. The cache keeps its own copy of this for eviction; this one is
    /// for the frames that leave by being *copied* rather than evicted, which
    /// never go through the eviction path that reports it.
    cost_ms: u32,
}

impl lumit_cache::ByteSized for FrameTexture {
    fn byte_size(&self) -> usize {
        (self.texture.width() as usize) * (self.texture.height() as usize) * 4 + 64
    }
}

/// One source decode a prefetcher should perform ahead of the playhead:
/// exactly what the render's own decode would ask for, so filing the result
/// under [`HeadlessRenderer::preload_decoded`] makes that render a cache hit.
pub struct PrefetchWant {
    pub item: Uuid,
    pub path: PathBuf,
    pub frame: usize,
    pub target_width: Option<u32>,
}

/// A frame composited and display-encoded but not yet shown, still on the
/// graphics card — the payload of the playback scheduler's ring buffer
/// (docs/impl/playback-scheduler.md §5). Rendering and presenting used to be
/// one call (`render_to_shared`); splitting them is what lets the worker
/// render frames AHEAD of the clock and present each one only when it is due,
/// so one slow frame spends the slack the cheap frames before it banked.
/// Holding one costs its texture's VRAM and nothing else; dropping it frees
/// that. It is only valid on the renderer that made it.
pub struct PreparedFrame {
    /// A share of the cached texture, not a texture of its own. The share is
    /// what tells the pool of textures for re-use that a present still needs
    /// this one ([`HeadlessRenderer::upload_frame_texture`]).
    texture: std::sync::Arc<wgpu::Texture>,
}

impl PreparedFrame {
    /// The frame's ACTUAL pixel dimensions — the comp size times the preview
    /// scale the composite ran at, not the logical comp size. Every present
    /// path sizes its shared surface off these, so a coarser tier shares a
    /// genuinely smaller texture.
    ///
    /// Public and platform-independent on purpose: it is the one thing a
    /// present path can ask a prepared frame without owning a transport, so
    /// the field has a reader on macOS too, which has no transport yet
    /// (K-033/K-183).
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
}

impl HeadlessRenderer {
    /// Build a headless renderer, acquiring a GPU adapter and compiling the
    /// shader engines. `Err` when no adapter exists (the bridge turns this into
    /// its "no adapter" state) or the device request fails.
    pub fn new() -> Result<Self, String> {
        let gpu = lumit_gpu::GpuContext::headless().map_err(|e| e.to_string())?;
        let parts = Parts {
            colour: lumit_gpu::ColourEngine::new(&gpu),
            compositor: lumit_gpu::Compositor::new(&gpu),
            fx: lumit_gpu::fx::FxEngine::new(&gpu),
            lut_cache: std::cell::RefCell::new(crate::fxops::LutCache::default()),
            fx_cache: std::cell::RefCell::new(crate::fxops::FxCache::default()),
        };
        let scope = lumit_gpu::scope::ScopeEngine::new(&gpu);
        // Flow runs on this same device rather than opening one of its own
        // (K-331); the handles are reference-counted, so this shares it.
        let pool = DecodePool::with_gpu(&gpu);
        Ok(Self {
            gpu,
            parts: Some(parts),
            scope,
            items: HashMap::new(),
            probe_cache: HashMap::new(),
            audio_jobs: AudioJobsBuilder::new(),
            pool,
            retained: None,
            frame_textures: {
                let mut lru = lumit_cache::ByteLru::new(DEFAULT_VRAM_CACHE_BYTES);
                // Evictions have to be visible, or the tiers below never hear
                // that a frame exists and the ladder is a drop (docs/06 §5.3).
                lru.collect_evictions();
                lru
            },
            demotions: Vec::new(),
            upload_pool: Vec::new(),
            frame_texture_version: 0,
            frame_texture_hits: 0,
            progress: None,
            profile: None,
            watching: false,
            measuring: false,
            view: lumit_gpu::DisplayParams::NEUTRAL,
            transparent_background: false,
            region: None,
            #[cfg(all(windows, feature = "shared-texture"))]
            shared: Vec::new(),
            #[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
            shared_dmabuf: Vec::new(),
            #[cfg(all(target_os = "macos", feature = "shared-texture-macos"))]
            shared_iosurface: Vec::new(),
        })
    }

    /// Install (or remove) the sink that hears how far each frame has got —
    /// the Viewer's progress bar (docs/13 §7.1). Installed once for the
    /// session; which frames actually report is [`Self::watch_frames`].
    pub fn set_progress_sink(&mut self, sink: Option<crate::profile::ProgressSink>) {
        self.progress = sink;
    }

    /// Install (or remove) the sink that hears what each measured frame cost,
    /// per layer and per effect — the render-time indicators.
    pub fn set_profile_sink(&mut self, sink: Option<crate::profile::ProfileSink>) {
        self.profile = sink;
    }

    /// Should the frames from here on report their progress? Off during
    /// playback, on for a scrub or a value drag — the renderer cannot tell
    /// which it is being driven for, so the driver says.
    pub fn watch_frames(&mut self, watching: bool) {
        self.watching = watching;
    }

    /// Should the frames from here on be measured? This one is not free: a
    /// measured frame fences the graphics card at every node (see
    /// `crate::profile`), so it is on only while something is showing the
    /// numbers, and never during playback.
    pub fn measure_frames(&mut self, measuring: bool) {
        self.measuring = measuring;
    }

    /// Whether the renders from here on may add to the per-effect cache
    /// (K-421). On for a committed document's scrub and edit renders; off for
    /// a drag's provisional values and for playback, which would only churn
    /// the budget. Lookups happen either way.
    pub fn keep_effect_outputs(&mut self, keep: bool) {
        if let Some(parts) = self.parts.as_ref() {
            parts.fx_cache.borrow_mut().keep_outputs(keep);
        }
    }

    /// Resize the per-effect cache (K-421), evicting down to the new budget.
    pub fn set_effect_cache_budget(&mut self, bytes: usize) {
        if let Some(parts) = self.parts.as_ref() {
            parts.fx_cache.borrow_mut().set_budget(bytes);
        }
    }

    /// `(used_bytes, budget_bytes, entries)` of the per-effect cache, and
    /// `(kernels run, ops served held)` since the renderer was made.
    #[must_use]
    pub fn effect_cache_stats(&self) -> ((usize, usize, usize), (u64, u64)) {
        self.parts.as_ref().map_or(((0, 0, 0), (0, 0)), |parts| {
            let c = parts.fx_cache.borrow();
            (c.stats(), c.counts())
        })
    }

    /// `(nested frames realised, nested frames served held)` since the
    /// renderer was made (K-422).
    #[must_use]
    pub fn nested_frame_counts(&self) -> (u64, u64) {
        self.parts
            .as_ref()
            .map_or((0, 0), |parts| parts.fx_cache.borrow().nested_counts())
    }

    /// Whether the frames from here on are being measured **and** there is
    /// somewhere for the numbers to go.
    ///
    /// The caller that owns the tiers above this renderer asks, because only a
    /// *composited* frame yields numbers: a frame served from a cache costs
    /// nothing and therefore reveals nothing. The cache is still allowed to
    /// answer a measured request (K-420) — the caller notes that it did, and
    /// composites the frame again for its numbers when the editor is idle.
    #[must_use]
    pub fn measuring(&self) -> bool {
        self.measuring && self.profile.is_some()
    }

    /// The recorder for one frame, or `None` when this frame is neither
    /// watched nor measured — in which case the render walks exactly as it did
    /// before the profiler existed.
    fn profiler_for(&self, comp: Uuid, frame: u64) -> Option<crate::profile::FrameProfiler> {
        let watching = self.watching && self.progress.is_some();
        let measuring = self.measuring && self.profile.is_some();
        (watching || measuring).then(|| {
            crate::profile::FrameProfiler::new(
                comp,
                frame,
                watching.then(|| self.progress.clone()).flatten(),
                measuring,
            )
        })
    }

    /// Build the inputs one export of `comp_id` needs (the bridge's v0.4 export
    /// path, K-175): the footage [`ItemInfo`] map (probed exactly as a render
    /// probes, sharing this renderer's cache), the comp's audio jobs, and a GPU
    /// context sharing this renderer's device. `None` when `comp_id` is unknown.
    /// The exporter (`crate::export::start`) takes these and spawns its own
    /// encode thread (K-017), so this call is cheap and holds no GPU work.
    pub fn export_inputs(&mut self, doc: &Document, comp_id: Uuid) -> Option<ExportInputs> {
        let comp = doc.comp(comp_id)?;
        let audio = self.collect_audio(doc, comp);
        Some(ExportInputs { audio })
    }

    /// Collect `comp`'s audio jobs for export — see [`AudioJobsBuilder`], which
    /// this renderer holds so the has-audio probe cache warms across a session.
    fn collect_audio(&mut self, doc: &Document, comp: &Composition) -> Vec<AudioJob> {
        self.audio_jobs.audio_jobs(doc, comp)
    }

    /// Set the Viewer's exposure and tone map for every frame this renderer
    /// composites from here on (K-314). Preview only — see [`Self::view`].
    ///
    /// A non-neutral view **names its frames differently** rather than leaving
    /// them nameless (K-346, superseding that part of K-314): the look is baked
    /// into the display-encoded pixels these tiers hold, so a frame under one
    /// is a different picture and takes a different name. Refusing a name
    /// instead switched every tier off for as long as a control was engaged,
    /// which is a whole session for anyone who works with the tone map on.
    /// Neutral is unchanged and keeps the names it always had, so frames banked
    /// before this still come back. An export is always neutral, so a graded
    /// preview frame can never be served as one.
    pub fn set_display_view(&mut self, view: lumit_gpu::DisplayParams) {
        self.view = view;
    }

    /// Leave the fronted comp's background colour out of the composite, so
    /// pixels nothing covers stay transparent and the Viewer's transparency
    /// grid shows through them (K-352). A way of looking, like the display
    /// view above — the export renderer never has this called on it, so an
    /// export always draws the backdrop.
    ///
    /// The flag is folded into the frame's name (see [`Self::named_under_view`]):
    /// the two backdrops are two different pictures, and a frame banked under
    /// one must never be served as the other.
    pub fn set_transparent_background(&mut self, transparent: bool) {
        self.transparent_background = transparent;
    }

    /// Composite only a sub-rectangle of the fronted comp — the Viewer's
    /// **region of interest** (K-362, docs/07 §2.2). Given as fractions of the
    /// comp (`[u0, v0, u1, v1]`, top-left to bottom-right) so the caller never
    /// has to know which raster the engine will settle on; `None` clears it.
    ///
    /// A way of looking, like the two above: the export renderer is never sent
    /// one, which is what keeps a region from ever reaching a file. The flag is
    /// folded into the frame's name — a cropped frame is not the full frame,
    /// and serving one for the other would put a corner of the picture on
    /// screen as though it were all of it.
    ///
    /// Degenerate input (inverted, empty, out of range, not finite) clears the
    /// region rather than faulting: a drag that ends where it began is a
    /// gesture, not an error.
    pub fn set_region(&mut self, region: Option<[f32; 4]>) {
        self.region = region.filter(|r| {
            r.iter().all(|v| v.is_finite())
                && r[0] >= 0.0
                && r[1] >= 0.0
                && r[2] <= 1.0
                && r[3] <= 1.0
                && r[2] - r[0] > 1e-3
                && r[3] - r[1] > 1e-3
                // A region covering the whole comp is no region at all, and
                // saying so keeps its frames sharing the full frame's names.
                && (r[0] > 0.0 || r[1] > 0.0 || r[2] < 1.0 || r[3] < 1.0)
        });
    }

    /// The region of interest, as fractions of the comp; `None` is the whole
    /// frame.
    #[must_use]
    pub fn region(&self) -> Option<[f32; 4]> {
        self.region
    }

    /// The region in comp pixels for a `w`×`h` composition, rounded out to
    /// whole pixels so the window never lands on a fraction of one.
    fn region_px(&self, w: u32, h: u32) -> Option<lumit_gpu::Region> {
        let r = self.region?;
        let (fw, fh) = (w as f32, h as f32);
        let x = (r[0] * fw).floor().clamp(0.0, fw - 1.0);
        let y = (r[1] * fh).floor().clamp(0.0, fh - 1.0);
        let rw = (r[2] * fw).ceil().clamp(1.0, fw) - x;
        let rh = (r[3] * fh).ceil().clamp(1.0, fh) - y;
        (rw >= 1.0 && rh >= 1.0).then_some(lumit_gpu::Region { x, y, w: rw, h: rh })
    }

    /// A content name with the Viewer's own way of looking folded in.
    ///
    /// Neutral returns the name untouched — byte-for-byte what
    /// [`crate::cache::frame_key`] gave — so nothing already banked is
    /// orphaned, on disk least of all. Anything else mixes the look in through
    /// the same hash the name was built with, under its own tag so a look can
    /// never be confused for content.
    fn named_under_view(&self, base: u128) -> u128 {
        if self.view.is_neutral() && !self.transparent_background && self.region.is_none() {
            return base;
        }
        let mut h = blake3::Hasher::new();
        h.update(b"view/");
        h.update(&base.to_le_bytes());
        h.update(&self.view.gain.to_bits().to_le_bytes());
        h.update(&[
            u8::from(self.view.tone_map),
            u8::from(self.transparent_background),
        ]);
        // The region (K-362). A cropped frame is a different picture of a
        // different size, so it takes a different name — which is exactly what
        // lets scrubbing inside a region use the cache at all, rather than
        // refusing to name frames while one is set.
        if let Some(r) = self.region {
            h.update(b"roi/");
            for v in r {
                h.update(&v.to_bits().to_le_bytes());
            }
        }
        let bytes = h.finalize();
        let mut k = [0u8; 16];
        k.copy_from_slice(&bytes.as_bytes()[..16]);
        u128::from_le_bytes(k)
    }

    /// What the Viewer is currently looking through.
    #[must_use]
    pub fn display_view(&self) -> lumit_gpu::DisplayParams {
        self.view
    }

    /// Let this renderer make a Lens flare's bake beside the frame rather than
    /// inside it (K-350), so choosing a lens is a wait you can watch instead of
    /// half a second of stopped picture.
    ///
    /// **Off by default, and the exporter never turns it on.** An export builds
    /// its own renderer on its own device, so it starts with an empty bake
    /// cache and bakes inside the frame exactly as it always did — which is
    /// what keeps K-031's preview-equals-export identity true and an export
    /// bit-for-bit what it was. The Viewer's renderer turns it on.
    ///
    /// A frame drawn with the previous lens must not be filed under a name
    /// that says it was drawn with this one, so such a frame is made and not
    /// kept — see [`Self::flare_substitutions`], which says exactly which
    /// frames those were (K-431).
    pub fn set_deferred_flare_bakes(&self, deferred: bool) {
        if let Some(parts) = self.parts.as_ref() {
            parts.fx.set_deferred_flare_bakes(deferred);
        }
    }

    /// A number that moves whenever a flare bake is queued or lands.
    ///
    /// Read either side of a render to answer "did this frame draw the lens its
    /// parameters name?", and read on an idle tick to notice that a bake has
    /// landed and the picture is now worth making again.
    #[must_use]
    pub fn flare_bake_generation(&self) -> u64 {
        self.parts
            .as_ref()
            .map_or(0, |parts| parts.fx.flare_bake_generation())
    }

    /// How many times a frame has drawn a lens flare with other optics than
    /// its parameters name (K-431). Read either side of a render: unmoved
    /// means the frame may be banked under the name taken before it.
    #[must_use]
    pub fn flare_substitutions(&self) -> u64 {
        self.parts
            .as_ref()
            .map_or(0, |parts| parts.fx.flare_substitutions())
    }

    /// Whether a flare bake is being made right now.
    #[must_use]
    pub fn flare_bake_pending(&self) -> bool {
        self.parts
            .as_ref()
            .is_some_and(|parts| parts.fx.flare_bake_pending())
    }

    /// The content-hash name of this comp frame ([`crate::cache::frame_key`]),
    /// computed from **this renderer's own** probe results so the name and the
    /// pixels can never disagree about what a source file is. `None` while some
    /// footage is unprobed — the frame renders live and is not cached. The
    /// Viewer's own way of looking is folded in ([`Self::named_under_view`]),
    /// so an exposed frame is named as one rather than left nameless.
    ///
    /// Takes `&mut self` because it probes anything new, exactly as a render
    /// would; a caller that then renders pays for the probe only once.
    pub fn frame_key(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
    ) -> Option<u128> {
        // A flare bake in flight used to make this answer `None` (K-350) —
        // for every comp, whether or not it held a flare. A keyframed
        // aperture keeps a bake in flight for as long as it plays, so that
        // rule stopped the whole project caching (K-431). The name is taken
        // here and the frame is *checked* afterwards instead: see
        // [`Self::flare_substitutions`], which counts the frames that
        // actually drew other optics than they name, and those alone are the
        // frames nobody banks.
        let comp = doc.comp(comp_id)?;
        self.sync_items(doc, comp);
        crate::cache::frame_key(
            doc,
            comp,
            frame as usize,
            quality,
            &ProbeView(&self.probe_cache),
        )
        .map(|k| self.named_under_view(k))
    }

    /// Probe what comp `comp_id` can show, so a batch of
    /// [`Self::frame_key_presynced`] calls can run against a settled probe
    /// cache. [`Self::frame_key`] does this itself, per call — which rebuilds
    /// the footage map every time, and a consumer naming hundreds of frames of
    /// the SAME document (the cache bar, the playback look-ahead) was paying
    /// that rebuild per frame. Call this once per document, then name as many
    /// frames as needed. An unknown comp probes nothing, calmly.
    pub fn presync_items(&mut self, doc: &Document, comp_id: Uuid) {
        if let Some(comp) = doc.comp(comp_id) {
            self.sync_items(doc, comp);
        }
    }

    /// [`Self::frame_key`] against the probes already gathered — no probing, no
    /// footage-map rebuild, and thus `&self`. Only correct after
    /// [`Self::presync_items`] was called for this document and this comp; an
    /// unprobed source simply makes the frame unnameable (`None`), never
    /// wrongly named, so a caller that forgets the presync renders live rather
    /// than mis-caching.
    #[must_use]
    pub fn frame_key_presynced(
        &self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
    ) -> Option<u128> {
        let comp = doc.comp(comp_id)?;
        crate::cache::frame_key(
            doc,
            comp,
            frame as usize,
            quality,
            &ProbeView(&self.probe_cache),
        )
        .map(|k| self.named_under_view(k))
    }

    /// Composite one interactive frame and return the display-encoded GPU
    /// texture — the shared body of both interactive entry points. The texture
    /// is at the comp's dimensions times the preview scale (`quality`'s
    /// display scale under auto resolution — see [`composite_scale`]): the
    /// composite itself runs on the smaller raster, which is where a coarser
    /// preview actually gets cheaper. The returned pair is the LOGICAL comp
    /// dims; the texture's own `width()`/`height()` are the actual ones.
    ///
    /// Its callers differ only in what they do with the texture: read it back to
    /// bytes ([`Self::render_preview`]) or copy it into a texture the frontend
    /// samples directly ([`Self::render_to_shared`]). So both show the
    /// same pixels, and both get the drag fast path.
    fn preview_display_texture(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
    ) -> Result<(wgpu::Texture, u32, u32), String> {
        self.preview_display_texture_fmt(doc, comp_id, frame, quality, false)
    }

    /// [`Self::preview_display_texture`] with the output channel order chosen:
    /// `bgra` is for the shared-texture Viewer only (see `render_to_shared`).
    fn preview_display_texture_fmt(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        bgra: bool,
    ) -> Result<(wgpu::Texture, u32, u32), String> {
        let comp = doc
            .comp(comp_id)
            .ok_or_else(|| "headless preview: unknown composition".to_string())?;
        let (cw, ch) = (comp.width, comp.height);
        // Fills `probe_cache` for anything new this comp can show, which
        // `ProbeView` then reads.
        self.sync_items(doc, comp);
        let fps = comp.frame_rate.fps().max(1.0);
        let t = frame as f64 / fps;

        // The frame's recorder: absent unless somebody is drawing a bar for
        // this frame or reading its numbers (docs/13 §7.1).
        let watcher = self.profiler_for(comp_id, frame);
        // The nested-frame store (K-422): what the builder names each Precomp
        // by, and what the planner asks before decoding into one. Both use
        // the one keyer, so the name the plan found held is the name the
        // realiser asks for. A measured frame realises every Precomp so its
        // inner rows get numbers (see `Realiser::realise_nested`), so it must
        // not skip their decodes either.
        let probes = ProbeView(&self.probe_cache);
        let keys = crate::cache::NestedKeys {
            doc,
            probes: &probes,
            quality,
        };
        let jobs = {
            let held = |nested: &Composition, lt: f64| -> bool {
                let Some(parts) = self.parts.as_ref() else {
                    return false;
                };
                let Some(key) = crate::cache::NestedKeyer::nested_key(&keys, nested, lt) else {
                    return false;
                };
                let scale = composite_scale(quality);
                let samples = self.gpu.sample_count(doc.anti_aliasing.samples());
                parts
                    .fx_cache
                    .borrow_mut()
                    .pin_nested(crate::fxops::nested_texture_key(key, scale, samples))
            };
            if let Some(parts) = self.parts.as_ref() {
                parts.fx_cache.borrow_mut().unpin_nested();
            }
            let held: Option<crate::plan::HeldNested<'_>> =
                if watcher.is_none() { Some(&held) } else { None };
            crate::plan::plan_comp_frame_held(doc, comp, t, quality, &probes, held)
        };
        if let Some(w) = &watcher {
            w.planned();
        }
        // The whole point: decode only when the wanted pixels actually differ.
        let reusable = matches!(
            &self.retained,
            Some(r) if r.comp == comp_id
                && r.frame == frame
                && crate::plan::same_decode(&r.jobs, &jobs)
        );
        if !reusable {
            let total = jobs.len() as u32;
            let pixels = self
                .pool
                .decode_comp(comp_id, frame as usize, &jobs, 0, &|done| {
                    if let Some(w) = &watcher {
                        w.decoded(done as u32, total);
                    }
                })
                .map_err(|e| format!("headless preview: {e}"))?;
            self.retained = Some(Retained {
                comp: comp_id,
                frame,
                jobs,
                pixels,
            });
        }
        let Some(retained) = self.retained.as_ref() else {
            return Err("headless preview: no decoded pixels".into());
        };

        let Some(parts) = self.parts.take() else {
            return Err("headless preview: renderer is unavailable after an earlier fault".into());
        };
        let out = {
            let realiser = crate::realise::Realiser {
                ctx: self.gpu.clone_handle(),
                engine: &parts.colour,
                compositor: &parts.compositor,
                fx: &parts.fx,
                lut_cache: &parts.lut_cache,
                fx_cache: &parts.fx_cache,
                render_scale: composite_scale(quality),
                // The project's setting, resolved against what this adapter
                // will actually give (K-274). Preview and export both read the
                // same document field — unlike `render_scale`, which is a
                // preview-only reduction — so the two stay the same picture.
                samples: self.gpu.sample_count(doc.anti_aliasing.samples()),
                profiler: watcher.as_ref(),
            };
            let pixels_by_layer: HashMap<Uuid, &crate::decode::CompLayerPixels> = retained
                .pixels
                .layers
                .iter()
                .map(|lp| (lp.layer, lp))
                .collect();
            let mut visited = vec![comp_id];
            if let Some(w) = &watcher {
                w.building();
            }
            let draws = crate::build::build_comp_draws_at(
                doc,
                comp,
                t,
                t,
                &pixels_by_layer,
                &mut visited,
                Some(&keys),
                false,
            );
            // The comp's backdrop is a way of viewing, not a layer (K-241);
            // with the transparency grid up the Viewer asks for none at all,
            // so what nothing covers arrives with zero alpha and the grid
            // shows through it (K-352).
            let background = if self.transparent_background {
                [0.0; 4]
            } else {
                comp.background.0.map(f64::from)
            };
            if let Some(w) = &watcher {
                w.compositing(draws.len() as u32);
            }
            // The region of interest (K-362): composite only the window the
            // Viewer asked for. `realise_region` refuses it — and composites
            // the whole frame — where an adjustment or a motion-blurring layer
            // stages through a comp-sized intermediate, so the picture is the
            // same either way and only the work differs. The crop below then
            // makes the returned texture the region's size regardless, so
            // everything downstream sees one shape.
            let roi = self.region_px(cw, ch);
            let linear = realiser.realise_region(
                crate::track::camera_pose(doc, comp, t),
                cw,
                ch,
                background,
                &draws,
                roi,
            );
            let linear = match roi {
                Some(r) if linear.width() != r.target_size(realiser.render_scale).0 => {
                    crop_texture(&self.gpu, &linear, r, realiser.render_scale, (cw, ch))
                }
                _ => linear,
            };
            if let Some(w) = &watcher {
                w.presenting();
            }
            // The one place the Viewer's own way of looking is applied: on the
            // linear composite, on its way to display bytes (docs/06 §3.3).
            Ok(if bgra {
                parts.colour.display_bgra(&self.gpu, &linear, self.view)
            } else {
                parts.colour.display(&self.gpu, &linear, self.view)
            })
        };
        // Return the engines to the pool even on error, so one failed frame does
        // not discard the compiled shaders.
        self.parts = Some(parts);
        // A frame that faulted is not published as a measurement: half a walk's
        // numbers would read as a comp that got cheaper.
        if out.is_ok() {
            if let (Some(profile), Some(sink)) =
                (watcher.and_then(|w| w.finish()), self.profile.as_ref())
            {
                sink(profile);
            }
        }
        out.map(|shown| (shown, cw, ch))
    }

    /// The interactive render: composition `comp_id` at integer `frame`, read
    /// back to tightly-packed RGBA8 as `(pixels, width, height)`.
    ///
    /// This is the path a Viewer should drive. Unlike [`Self::render_rgba`] it:
    ///
    /// - **decodes at the preview resolution** `quality` asks for, so a source
    ///   shown in a small viewport is decoded small rather than in full and then
    ///   thrown away;
    /// - **reuses the pixels it already has** whenever the decode plan has not
    ///   changed. Dragging a transform or effect value alters what is done with
    ///   the footage, never which frame of it is wanted, so a drag composites
    ///   from the retained pixels and touches no file at all. That is what makes
    ///   a value drag feel live rather than stuttery.
    ///
    /// A Retime drag is the one live edit that *does* change the decode — it
    /// moves to a different source frame — so it cannot ride the retained
    /// pixels. It arrives as a patched document like any other provisional
    /// value, and the plan below re-reads the map from it. (A bespoke override
    /// parameter for this existed here, threaded through every caller and
    /// constructed by none of them; K-249 removed it.)
    ///
    /// The document handed in may be a throwaway with a drag's provisional value
    /// already patched in; nothing is cached against its identity here, so that
    /// costs nothing. `scale` shrinks the returned buffer for the trip back to
    /// the frontend only (see [`resize_output`]).
    ///
    /// A missing layer is drawn as colour bars by the compositor itself, so the
    /// returned buffer already carries the slate.
    pub fn render_preview(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        scale: f32,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let (shown, cw, ch) = self.preview_display_texture(doc, comp_id, frame, quality)?;
        let Some(parts) = self.parts.as_ref() else {
            return Err("headless preview: renderer is unavailable after an earlier fault".into());
        };

        // The interactive path now composites at the preview scale (the
        // Realiser's `render_scale`), so on the Viewer path `shown` already IS
        // the size the frontend asked for and no second pass runs at all. The
        // resize below survives for the caller that composites full-size but
        // wants a smaller buffer back (export's letterbox path, `render_rgba`
        // with a scale) — reduced on the graphics card, before the read-back,
        // using the same `scaled_size` rounding the composite target used, so
        // the two can never disagree.
        let (sw, sh) = scaled_size(cw, ch, scale);
        if (sw, sh) == (shown.width(), shown.height()) {
            return parts
                .colour
                .readback8(&self.gpu, &shown)
                .map(|rgba| (rgba, sw, sh))
                .map_err(|e| format!("headless preview: {e}"));
        }
        // Neutral, and it must stay so: `shown` is already display-encoded, and
        // the Viewer's view was applied on the way there. Passing it again here
        // would expose the picture twice.
        let reduced = parts.colour.display_scaled(
            &self.gpu,
            &shown,
            sw,
            sh,
            lumit_gpu::DisplayParams::NEUTRAL,
        );
        parts
            .colour
            .readback8(&self.gpu, &reduced)
            .map(|rgba| (rgba, sw, sh))
            .map_err(|e| format!("headless preview: {e}"))
    }

    /// How many comp frames the interactive path has actually decoded. A live
    /// value drag must not move this — that is the whole promise of
    /// [`Self::render_preview`], and the preview tests assert it here.
    #[must_use]
    pub fn decoded_frames(&self) -> u64 {
        self.pool.comp_decodes()
    }

    /// The source decodes rendering `comp_id` at `frame` will perform — what a
    /// decode-ahead thread does early so the render itself is a cache hit
    /// (docs/impl/playback-scheduler.md §5, decode ∥ evaluate). Runs the same
    /// plan the render will run, so the two cannot want different frames.
    /// Slated (missing) media wants nothing — there is nothing to decode.
    pub fn prefetch_wants(
        &mut self,
        doc: &Document,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
    ) -> Vec<PrefetchWant> {
        let Some(comp) = doc.comp(comp_id) else {
            return Vec::new();
        };
        self.sync_items(doc, comp);
        let fps = comp.frame_rate.fps().max(1.0);
        let t = frame as f64 / fps;
        let jobs = plan_comp_frame(doc, comp, t, quality, &ProbeView(&self.probe_cache));
        let mut wants = Vec::new();
        for job in &jobs {
            if job.slate {
                continue;
            }
            let mut want = |frame: usize| {
                wants.push(PrefetchWant {
                    item: job.item,
                    path: job.path.clone(),
                    frame,
                    target_width: job.target_width,
                });
            };
            want(job.source_frame);
            if let Some((ceil, _)) = job.blend {
                want(ceil);
            }
            for &(_, neighbour) in &job.temporal {
                want(neighbour);
            }
        }
        wants
    }

    /// File one prefetched decode into the decoded-source cache, under exactly
    /// the key the render's own decode would use — so the render finds it and
    /// decodes nothing. Wrong or stale pixels cannot be filed under a live
    /// key: the key IS (item, source frame, decode width).
    pub fn preload_decoded(
        &mut self,
        item: Uuid,
        frame: usize,
        target_width: Option<u32>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) {
        self.pool
            .preload(item, frame, target_width, width, height, rgba);
    }

    /// Forget the retained per-layer pixels. The next [`Self::render_preview`]
    /// decodes afresh. Called when something outside the document changes what
    /// the sources *are* — a probe landing, a relink — since the decode plan
    /// alone cannot see that.
    pub fn forget_retained(&mut self) {
        self.retained = None;
    }

    /// The decoded-frame cache's bytes and how many decoders are open — see
    /// [`crate::decode::DecodePool::memory`].
    #[must_use]
    pub fn decode_memory(&self) -> (usize, usize) {
        self.pool.memory()
    }

    /// What the graphics driver holds for this renderer's device — see
    /// [`lumit_gpu::GpuContext::allocator_bytes`].
    #[must_use]
    pub fn gpu_allocator_bytes(&self) -> Option<(u64, u64)> {
        self.gpu.allocator_bytes()
    }

    /// Whether the card's memory is this process's memory — see
    /// [`lumit_gpu::GpuContext::unified_memory`].
    #[must_use]
    pub fn unified_memory(&self) -> bool {
        self.gpu.unified_memory
    }

    /// How many textures and buffers the driver is still holding for this
    /// renderer — see [`lumit_gpu::GpuContext::live_objects`].
    #[must_use]
    pub fn gpu_live_objects(&self) -> (u64, u64) {
        self.gpu.live_objects()
    }

    /// Give the driver a turn to reclaim what has been dropped — see
    /// [`lumit_gpu::GpuContext::reclaim`]. Called once per worker turn.
    pub fn reclaim_gpu(&self) {
        self.gpu.reclaim();
    }

    /// Wait for the card to catch up and then reclaim — see
    /// [`lumit_gpu::GpuContext::settle`]. For measuring what is held at rest,
    /// and for an engine with nothing left to draw; never on a frame path.
    pub fn settle_gpu(&self) {
        self.gpu.settle();
    }

    /// Resize the decoded-source-frame cache (Settings → Performance).
    pub fn set_decode_budget(&mut self, bytes: usize) {
        self.pool.set_budget(bytes);
    }

    /// Drop every cached decoded source frame and the retained pixels, keeping
    /// the open decoders (Settings → Clear cache).
    pub fn clear_decoded(&mut self) {
        self.pool.clear();
        self.retained = None;
    }

    /// Render composition `comp_id` at integer `frame` to tightly-packed RGBA8,
    /// returning `(pixels, width, height)`. `scale` of 1.0 is the comp's own
    /// resolution; a smaller positive `scale` downsamples the output.
    ///
    /// Since the comp-walk unification (K-031) this IS [`Self::render_preview`]
    /// at full decode quality — export and interactive rendering are one path
    /// by construction. The name survives for the callers and tests that mean
    /// "the frame as an export would write it".
    pub fn render_rgba(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        scale: f32,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        self.render_preview(doc, comp_id, frame, Quality::default(), scale)
    }

    /// Compute a scope trace (waveform/vectorscope/histogram, K-096 v1) from an
    /// already-rendered comp frame's display bytes, returning the `GRID × GRID`
    /// RGBA8 trace. `rgba` is the exact frame the Viewer shows (served from the
    /// bridge's rendered-frame cache, so the scope traces the same frame at no
    /// re-render cost); the binning runs on the GPU and only the tiny trace is
    /// read back.
    ///
    /// `kind` is `0` luma / `1` RGB waveform / `2` vectorscope / `3` histogram
    /// (an unknown value is a calm `Err`); `colours` carries the frontend's fixed
    /// `ScopeColours` as `[bg, trace, red, green, blue]` RGB byte triples, so no
    /// colour literal lives in the engine (docs/15-DESIGN.md) and the bridge need
    /// not name `lumit-gpu`. `Err` on an unknown kind or if the tiny readback
    /// fails.
    pub fn render_scope(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        kind: u32,
        colours: [[u8; 3]; 5],
    ) -> Result<Vec<u8>, String> {
        let kind = match kind {
            0 => lumit_gpu::scope::ScopeKind::WaveformLuma,
            1 => lumit_gpu::scope::ScopeKind::WaveformRgb,
            2 => lumit_gpu::scope::ScopeKind::Vectorscope,
            3 => lumit_gpu::scope::ScopeKind::Histogram,
            other => return Err(format!("headless scope: unknown kind {other}")),
        };
        let colours = lumit_gpu::scope::ScopeColours {
            bg: colours[0],
            trace: colours[1],
            red: colours[2],
            green: colours[3],
            blue: colours[4],
        };
        self.scope
            .trace_rgba8(&self.gpu, kind, colours, rgba, width, height)
            .map_err(|e| e.to_string())
    }

    /// Composite and display-encode one frame WITHOUT showing it — the render
    /// half of [`Self::render_to_shared`], split out so the playback scheduler
    /// can render ahead of the clock into its ring and present each frame only
    /// when it is due (docs/impl/playback-scheduler.md §5). `bgra` chooses the
    /// channel order the eventual present needs (true on the Windows
    /// shared-texture path — ANGLE only opens BGRA surfaces). Shares the
    /// interactive path, so it shares the drag fast path too.
    ///
    /// `cacheable` opts the frame into the VRAM final-frame cache: a held
    /// frame is served without compositing anything, and a rendered one is
    /// kept for next time. Pass false for any render of a document the store
    /// has not committed (a live drag's provisional values) — those pixels
    /// must neither be served stale nor poison the cache.
    ///
    /// The name a cacheable frame is filed under is its content hash, so a frame
    /// whose footage is not yet probed has no name and is simply rendered live
    /// (see [`Self::frame_key`]) — never filed under a promise it cannot keep.
    pub fn render_prepared(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        bgra: bool,
        cacheable: bool,
    ) -> Result<PreparedFrame, String> {
        let name = cacheable
            .then(|| self.frame_key(doc, comp_id, frame, quality))
            .flatten();
        self.render_prepared_named(doc, comp_id, frame, quality, bgra, name)
    }

    /// [`Self::render_prepared`] with the frame's content name already computed.
    ///
    /// A caller that has looked in the tiers below before deciding to composite
    /// has necessarily named the frame already, and naming one means hashing the
    /// whole composition at that time — cheap beside a composite, but not free,
    /// and not worth paying twice per frame. `None` means "do not cache this
    /// one", which is both what a provisional drag render wants and what an
    /// unnameable frame (footage still being probed) gets.
    pub fn render_prepared_named(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        bgra: bool,
        name: Option<u128>,
    ) -> Result<PreparedFrame, String> {
        let key = name.map(|k| (k, bgra));
        // A held frame is served whether or not this frame is being measured
        // (K-420). A cache hit has nothing to say about what the layers cost,
        // but refusing it meant a frame the bar showed green was composited
        // again — and fenced at every layer — on arrival. The owner of the
        // tiers measures such a frame afterwards, in an idle moment, rather
        // than making the user wait for the numbers.
        if let Some(key) = key {
            if let Some(held) = self.frame_textures.get(&key) {
                self.frame_texture_hits += 1;
                return Ok(PreparedFrame {
                    texture: held.texture.clone(),
                });
            }
        }
        let started = std::time::Instant::now();
        // A flare that fell back to the previous lens during this composite
        // (K-350) made a picture of a lens its name does not describe. The
        // name was taken before the render, so it has to be dropped
        // afterwards — the alternative is an entry that lies about its own
        // content, which no later edit or undo can clear (K-178). Counted, so
        // only the frames it actually happened to are dropped (K-431).
        let subs_before = self.flare_substitutions();
        let (texture, _, _) =
            self.preview_display_texture_fmt(doc, comp_id, frame, quality, bgra)?;
        let key = key.filter(|_| self.flare_substitutions() == subs_before);
        let texture = std::sync::Arc::new(texture);
        if let Some(key) = key {
            // What it actually cost, so the store's cost-aware eviction has
            // something true to weigh (docs §5.3: stale × cheap × large) and the
            // demotion below can tell a dear frame from a trivial one. Rounded up
            // to at least 1: a cost of zero would divide the eviction score by
            // nothing at all.
            let cost_ms = started.elapsed().as_millis().clamp(1, u128::from(u32::MAX)) as u32;
            self.frame_textures.insert_with_cost(
                key,
                FrameTexture {
                    texture: texture.clone(),
                    provenance: FrameProvenance {
                        comp: comp_id,
                        frame,
                        scale_q: preview_scale_q(quality),
                        quality,
                    },
                    from_lower_tier: false,
                    cost_ms,
                },
                cost_ms,
            );
            self.frame_texture_version += 1;
            self.start_demotions();
        }
        Ok(PreparedFrame { texture })
    }

    /// Start reading back whatever that insert pushed out of VRAM, so it can
    /// fall to the tiers below instead of being lost (docs/06 §5.3's demotion).
    ///
    /// Nothing here waits: each read-back is *encoded* and left running on the
    /// graphics card, which is what keeps an eviction off the preview's critical
    /// path. A frame too cheap to be worth moving is dropped, as is any beyond
    /// the in-flight ceiling.
    fn start_demotions(&mut self) {
        for ((key, bgra), evicted, cost_ms) in self.frame_textures.take_evicted() {
            // Already downstairs, or no room in flight: both mean this frame is
            // not read back, and neither loses anything but a possible re-render.
            let read_back = !evicted.from_lower_tier
                && self.demotions.len() < MAX_DEMOTIONS_IN_FLIGHT
                && self.parts.is_some();
            if read_back {
                // No engines means an earlier render faulted; there is nothing to
                // encode with, and a lost demotion only costs a re-render.
                if let Some(parts) = self.parts.as_ref() {
                    self.demotions.push(Demotion {
                        key,
                        bgra,
                        cost_ms,
                        provenance: evicted.provenance,
                        pending: parts.colour.start_readback8(&self.gpu, &evicted.texture),
                        backup: false,
                    });
                }
            }
            // The texture itself can serve the next promoted frame, whether or
            // not its pixels went downstairs. Only a texture that a promotion
            // made can: a composited frame is a render target, and the card does
            // not let you write bytes into one. A present may still hold the
            // texture; the pool tests for that before it hands one out.
            if self.upload_pool.len() < MAX_POOLED_TEXTURES
                && evicted
                    .texture
                    .usage()
                    .contains(wgpu::TextureUsages::COPY_DST)
            {
                self.upload_pool.push(evicted.texture);
            }
        }
    }

    /// Copy one held frame down to the tiers below **without evicting it** —
    /// the idle backup (docs/06 §5.5).
    ///
    /// # Why this has to exist
    ///
    /// Until now a frame reached the disk tier by exactly one route: it was
    /// pushed out of the card's cache, read back, and parked on the way down.
    /// That route needs the cache to be *full*. Give the card's cache a budget
    /// larger than a session ever fills — 10 GB, say — and it is never full,
    /// nothing is ever pushed out, and thus nothing is ever written to disk. The
    /// tier that exists to make tomorrow's session start warm stayed empty, and
    /// the bigger the budget the user gave it, the more certainly it stayed
    /// empty. That is the wrong way round.
    ///
    /// So the ladder gets a second way down, for when there is time to spare:
    /// pick a held frame that is not on disk yet, and start a read-back of it.
    /// The frame stays on the card and keeps serving the Viewer; what goes down
    /// is a copy.
    ///
    /// `parked` answers "is this frame already on disk?" — the owner's mirror of
    /// the disk tier, asked here so this crate needs no knowledge of where the
    /// frames go.
    ///
    /// Returns whether a copy was started. `false` means there is nothing left
    /// to back up, or no room in flight, which is the caller's signal to stop
    /// asking until something changes.
    pub fn start_backup(&mut self, parked: &dyn Fn(u128) -> bool) -> bool {
        if self.demotions.len() >= MAX_DEMOTIONS_IN_FLIGHT {
            return false;
        }
        let Some(parts) = self.parts.as_ref() else {
            return false;
        };
        // The first held frame that is not downstairs and is not already on its
        // way there. Order does not matter: every held frame is wanted on disk
        // eventually, and the caller comes back for the next one.
        let Some(&(key, bgra)) = self
            .frame_textures
            .keys()
            .find(|(key, _)| !parked(*key) && !self.demotions.iter().any(|d| d.key == *key))
        else {
            return false;
        };
        let Some(held) = self.frame_textures.peek(&(key, bgra)) else {
            return false;
        };
        if held.from_lower_tier {
            // Already below; nothing to copy.
            return false;
        }
        self.demotions.push(Demotion {
            key,
            bgra,
            cost_ms: held.cost_ms,
            provenance: held.provenance,
            pending: parts.colour.start_readback8(&self.gpu, &held.texture),
            backup: true,
        });
        true
    }

    /// Take a texture from the pool that the next promoted frame can use, if
    /// there is one.
    ///
    /// A texture is only free when this pool holds the last share of it. A
    /// present holds a share for as long as it can show the frame, and a write
    /// into a texture that is still on screen would show the wrong picture. The
    /// share count is thus the test, and it needs no bookkeeping of its own.
    fn take_pooled(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Option<std::sync::Arc<wgpu::Texture>> {
        let at = self.upload_pool.iter().position(|t| {
            std::sync::Arc::strong_count(t) == 1
                && t.width() == width
                && t.height() == height
                && t.format() == format
                && t.usage().contains(wgpu::TextureUsages::COPY_DST)
        })?;
        Some(self.upload_pool.swap_remove(at))
    }

    /// Collect any demotion read-backs the graphics card has finished — the
    /// frames that just left VRAM, ready for the RAM and disk tiers.
    ///
    /// Called once per worker loop turn. Cheap when nothing is in flight (an
    /// empty vector), and it never waits: a read-back still running is simply
    /// asked again next turn. A failed one is dropped without a word beyond the
    /// count — the frame is re-rendered if it is wanted, which is what a cache
    /// miss always costs.
    pub fn poll_demotions(&mut self) -> Vec<DemotedFrame> {
        let mut done = Vec::new();
        let mut still_running = Vec::with_capacity(self.demotions.len());
        for mut demotion in std::mem::take(&mut self.demotions) {
            let (width, height) = demotion.pending.size();
            match demotion.pending.poll(&self.gpu) {
                None => still_running.push(demotion),
                Some(Ok(rgba)) => {
                    // A backup's frame is still on the card. Mark it as held
                    // below now the copy is made, so the day it *is* pushed out
                    // it goes quietly instead of being read a second time.
                    if demotion.backup {
                        if let Some(held) =
                            self.frame_textures.peek_mut(&(demotion.key, demotion.bgra))
                        {
                            held.from_lower_tier = true;
                        }
                    }
                    done.push(DemotedFrame {
                        key: demotion.key,
                        width,
                        height,
                        rgba,
                        bgra: demotion.bgra,
                        cost_ms: demotion.cost_ms,
                        provenance: demotion.provenance,
                    });
                }
                Some(Err(_)) => {}
            }
        }
        self.demotions = still_running;
        done
    }

    /// Put a frame held as bytes back on the graphics card — the way UP the
    /// ladder (docs/06 §5.1: "promotes RAM→VRAM, and disk→RAM→VRAM ahead of the
    /// playhead").
    ///
    /// **This is the piece that makes the lower tiers worth having.** Until it
    /// existed nothing could turn held bytes into a texture the Viewer shows, so
    /// a frame demoted out of VRAM — or read back off disk — would have been
    /// composited again anyway, and the tiers below were bookkeeping with no
    /// payoff. Now a promoted frame is presented exactly like a freshly rendered
    /// one, and lands in the VRAM cache so the next visit is free too.
    ///
    /// `None` when the payload is not exactly one frame of the stated size — a
    /// corrupt or truncated entry is refused rather than shown as garbage.
    pub fn upload_frame_texture(&mut self, frame: Promotion<'_>) -> Option<PreparedFrame> {
        // A texture that the cache has finished with, and that nothing shows any
        // more, holds the next frame as well as a new one does. Playback goes
        // past a promoted frame each time it comes round, so a new texture for
        // each of them is an allocation on the card for each frame.
        let format = lumit_gpu::ColourEngine::display8_format(frame.bgra);
        let pooled = self.take_pooled(frame.width, frame.height, format);
        let parts = self.parts.as_ref()?;
        let texture = match pooled {
            Some(free)
                if parts.colour.write_display8(
                    &self.gpu,
                    &free,
                    frame.bytes,
                    frame.width,
                    frame.height,
                ) =>
            {
                free
            }
            // Nothing free of the correct size, or a payload the write refused:
            // make one, which also refuses a payload that is not one frame.
            _ => std::sync::Arc::new(parts.colour.upload_display8(
                &self.gpu,
                frame.bytes,
                frame.width,
                frame.height,
                frame.bgra,
            )?),
        };
        self.frame_textures.insert_with_cost(
            (frame.key, frame.bgra),
            FrameTexture {
                texture: texture.clone(),
                provenance: frame.provenance,
                from_lower_tier: true,
                cost_ms: frame.cost_ms.max(1),
            },
            frame.cost_ms.max(1),
        );
        self.frame_texture_version += 1;
        // An upload can displace something, exactly as a render can; the frame it
        // displaces has the same right to fall downstairs.
        self.start_demotions();
        Some(PreparedFrame { texture })
    }

    /// Read a held frame's pixels back off the card, as tight 8-bit bytes in
    /// the channel order it is held in — `(width, height, bytes)`.
    ///
    /// For the Scopes: a frame the Viewer is showing from the card is the frame
    /// they should trace, and reading it back is a copy where compositing it
    /// again was a render. This one waits for the card, which is the right
    /// trade for a trace — it is throttled to a few a second and was paying
    /// for a whole composite — and the wrong one for anything on the
    /// preview's critical path, which is what [`Self::start_backup`] and the
    /// demotions use the asynchronous read-back for. Does not touch the
    /// entry's eviction recency. `None` when the frame is not held, or the
    /// read-back fails.
    #[must_use]
    pub fn read_back_frame_texture(&self, key: u128, bgra: bool) -> Option<(u32, u32, Vec<u8>)> {
        let held = self.frame_textures.peek(&(key, bgra))?;
        let parts = self.parts.as_ref()?;
        let bytes = parts.colour.readback8(&self.gpu, &held.texture).ok()?;
        Some((held.texture.width(), held.texture.height(), bytes))
    }

    /// How many times the held set has changed — bumped by every insert, every
    /// clear and every resize.
    ///
    /// For mirrors of the contents (the cache bar). `(used, entries)` is not
    /// enough to notice a change: a cache sitting AT its budget swaps one frame
    /// for another of the same size, so both numbers stay put while every frame
    /// in it is different. The bar then draws yesterday's holdings for as long
    /// as the cache stays full, which reads as "the fill has stopped".
    #[must_use]
    pub fn frame_texture_version(&self) -> u64 {
        self.frame_texture_version
    }

    /// Resize the VRAM final-frame cache (Settings → Performance), evicting
    /// down to the new budget immediately.
    pub fn set_frame_texture_budget(&mut self, bytes: usize) {
        self.frame_textures.set_budget(bytes);
        self.frame_texture_version += 1;
    }

    /// Drop every cached frame texture — a committed edit changed the document
    /// and these are keyed by position, or the user asked (Clear cache).
    pub fn clear_frame_textures(&mut self) {
        self.frame_textures.clear();
        // The per-effect intermediates go with the frames (K-421): a user
        // who asked for an empty cache meant all of it.
        if let Some(parts) = self.parts.as_ref() {
            parts.fx_cache.borrow_mut().clear();
        }
        // Give the memory on the card back as well: the pool exists to make
        // promotions cheap, and after a clear there is nothing to promote.
        self.upload_pool.clear();
        self.frame_texture_version += 1;
    }

    /// `(used_bytes, budget_bytes, entries)` of the VRAM final-frame cache.
    #[must_use]
    pub fn frame_texture_stats(&self) -> (usize, usize, usize) {
        (
            self.frame_textures.used_bytes(),
            self.frame_textures.budget_bytes(),
            self.frame_textures.len(),
        )
    }

    /// Every held frame's content hash. Channel order is dropped: one platform
    /// only ever uses one.
    ///
    /// For counting and for tests. The cache bar does NOT read this: under
    /// content keying a hash does not say where its frame sits, so the bar is
    /// built by asking [`Self::has_frame_texture`] for each frame's own name (the
    /// worker does it and publishes the strip, docs/06 §5.6).
    #[must_use]
    pub fn frame_texture_keys(&self) -> Vec<u128> {
        self.frame_textures.keys().map(|&(hash, _)| hash).collect()
    }

    /// Whether the frame named `key` is already held, without touching its
    /// eviction recency — what the idle fill and the cache bar ask before
    /// rendering or drawing.
    #[must_use]
    pub fn has_frame_texture(&self, key: u128, bgra: bool) -> bool {
        self.frame_textures.contains_key(&(key, bgra))
    }

    /// How many renders the VRAM cache has answered. Test observability.
    #[must_use]
    pub fn frame_texture_hits(&self) -> u64 {
        self.frame_texture_hits
    }

    /// Render composition `comp_id` at integer `frame` into the Windows shared
    /// GPU texture, returning its NT handle and dimensions ([`SharedFrameInfo`],
    /// K-177) — the zero-copy sibling of [`Self::render_preview`]. The frame
    /// never leaves the graphics card: it is composited and display-encoded by
    /// the identical interactive path, then copied GPU-to-GPU into the shared
    /// texture instead of being read back to the CPU.
    ///
    /// Because it shares that path it also shares the drag fast path: on the
    /// shipped Windows build, dragging a value re-composites and copies without
    /// decoding or reading anything back at all.
    ///
    /// The shared texture is created on the first call and re-used across frames
    /// (a stable handle); a comp of different dimensions re-creates it and reports
    /// the new handle. `Err` on an unknown comp, when wgpu is not on the D3D12
    /// backend, or any D3D interop failure — the bridge turns that into "no
    /// shared frame" and Dart falls back to the read-back path.
    #[cfg(all(windows, feature = "shared-texture"))]
    pub fn render_to_shared(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        cacheable: bool,
    ) -> Result<SharedFrameInfo, String> {
        // BGRA, not the RGBA every other path uses: the shared texture's
        // consumer is ANGLE, which only opens BGRA share-handle surfaces.
        let prepared = self.render_prepared(doc, comp_id, frame, quality, true, cacheable)?;
        self.present_prepared(&prepared)
    }

    /// Show an already-rendered frame: copy it into the Windows shared texture
    /// and report the handle — the present half of [`Self::render_to_shared`].
    /// Cheap next to a render (one GPU-to-GPU copy), which is what lets the
    /// scheduler pace presents against the clock while renders run ahead.
    #[cfg(all(windows, feature = "shared-texture"))]
    pub fn present_prepared(
        &mut self,
        prepared: &PreparedFrame,
    ) -> Result<SharedFrameInfo, String> {
        let shown = &prepared.texture;
        // The texture's ACTUAL dims — the comp size times the preview scale the
        // composite ran at. The registration sizes off them, so a coarser tier
        // shares a genuinely smaller texture; Dart stretches it into the same
        // Viewer rect, which is what makes the tier cheaper at all.
        let (aw, ah) = prepared.size();
        // Re-create the shared texture when it is missing or the size changed
        // (a comp resize or a tier change) — a new handle is reported then,
        // which the bridge relays so Dart re-registers.
        // Reuse the target for this size when we already hold one — the same
        // handle comes back, so Dart does not re-register and nothing has to be
        // bound afresh. Only a size never seen (or long unused) mints one.
        let found = self
            .shared
            .iter()
            .position(|sh| sh.width == aw && sh.height == ah);
        match found {
            Some(i) => {
                // Most recently used last, so the eviction below takes the
                // size that has gone longest without a frame.
                let sh = self.shared.remove(i);
                self.shared.push(sh);
            }
            None => {
                let made = lumit_gpu::shared::SharedTexture::new(&self.gpu, aw, ah)?;
                self.shared.push(made);
                while self.shared.len() > SHARED_TARGET_POOL {
                    self.shared.remove(0);
                }
            }
        }
        let target = self
            .shared
            .last()
            .ok_or_else(|| "headless render: shared texture missing after create".to_string())?;
        target.present(&self.gpu, shown);
        Ok(SharedFrameInfo {
            handle: target.handle(),
            width: aw,
            height: ah,
            format: "rgba8888",
        })
    }

    /// Render composition `comp_id` at integer `frame` into the Linux DMA-BUF GPU
    /// texture, returning its exported fd and DRM metadata ([`SharedFrameInfoLinux`],
    /// K-177) — the Linux sibling of [`Self::render_to_shared`]. The frame never
    /// leaves the graphics card: it is composited and display-encoded by the same
    /// interactive path (so it shares the drag fast path), then copied into the
    /// DMA-BUF texture instead of being read back.
    ///
    /// The texture is created on the first call and re-used across frames (a
    /// stable fd); a comp of different dimensions re-creates it and reports the new
    /// fd. `Err` on an unknown comp, when wgpu is not on the Vulkan backend, when
    /// the external-memory extensions were not enabled, or any Vulkan failure — the
    /// bridge turns that into "no shared frame" and Dart falls back to read-back.
    #[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
    pub fn render_to_shared_dmabuf(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        cacheable: bool,
    ) -> Result<SharedFrameInfoLinux, String> {
        let prepared = self.render_prepared(doc, comp_id, frame, quality, false, cacheable)?;
        self.present_prepared_dmabuf(&prepared)
    }

    /// Show an already-rendered frame via the DMA-BUF texture — the Linux
    /// sibling of [`Self::present_prepared`], for the scheduler's paced
    /// presents.
    #[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
    pub fn present_prepared_dmabuf(
        &mut self,
        prepared: &PreparedFrame,
    ) -> Result<SharedFrameInfoLinux, String> {
        let shown = &prepared.texture;
        // The texture's ACTUAL dims (comp size × preview scale) — see the
        // Windows sibling above.
        let (aw, ah) = prepared.size();
        // Re-create the DMA-BUF texture when it is missing or the size changed
        // (a comp resize or a tier change) — a new fd is reported then, which
        // the bridge relays so Dart re-registers.
        // Per size and reused — see `shared` for why re-creating churns
        // handles the frontend cannot keep up with.
        let found = self
            .shared_dmabuf
            .iter()
            .position(|sh| sh.width == aw && sh.height == ah);
        match found {
            Some(i) => {
                let sh = self.shared_dmabuf.remove(i);
                self.shared_dmabuf.push(sh);
            }
            None => {
                let made = lumit_gpu::shared_linux::SharedDmabuf::new(&self.gpu, aw, ah)?;
                self.shared_dmabuf.push(made);
                while self.shared_dmabuf.len() > SHARED_TARGET_POOL {
                    self.shared_dmabuf.remove(0);
                }
            }
        }
        let target = self
            .shared_dmabuf
            .last()
            .ok_or_else(|| "headless render: dmabuf texture missing after create".to_string())?;
        target.present(&self.gpu, shown);
        let info = target.info();
        Ok(SharedFrameInfoLinux {
            fd: info.fd,
            width: info.width,
            height: info.height,
            stride: info.stride,
            offset: info.offset,
            drm_fourcc: info.drm_fourcc,
            modifier: info.modifier,
        })
    }

    /// Render composition `comp_id` at integer `frame` into the macOS IOSurface
    /// texture, returning the surface's id and dimensions ([`SharedFrameInfo`],
    /// K-195) — the Metal sibling of the Windows [`Self::render_to_shared`]. The
    /// frame never leaves the graphics card: it is composited and display-encoded
    /// by the same interactive path (so it shares the drag fast path), then
    /// copied into the IOSurface-backed texture instead of being read back.
    ///
    /// The surface is created on the first call and re-used across frames (a
    /// stable id); a comp of different dimensions re-creates it and reports the
    /// new id. `Err` on an unknown comp, when wgpu is not on the Metal backend,
    /// or any IOSurface/Metal failure — the bridge turns that into "no shared
    /// frame" and the frame is dropped.
    #[cfg(all(target_os = "macos", feature = "shared-texture-macos"))]
    pub fn render_to_shared(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        cacheable: bool,
    ) -> Result<SharedFrameInfo, String> {
        // BGRA, as on Windows: the consumer here is a `CVPixelBuffer` of type
        // `kCVPixelFormatType_32BGRA`, the one format Flutter's macOS texture
        // path accepts.
        let prepared = self.render_prepared(doc, comp_id, frame, quality, true, cacheable)?;
        self.present_prepared(&prepared)
    }

    /// Show an already-rendered frame via the IOSurface texture — the macOS
    /// sibling of the Windows [`Self::present_prepared`], for the scheduler's
    /// paced presents.
    #[cfg(all(target_os = "macos", feature = "shared-texture-macos"))]
    pub fn present_prepared(
        &mut self,
        prepared: &PreparedFrame,
    ) -> Result<SharedFrameInfo, String> {
        // The texture's ACTUAL dims (comp size × preview scale) — see the
        // Windows sibling above.
        let (aw, ah) = prepared.size();
        // Re-create the surface when it is missing or the size changed (a comp
        // resize or a tier change) — a new id is reported then, which the bridge
        // relays so Dart re-registers.
        // Per size and reused — see `shared`.
        let found = self
            .shared_iosurface
            .iter()
            .position(|sh| sh.width == aw && sh.height == ah);
        match found {
            Some(i) => {
                let sh = self.shared_iosurface.remove(i);
                self.shared_iosurface.push(sh);
            }
            None => {
                let made = lumit_gpu::shared_metal::SharedIoSurface::new(&self.gpu, aw, ah)?;
                self.shared_iosurface.push(made);
                while self.shared_iosurface.len() > SHARED_TARGET_POOL {
                    self.shared_iosurface.remove(0);
                }
            }
        }
        let target = self
            .shared_iosurface
            .last()
            .ok_or_else(|| "headless render: iosurface missing after create".to_string())?;
        target.present(&self.gpu, &prepared.texture);
        Ok(SharedFrameInfo {
            handle: target.handle(),
            width: aw,
            height: ah,
            format: "rgba8888",
        })
    }

    /// Acquire (or reuse) the Viewer target for `w × h` and report its handle,
    /// without rendering anything into it.
    ///
    /// Exists for the tests that pin the *handle churn* — how many distinct
    /// handles a run of presents hands out — which is the thing that crashed
    /// the compositor and is invisible to any assertion about pixels.
    #[cfg(all(windows, feature = "shared-texture"))]
    pub fn present_probe_size(&mut self, w: u32, h: u32) -> Result<u64, String> {
        let found = self
            .shared
            .iter()
            .position(|sh| sh.width == w && sh.height == h);
        match found {
            Some(i) => {
                let sh = self.shared.remove(i);
                self.shared.push(sh);
            }
            None => {
                let made = lumit_gpu::shared::SharedTexture::new(&self.gpu, w, h)?;
                self.shared.push(made);
                while self.shared.len() > SHARED_TARGET_POOL {
                    self.shared.remove(0);
                }
            }
        }
        self.shared
            .last()
            .map(lumit_gpu::shared::SharedTexture::handle)
            .ok_or_else(|| "shared target missing after acquire".to_string())
    }

    /// How many differently-sized Viewer targets are being held.
    #[cfg(all(windows, feature = "shared-texture"))]
    #[must_use]
    pub fn shared_target_count(&self) -> usize {
        self.shared.len()
    }

    /// Rebuild the `ItemInfo` map for what comp `comp` can show, probing any of
    /// those items not already in `probe_cache`. Slate items are sized to the
    /// comp's own dimensions, matching export's `item_infos`.
    ///
    /// **Only the comp's own footage is probed** — the items its layers name,
    /// and transitively everything its Precomp layers and comp-sourced clips
    /// reach ([`lumit_core::model::comp_footage_items`]). A probe opens a file
    /// and loads or builds its frame index, so probing the whole Project panel
    /// here made the first frame of *any* comp wait for every file in the
    /// project, and a freshly made empty comp wait for all of them to show
    /// nothing. The cache is keyed by item, never emptied between comps, so an
    /// item probed for one comp is already probed when the next one needs it:
    /// the cost is paid once per file per session, and only for files something
    /// on screen can actually want.
    ///
    /// The frame-key interlock is unaffected: an item this comp shows is probed
    /// here before [`crate::cache::frame_key`] reads the probe cache, and an
    /// item this comp cannot show contributes nothing to its key.
    fn sync_items(&mut self, doc: &Document, comp: &Composition) {
        let slate = (comp.width, comp.height);
        self.items.clear();
        for id in lumit_core::model::comp_footage_items(doc, comp) {
            let Some(ProjectItem::Footage(f)) = doc.item(id) else {
                continue;
            };
            let probe = self
                .probe_cache
                .entry(f.id)
                .or_insert_with(|| probe_item(&footage_path(f)));
            match probe {
                Probe::Ok { fps, frames, .. } => {
                    self.items.insert(
                        f.id,
                        ItemInfo {
                            path: footage_path(f),
                            fps: *fps,
                            frames: *frames,
                            missing: None,
                        },
                    );
                }
                // A slate item carries the comp's size so its geometry matches a
                // real layer's (the same reasoning export's `ItemInfo::missing`
                // documents). A `Failed` file in export is simply absent from the
                // map; here it slates instead, so an unreadable source is visibly
                // flagged in the Viewer rather than silently dropped.
                Probe::Slate => {
                    self.items.insert(
                        f.id,
                        ItemInfo {
                            path: footage_path(f),
                            fps: 1.0,
                            frames: 1,
                            missing: Some(slate),
                        },
                    );
                }
                // Audio-only media has no picture to composite: leave it out of
                // the map entirely, exactly as export's `item_infos` does, so
                // `footage_rgba` answers `Ok(None)` for it and the layer draws
                // nothing rather than the missing-footage slate.
                Probe::NoVideo => {}
            }
        }
    }
}

/// The comp audio-jobs walk WITHOUT the GPU renderer — the seam audio playback
/// prepares through, so building a mix never queues behind a slow comp render.
///
/// # In plain terms
///
/// "Which layers make sound, and where do they land on the timeline?" needs no
/// graphics card to answer — only the document and a quick look at each media
/// file (does it carry an audio stream?). [`HeadlessRenderer`] used to own this
/// walk, which meant asking for audio jobs meant owning a whole GPU renderer;
/// now the walk stands alone and the renderer simply holds one of these. The
/// bridge's audio-playback path holds its own, so preparing sound never waits
/// for a picture.
///
/// It is the headless twin of `AppState::comp_audio_jobs` (docs/09 §6): every
/// audible footage layer with an audio stream, its span mapped to the comp
/// timeline, plus nested Precomp layers' contents scaled by their carrier
/// Volumes. Solo silences non-soloed audio per comp, exactly as the video gate
/// does. The has-audio probe result is cached per item, so each file is probed
/// at most once per session.
#[derive(Default)]
pub struct AudioJobsBuilder {
    /// Whether each footage item carries an audio stream, cached so each file
    /// is probed at most once.
    has_audio: HashMap<Uuid, bool>,
}

impl AudioJobsBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The comp's audio jobs (empty for a silent comp) — the same list export,
    /// beat detection and playback all mix from, so they cannot disagree about
    /// what the comp sounds like.
    pub fn audio_jobs(&mut self, doc: &Document, comp: &Composition) -> Vec<AudioJob> {
        let mut jobs = Vec::new();
        let mut visited = vec![comp.id];
        self.walk(
            doc,
            comp,
            0.0,
            (f64::NEG_INFINITY, f64::INFINITY),
            &[],
            &mut visited,
            &mut jobs,
        );
        jobs
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        doc: &Document,
        comp: &Composition,
        base_s: f64,
        window: (f64, f64),
        carriers: &[(lumit_core::anim::Property, f64)],
        visited: &mut Vec<Uuid>,
        jobs: &mut Vec<AudioJob>,
    ) {
        let any_solo = lumit_core::model::any_solo(comp);
        for layer in &comp.layers {
            if !layer.switches.audible || (any_solo && !layer.switches.solo) {
                continue;
            }
            let in_s = (layer.in_point.0.to_f64() + base_s).max(window.0);
            let out_s = (layer.out_point.0.to_f64() + base_s).min(window.1);
            if out_s <= in_s {
                continue;
            }
            let offset_s = layer.start_offset.0.to_f64() + base_s;
            match &layer.kind {
                LayerKind::Footage { item, .. } => {
                    let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
                        continue;
                    };
                    if !self.item_has_audio(*item, &footage_path(f)) {
                        continue;
                    }
                    jobs.push(AudioJob {
                        item: *item,
                        path: footage_path(f),
                        in_s,
                        out_s,
                        offset_s,
                        volume: layer.volume_db.clone(),
                        carriers: carriers.to_vec(),
                    });
                }
                LayerKind::Precomp { comp: nested_id } => {
                    if visited.contains(nested_id) {
                        continue;
                    }
                    let Some(nested) = doc.comp(*nested_id) else {
                        continue;
                    };
                    let mut inner = carriers.to_vec();
                    inner.push((layer.volume_db.clone(), offset_s));
                    visited.push(*nested_id);
                    self.walk(doc, nested, offset_s, (in_s, out_s), &inner, visited, jobs);
                    visited.pop();
                }
                _ => {}
            }
        }
    }

    /// Whether footage `item` at `path` carries an audio stream, cached so each
    /// file is probed for audio at most once across a session.
    fn item_has_audio(&mut self, item: Uuid, path: &Path) -> bool {
        if let Some(&has) = self.has_audio.get(&item) {
            return has;
        }
        let has = path.is_file()
            && lumit_media::probe::probe(path)
                .map(|p| p.audio.is_some())
                .unwrap_or(false);
        self.has_audio.insert(item, has);
        has
    }
}

/// The scale the compositor should composite at for `quality`: the Viewer's
/// display scale when auto resolution asks for less than full, else 1.0.
/// Export always renders with the default quality, so it composites at 1.0
/// and the K-031 preview == export identity is untouched.
fn composite_scale(quality: Quality) -> f32 {
    if quality.auto_res {
        quality.display_scale.min(1.0)
    } else {
        1.0
    }
}

/// The on-disk path a footage item points at (absolute when known, else the
/// stored relative path) — the same resolution the bridge's decode path uses.
pub(crate) fn footage_path(f: &FootageItem) -> PathBuf {
    if f.media.absolute_path.is_empty() {
        PathBuf::from(&f.media.relative_path)
    } else {
        PathBuf::from(&f.media.absolute_path)
    }
}

/// Probe one footage path into a [`Probe`]. A path that is not a file, an
/// unreadable file, or one whose frame index will not build falls to
/// [`Probe::Slate`] — none of them is an error, they are the states the slate
/// exists for. A readable file with no video stream (audio-only) is
/// [`Probe::NoVideo`] instead: also not an error, but the opposite treatment —
/// no slate, no picture at all, since flagging a valid audio-only source as
/// "missing" would be actively wrong. A clean video caches its exact rate and
/// frame count, warming the on-disk frame index so the decoder open reuses it.
fn probe_item(path: &Path) -> Probe {
    if !path.is_file() {
        return Probe::Slate;
    }
    let Ok(probe) = lumit_media::probe::probe(path) else {
        return Probe::Slate;
    };
    let Some(video) = probe.video.as_ref() else {
        return Probe::NoVideo;
    };
    let Ok(index) = crate::media_index::load_or_build_index(path) else {
        return Probe::Slate;
    };
    Probe::Ok {
        fps: video.fps(),
        frames: index.frame_count(),
        width: video.width,
        height: video.height,
    }
}

/// The renderer's own probe cache, seen through the pipeline's one media
/// question ([`SourceProbes`]), so the decode planner and the frame-key stamper
/// read exactly what `sync_items` already resolved — no second probe, and no
/// chance of the two disagreeing about what a file is.
pub(crate) struct ProbeView<'a>(&'a HashMap<Uuid, Probe>);

impl SourceProbes for ProbeView<'_> {
    fn probe(&self, item: Uuid) -> SourceProbe {
        match self.0.get(&item) {
            None => SourceProbe::Unprobed,
            Some(Probe::NoVideo) => SourceProbe::AudioOnly,
            Some(Probe::Slate) => SourceProbe::Missing,
            Some(Probe::Ok {
                fps,
                frames,
                width,
                height,
            }) => SourceProbe::Video {
                fps: *fps,
                width: *width,
                height: *height,
                frames: *frames,
                // The has-audio question is answered by `AudioJobsBuilder`,
                // which probes for it separately; the picture path never asks.
                audio: false,
            },
        }
    }
}

/// Copy a sub-rectangle out of a finished composite (K-362), so a region of
/// interest returns a region-sized texture even on the frames where the
/// realiser had to composite the whole thing.
///
/// A straight texel copy, not a resample: the crop cannot change a single
/// pixel of what it keeps, which is what makes "with a region" and "without
/// one, then cropped" the same picture rather than nearly the same one.
fn crop_texture(
    ctx: &lumit_gpu::GpuContext,
    src: &wgpu::Texture,
    region: lumit_gpu::Region,
    scale: f32,
    comp: (u32, u32),
) -> wgpu::Texture {
    let (mut tw, mut th) = region.target_size(scale);
    tw = tw.min(src.width());
    th = th.min(src.height());
    // Where the window starts on the raster actually rendered. The ratio is
    // taken from that raster rather than from the render scale, so however
    // `scaled_size` rounded, the copy stays inside the source.
    let sx = (region.x * src.width() as f32 / comp.0.max(1) as f32).round() as u32;
    let sy = (region.y * src.height() as f32 / comp.1.max(1) as f32).round() as u32;
    let sx = sx.min(src.width().saturating_sub(tw));
    let sy = sy.min(src.height().saturating_sub(th));
    let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("comp-frame-cropped"),
        size: wgpu::Extent3d {
            width: tw,
            height: th,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: src.format(),
        usage: src.usage(),
        view_formats: &[],
    });
    let mut enc = ctx.encoder("roi-crop");
    enc.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d { x: sx, y: sy, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &out,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: tw,
            height: th,
            depth_or_array_layers: 1,
        },
    );
    drop(enc);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::{
        Composition, LayerKind, LinearColour, ProjectItem, SolidDef, Switches, TransformGroup,
    };
    use lumit_core::store::DocumentStore;
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

    /// The size a scaled preview comes back at, which is now also the size that
    /// is copied off the graphics card. The rounding matches what the
    /// processor-side resize did, so nothing downstream sees a different answer
    /// than it used to.
    #[test]
    fn a_scaled_preview_reports_the_reduced_size() {
        assert_eq!(scaled_size(1920, 1080, 0.5), (960, 540));
        assert_eq!(scaled_size(1920, 1080, 1.0 / 3.0), (640, 360));
        // Rounded, not truncated.
        assert_eq!(scaled_size(1919, 1081, 0.5), (960, 541));
    }

    /// Full scale must stay bit-identical: it takes the untouched path, with no
    /// resampling pass at all.
    #[test]
    fn full_scale_is_left_alone() {
        assert_eq!(scaled_size(1920, 1080, 1.0), (1920, 1080));
        // And nonsense is treated as full rather than producing a 0-wide frame.
        assert_eq!(scaled_size(1920, 1080, 0.0), (1920, 1080));
        assert_eq!(scaled_size(1920, 1080, f32::NAN), (1920, 1080));
        assert_eq!(scaled_size(1920, 1080, -1.0), (1920, 1080));
    }

    /// A scale small enough to round to nothing still has to produce a frame.
    #[test]
    fn a_tiny_scale_still_has_a_pixel() {
        assert_eq!(scaled_size(100, 100, 0.001), (1, 1));
    }

    /// A transform that centres a `w`×`h` object over a `w`×`h` comp (anchor at
    /// the object's middle, position at the comp's middle) — a copy of the
    /// engine's `centred_transform`, so the solid fills the frame.
    fn centred(w: u32, h: u32) -> TransformGroup {
        TransformGroup {
            anchor_x: Property::fixed(f64::from(w) * 0.5),
            anchor_y: Property::fixed(f64::from(h) * 0.5),
            position_x: Property::fixed(f64::from(w) * 0.5),
            position_y: Property::fixed(f64::from(h) * 0.5),
            ..Default::default()
        }
    }

    /// Build a document with one comp holding a single full-frame solid layer of
    /// `colour`, returning the store and the comp id. Drives the real model, so
    /// the render walks the same path a user-built comp would.
    fn doc_with_solid(colour: LinearColour, w: u32, h: u32) -> (DocumentStore, Uuid) {
        let mut doc = Document::new();
        let solid_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: solid_id,
            name: "Solid".into(),
            colour,
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        let comp_id = Uuid::now_v7();
        let layer = lumit_core::model::Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "Solid".into(),
            kind: LayerKind::Solid { def: solid_id },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: centred(w, h),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        doc.items.push(ProjectItem::Composition(Composition {
            id: comp_id,
            name: "Scene".into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (DocumentStore::new(doc), comp_id)
    }

    /// A stand-in bake: the smallest thing `warm_flare_bake` will accept, so a
    /// test of the *naming rules* costs a channel send rather than half a
    /// second of real optics.
    fn stub_bake() -> lumit_gpu::fx::FlareBake {
        std::sync::Arc::new(|| lumit_gpu::fx::FlareBakeData {
            surfaces: Vec::new(),
            ghosts: Vec::new(),
            spreads: Vec::new(),
            sensor_z_mm: 0.0,
            focal_mm: 1.0,
            native_fstop: 1.0,
            pupil_mm: 1.0,
            start_z_mm: 0.0,
            energy_gain: 1.0,
            reflectance: Vec::new(),
            starburst: Vec::new(),
            sb_res: 1,
            sb_fields: 1,
        }) as lumit_gpu::fx::FlareBake
    }

    /// [`doc_with_solid`] with a Lens flare on the layer, `edit` given the
    /// fresh instance to set whichever rows the test is about.
    fn doc_with_flare(
        edit: impl FnOnce(&mut lumit_core::model::EffectInstance),
    ) -> (DocumentStore, Uuid) {
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 32, 32);
        let mut doc = (*store.snapshot()).clone();
        let mut flare = lumit_core::fx::instantiate("lens_flare").expect("the flare is a builtin");
        edit(&mut flare);
        for item in &mut doc.items {
            if let ProjectItem::Composition(c) = item {
                if c.id == comp_id {
                    c.layers[0].effects.push(flare.clone());
                }
            }
        }
        (DocumentStore::new(doc), comp_id)
    }

    /// A footage item in the Project panel, on no layer anywhere. Returns its
    /// id. The path is deliberately not on disk: `probe_item` answers
    /// [`Probe::Slate`] for a path that is not a file without opening anything,
    /// so a probe here costs a `stat` and never FFmpeg.
    fn push_footage_item(doc: &mut Document, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        doc.items
            .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                id,
                name: name.into(),
                media: lumit_core::model::MediaRef {
                    relative_path: name.into(),
                    absolute_path: name.into(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
            }));
        id
    }

    /// Put a layer of `kind` into comp `comp` (which must exist in `doc`).
    fn push_layer(doc: &mut Document, comp: Uuid, kind: LayerKind) {
        let layer = lumit_core::model::Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "layer".into(),
            kind,
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(comp) {
            c.layers.push(layer);
        }
    }

    /// An empty composition added to `doc`, returning its id.
    fn push_comp(doc: &mut Document, name: &str, w: u32, h: u32) -> Uuid {
        let id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            id,
            name: name.into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        id
    }

    /// A full-frame red solid composites to red in the centre pixel — the GPU
    /// oracle that proves the headless seam drives the real compositor. Skips
    /// when the machine has no adapter (the lavapipe/hardware convention the
    /// lumit-gpu tests use).
    #[test]
    fn solid_comp_renders_its_colour_in_the_centre() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        // Pure-red scene-linear solid, 8×8.
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let (rgba, w, h) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert_eq!((w, h), (8, 8));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Centre pixel: strongly red, weak green/blue, opaque. sRGB-encoded, so
        // the exact byte depends on the transfer function; assert the channel
        // ordering and that red dominates rather than an exact value.
        let idx = (((h / 2) * w + w / 2) * 4) as usize;
        let (red, green, blue, alpha) = (rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]);
        assert!(red > 200, "red channel should dominate, got {red}");
        assert!(green < 60, "green should be low, got {green}");
        assert!(blue < 60, "blue should be low, got {blue}");
        assert_eq!(alpha, 255, "the solid is opaque");
    }

    /// **The transparency grid can see through an empty comp (K-352).** The
    /// comp's backdrop is opaque black by default, so every pixel nothing
    /// covers used to reach the Viewer with alpha 1 and the checkerboard
    /// behind the picture could never show — even with every layer hidden.
    /// With the flag on, the interactive path leaves the backdrop out and
    /// uncovered pixels arrive with zero alpha; off again, the backdrop is
    /// back. Fails without the `transparent_background` branch in
    /// `preview_display_texture_fmt`.
    ///
    /// An export stays opaque the same way it stays neutral (see
    /// `an_export_ignores_the_viewer_view`): `export::run` builds its own
    /// renderer, and nothing ever calls `set_transparent_background` on it.
    #[test]
    fn the_transparent_background_flag_uncovers_the_grid() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let mut doc = Document::new();
        let comp_id = push_comp(&mut doc, "Empty", 8, 8);
        let store = DocumentStore::new(doc);
        let doc = store.snapshot();
        let q = Quality::default();

        let (rgba, w, h) = r
            .render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("opaque render");
        let idx = (((h / 2) * w + w / 2) * 4) as usize;
        assert_eq!(rgba[idx + 3], 255, "born opaque: the backdrop is drawn");

        r.set_transparent_background(true);
        let (rgba, _, _) = r
            .render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("transparent render");
        assert_eq!(rgba[idx + 3], 0, "nothing covers this pixel, so no alpha");

        r.set_transparent_background(false);
        let (rgba, _, _) = r
            .render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("opaque again");
        assert_eq!(rgba[idx + 3], 255, "grid down, backdrop back");
    }

    /// `scale` below 1 downsamples the output buffer; the centre stays the solid
    /// colour, proving the resize path is wired and does not corrupt the frame.
    #[test]
    fn scale_downsamples_the_output() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 1.0, 0.0, 1.0]), 16, 16);
        let doc = store.snapshot();
        let (rgba, w, h) = r.render_rgba(&doc, comp_id, 0, 0.5).expect("render");
        assert_eq!((w, h), (8, 8), "half scale halves each dimension");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let idx = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(rgba[idx + 1] > 200, "green solid stays green after resize");
    }

    /// **The preview scale is real.** Under auto resolution the COMPOSITE
    /// itself runs on the scaled raster — the display texture comes back at
    /// comp × scale, not comp size shrunk afterwards. This is what makes a
    /// coarser realtime tier actually cheaper; before the fix this texture was
    /// always full comp size whatever the preview scale, so this test fails
    /// without it. The read-back still carries the right picture at the right
    /// size, with no second resize pass to disagree with.
    #[test]
    fn auto_resolution_composites_at_the_scaled_size() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 16, 16);
        let doc = store.snapshot();
        let q = crate::plan::Quality {
            auto_res: true,
            display_scale: 0.5,
            ..Default::default()
        };
        let (shown, cw, ch) = r
            .preview_display_texture(&doc, comp_id, 0, q)
            .expect("preview texture");
        assert_eq!((cw, ch), (16, 16), "the reported dims stay logical");
        assert_eq!(
            (shown.width(), shown.height()),
            (8, 8),
            "the composite ran at the preview scale, not at comp size"
        );
        // And the read-back entry point agrees end to end: right size, still red.
        let (rgba, w, h) = r.render_preview(&doc, comp_id, 0, q, 0.5).expect("preview");
        assert_eq!((w, h), (8, 8));
        let idx = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(rgba[idx] > 200, "red solid stays red at the scaled size");
    }

    /// **A view names its frames apart rather than leaving them nameless**
    /// (K-346, superseding that half of K-314). The look is baked into the
    /// display-encoded pixels the tiers hold, so a frame under one is a
    /// different picture and takes a different name — which is what lets the
    /// caches keep working while an exposure is dialled in, where the old rule
    /// switched all three off for as long as a control was engaged. Neutral
    /// keeps the name it always had, so frames banked before this still come
    /// back. Needs no adapter — naming is a hash of the document, not a render.
    #[test]
    fn a_view_names_its_frames_apart_rather_than_leaving_them_nameless() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let q = crate::plan::Quality::default();

        let neutral = r.frame_key(&doc, comp_id, 0, q);
        assert!(neutral.is_some(), "a neutral view names its frames");
        r.presync_items(&doc, comp_id);
        assert_eq!(
            r.frame_key_presynced(&doc, comp_id, 0, q),
            neutral,
            "both naming entry points agree while neutral"
        );

        // Exposure alone, tone map alone, and both — each is its own picture.
        let mut seen = vec![neutral];
        for view in [
            lumit_gpu::DisplayParams::from_stops(1.0, false),
            lumit_gpu::DisplayParams::from_stops(0.0, true),
            lumit_gpu::DisplayParams::from_stops(-2.3, true),
        ] {
            r.set_display_view(view);
            let named = r.frame_key(&doc, comp_id, 0, q);
            assert!(named.is_some(), "{view:?} must still name the frame");
            assert!(
                !seen.contains(&named),
                "{view:?} must not share a name with a look already seen"
            );
            assert_eq!(
                r.frame_key_presynced(&doc, comp_id, 0, q),
                named,
                "{view:?}: both naming entry points must agree under a look"
            );
            // Asked twice under the same look, the name must not move, or a
            // frame would be banked under a name nothing looks up again.
            assert_eq!(
                r.frame_key(&doc, comp_id, 0, q),
                named,
                "{view:?}: naming is deterministic"
            );
            seen.push(named);
        }

        // And back: the name it had is the name it gets again, so the frames
        // banked before the control was touched are hits once more.
        r.set_display_view(lumit_gpu::DisplayParams::NEUTRAL);
        assert_eq!(
            r.frame_key(&doc, comp_id, 0, q),
            neutral,
            "returning to neutral returns the frame's own name"
        );

        // The backdrop is part of the picture too (K-352): a frame composited
        // without it must never be served as one composited with it, so the
        // transparency-grid flag names frames apart exactly as a view does.
        r.set_transparent_background(true);
        let transparent = r.frame_key(&doc, comp_id, 0, q);
        assert!(
            transparent.is_some(),
            "a transparent backdrop still names the frame"
        );
        assert!(
            !seen.contains(&transparent),
            "the two backdrops are two different pictures"
        );
        r.set_transparent_background(false);
        assert_eq!(
            r.frame_key(&doc, comp_id, 0, q),
            neutral,
            "backdrop back, name back — frames banked opaque are hits again"
        );
    }

    /// **The export cannot see the Viewer's view** (K-314), and it is neutral by
    /// *construction* rather than by discipline: `export::run` builds its own
    /// `HeadlessRenderer` and nothing ever calls the setter on it, then renders
    /// each frame through `render_preview` exactly as this does.
    ///
    /// So the property under test is that the view is renderer-owned state. The
    /// obvious regression — making it a global, a static or a thread-local, all
    /// of which would look fine in every preview test — fails here, because the
    /// export's fresh renderer would then inherit a view somebody else set. The
    /// first assertion also proves the view is doing something at all, so this
    /// cannot pass by the display transform being a no-op.
    #[test]
    fn an_export_renders_neutral_whatever_the_viewer_is_set_to() {
        let mut viewer = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.18, 0.18, 0.18, 1.0]), 8, 8);
        let doc = store.snapshot();
        let q = crate::plan::Quality::default();

        let (neutral, _, _) = viewer
            .render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("neutral render");

        viewer.set_display_view(lumit_gpu::DisplayParams::from_stops(2.0, true));
        let (exposed, _, _) = viewer
            .render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("exposed render");
        assert_ne!(
            exposed, neutral,
            "two stops and a tone map must visibly change the preview, or this \
             test would pass on a display transform that does nothing"
        );

        // What the exporter does, in the order it does it (`export::run`): its
        // own renderer, then `render_preview` per frame at full quality.
        let mut exporter = HeadlessRenderer::new().expect("export renderer");
        assert!(
            exporter.display_view().is_neutral(),
            "a fresh renderer starts neutral, which is what makes export neutral"
        );
        let (exported, _, _) = exporter
            .render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("export render");
        assert_eq!(
            exported, neutral,
            "the export's bytes are the neutral bytes, with the Viewer set to \
             two stops and a tone map"
        );
    }

    /// Audio-only media (a readable file with no video stream) must not draw
    /// the missing-footage slate: it is a valid source, not a broken one. Bugs
    /// here previously conflated the two (`Probe::Slate`), which painted the
    /// colour bars over a perfectly good audio-only layer in the Flutter
    /// Viewer. Bypasses real FFmpeg probing by seeding `probe_cache` directly
    /// with the outcome `probe_item` would give each file, so the test needs
    /// no media fixture. A genuinely missing file is asserted to still slate,
    /// so a regression collapsing `NoVideo` back onto `Slate` fails this test.
    #[test]
    fn audio_only_media_is_omitted_not_slated() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, _comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 4, 4);
        let mut doc = (*store.snapshot()).clone();
        // The comp is asked about at 64×64 below, so the slate it makes is that
        // size; a layer per item, since only what the comp can show is probed.
        let sized = push_comp(&mut doc, "sized", 64, 64);

        let audio_id = push_footage_item(&mut doc, "audio.wav");
        push_layer(&mut doc, sized, LayerKind::Footage { item: audio_id });
        r.probe_cache.insert(audio_id, Probe::NoVideo);
        let comp = doc.comp(sized).expect("sized comp").clone();
        r.sync_items(&doc, &comp);
        assert!(
            !r.items.contains_key(&audio_id),
            "audio-only media must contribute no picture, not a missing slate"
        );

        // Contrast: a genuinely missing/unreadable file DOES slate.
        let missing_id = push_footage_item(&mut doc, "gone.mp4");
        push_layer(&mut doc, sized, LayerKind::Footage { item: missing_id });
        r.probe_cache.insert(missing_id, Probe::Slate);
        let comp = doc.comp(sized).expect("sized comp").clone();
        r.sync_items(&doc, &comp);
        assert_eq!(
            r.items.get(&missing_id).map(|i| i.missing),
            Some(Some((64, 64))),
            "a missing/unreadable file still slates at the comp's size"
        );
        // The audio-only item stays omitted across the second sync_items call.
        assert!(!r.items.contains_key(&audio_id));

        // K-435: a file that HAS a picture, placed as an Audio layer, is not
        // probed or indexed for the picture either. Without the `audio_only`
        // skip in `comp_footage_items` the renderer would open and frame-index
        // a video the user placed for its sound alone.
        let video_id = push_footage_item(&mut doc, "music-video.mp4");
        push_layer(&mut doc, sized, LayerKind::Footage { item: video_id });
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(sized) {
            c.layers
                .last_mut()
                .expect("the layer just pushed")
                .audio_only = true;
        }
        r.probe_cache.insert(
            video_id,
            Probe::Ok {
                fps: 25.0,
                frames: 125,
                width: 64,
                height: 64,
            },
        );
        let comp = doc.comp(sized).expect("sized comp").clone();
        r.sync_items(&doc, &comp);
        assert!(
            !r.items.contains_key(&video_id),
            "a video placed for its sound alone contributes no picture"
        );
    }

    /// **A render probes only what its comp can show.** Probing is opening a
    /// file and loading or building its frame index, and it used to run over
    /// every footage item in the project before the first frame of any comp —
    /// so a project with a full Project panel paid for all of them before the
    /// first pixel, and a freshly made empty comp paid for all of them to show
    /// nothing.
    ///
    /// `probe_cache` is the counter: an item is in it exactly once it has been
    /// probed, and the cache is never emptied between comps, so the assertions
    /// below read both "what did this call probe" and "what has been probed at
    /// all". No media fixture is needed — the paths are not on disk, so each
    /// probe is a `stat` that answers [`Probe::Slate`].
    #[test]
    fn a_comp_probes_its_own_footage_and_nothing_else() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let mut doc = Document::new();
        let (shown, nested, spare) = (
            push_footage_item(&mut doc, "shown.mp4"),
            push_footage_item(&mut doc, "nested.mp4"),
            push_footage_item(&mut doc, "spare.mp4"),
        );
        let empty = push_comp(&mut doc, "empty", 32, 32);
        let one = push_comp(&mut doc, "one", 32, 32);
        push_layer(&mut doc, one, LayerKind::Footage { item: shown });
        let inner = push_comp(&mut doc, "inner", 32, 32);
        push_layer(&mut doc, inner, LayerKind::Footage { item: nested });
        let outer = push_comp(&mut doc, "outer", 32, 32);
        push_layer(&mut doc, outer, LayerKind::Precomp { comp: inner });

        let probed = |r: &HeadlessRenderer| {
            let mut ids: Vec<Uuid> = r.probe_cache.keys().copied().collect();
            ids.sort();
            ids
        };
        let sorted = |mut ids: Vec<Uuid>| {
            ids.sort();
            ids
        };
        let doc = Arc::new(doc);

        // A comp with no layers, in a project with three footage items.
        r.presync_items(&doc, empty);
        assert!(
            probed(&r).is_empty(),
            "an empty comp must open no files at all"
        );

        // One of three items, and only that one.
        r.presync_items(&doc, one);
        assert_eq!(probed(&r), vec![shown]);

        // Footage a Precomp layer reaches is footage the comp can show, so it
        // is probed — and the frame-key interlock depends on it, since an
        // unprobed source makes the frame unnameable.
        r.presync_items(&doc, outer);
        assert_eq!(
            probed(&r),
            sorted(vec![shown, nested]),
            "a second comp probes what it adds, and what was probed stays probed"
        );
        assert!(
            r.frame_key(&doc, outer, 0, crate::plan::Quality::default())
                .is_some(),
            "everything the comp can show is probed, so its frames are nameable"
        );

        // The item on no layer anywhere is never opened, however many comps
        // have been rendered.
        assert!(
            !probed(&r).contains(&spare),
            "footage no comp shows must never be probed"
        );
    }

    /// The zero-copy path (K-177) renders a real comp into a shared GPU texture
    /// and reports a non-zero NT handle whose dimensions are stable across two
    /// frames (the texture is re-used, not re-created). Skips when there is no
    /// GPU adapter; also skips calmly if this machine's wgpu is not on the D3D12
    /// backend (the shared path needs D3D12 — the read-back path still works).
    #[cfg(all(windows, feature = "shared-texture"))]
    #[test]
    fn solid_comp_renders_to_a_stable_shared_handle() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 0.0, 1.0, 1.0]), 32, 16);
        let doc = store.snapshot();
        let first =
            match r.render_to_shared(&doc, comp_id, 0, crate::plan::Quality::default(), true) {
                Ok(info) => info,
                Err(e) => {
                    // e.g. wgpu chose Vulkan over D3D12, or no shared-heap support.
                    eprintln!("skipping: shared texture unavailable here: {e}");
                    return;
                }
            };
        assert_ne!(first.handle, 0, "a shared render yields a non-zero handle");
        assert_eq!((first.width, first.height), (32, 16));
        assert_eq!(first.format, "rgba8888");

        // A second frame re-uses the same texture: same dimensions, same handle.
        let second = r
            .render_to_shared(&doc, comp_id, 1, crate::plan::Quality::default(), true)
            .expect("second shared render");
        assert_eq!((second.width, second.height), (32, 16));
        assert_eq!(
            second.handle, first.handle,
            "the handle is stable while the comp size is unchanged"
        );
    }

    /// An unknown comp id on the shared path is a calm error, never a panic.
    #[cfg(all(windows, feature = "shared-texture"))]
    #[test]
    fn unknown_comp_is_an_error_on_the_shared_path() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, _comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 4, 4);
        let doc = store.snapshot();
        assert!(r
            .render_to_shared(
                &doc,
                Uuid::now_v7(),
                0,
                crate::plan::Quality::default(),
                true
            )
            .is_err());
    }

    /// The audio-jobs builder needs no GPU: a comp holding a solid (no sound)
    /// and a footage layer whose file is not on disk yields no jobs, calmly,
    /// and the has-audio probe result is cached so the file is checked once.
    #[test]
    fn audio_jobs_builder_needs_no_gpu_and_caches_the_probe() {
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 8, 8);
        let mut doc = (*store.snapshot()).clone();
        // Add a footage item + an audible layer pointing at a missing file.
        let item_id = Uuid::now_v7();
        doc.items
            .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                id: item_id,
                name: "gone.mp4".into(),
                media: lumit_core::model::MediaRef {
                    relative_path: "gone.mp4".into(),
                    absolute_path: "Z:/definitely/not/here/gone.mp4".into(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
            }));
        if let Some(ProjectItem::Composition(c)) = doc
            .items
            .iter_mut()
            .find(|i| matches!(i, ProjectItem::Composition(_)))
        {
            c.layers.push(lumit_core::model::Layer {
                graph: Default::default(),
                markers: Vec::new(),
                id: Uuid::now_v7(),
                name: "gone.mp4".into(),
                kind: LayerKind::Footage { item: item_id },
                in_point: CompTime(Rational::new(0, 1).unwrap()),
                out_point: CompTime(Rational::new(5, 1).unwrap()),
                start_offset: CompTime(Rational::new(0, 1).unwrap()),
                transform: TransformGroup::default(),
                matte: None,
                parent: None,
                label: 0,
                volume_db: Property::zero(),
                audio_only: false,
                retime: None,
                interpolation: Default::default(),
                parked_flow: None,
                blend: Default::default(),
                masks: Vec::new(),
                paint: Vec::new(),
                effects: Vec::new(),
                switches: Switches::default(),
                extra: serde_json::Map::new(),
            });
        }
        let comp = doc.comp(comp_id).unwrap().clone();
        let mut builder = AudioJobsBuilder::new();
        assert!(builder.audio_jobs(&doc, &comp).is_empty());
        assert_eq!(builder.has_audio.len(), 1, "the probe result is cached");
        assert_eq!(builder.has_audio.get(&item_id), Some(&false));
        // A second build reads the cache (no way to observe the skipped disk
        // probe directly, but the cached map must not grow).
        assert!(builder.audio_jobs(&doc, &comp).is_empty());
        assert_eq!(builder.has_audio.len(), 1);
    }

    /// **The export contract for the deferred flare bake (K-350).** A fresh
    /// renderer bakes lens flares *inside* the frame, exactly as it always did.
    ///
    /// This is how "an export is never a provisional picture" is kept true. An
    /// export builds its own renderer (`export::run`), and nobody calls
    /// `set_deferred_flare_bakes` on it — only the Viewer's worker does — so
    /// the promise is a property of the code's shape rather than a rule anyone
    /// has to remember. The default being the exact one is the load-bearing
    /// half: a path that forgets to choose gets the safe behaviour.
    #[test]
    fn a_fresh_renderer_bakes_flares_inside_the_frame() {
        let r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        assert!(
            !r.flare_bake_pending(),
            "a renderer that has made no frames has nothing baking"
        );
        assert_eq!(
            r.flare_bake_generation(),
            0,
            "and nothing has been queued or landed"
        );
    }

    /// A lens baking somewhere **does not stop the rest of the project being
    /// named** (K-431, superseding the K-350 rule it replaces).
    ///
    /// The regression: `frame_key` used to answer `None` for every comp while
    /// any bake was in flight. A keyframed f-stop asks for a slightly
    /// different iris on every frame, so a bake was in flight for as long as
    /// it played — and nothing anywhere in the project could be named, banked
    /// or filled in the background. What matters is whether a frame *drew*
    /// other optics than it names, which is counted rather than guessed at.
    #[test]
    fn a_baking_flare_does_not_unname_other_frames() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let named = r.frame_key(&doc, comp_id, 0, Quality::default());
        assert!(named.is_some(), "an ordinary frame of a solid names itself");

        // Queue a bake by hand: the effect engine is the authority the name
        // asks, and driving it directly keeps this a test of the *rule*
        // rather than of how long a real bake takes on a software rasteriser.
        r.set_deferred_flare_bakes(true);
        let queued = {
            let Some(parts) = r.parts.as_ref() else {
                return;
            };
            parts.fx.warm_flare_bake(0xfeed_face, &stub_bake())
        };
        if !queued {
            return; // no bake thread on this machine
        }
        assert_eq!(
            r.frame_key(&doc, comp_id, 0, Quality::default()),
            named,
            "a bake in flight elsewhere leaves this frame's name exactly as it was"
        );
        assert_eq!(
            r.flare_substitutions(),
            0,
            "and nothing was stood in for, so the frame is one to keep"
        );
    }

    /// **A keyframed aperture names, and keeps, every frame it draws**
    /// (K-431). Ten frames of an animated f-stop, with a bake in flight the
    /// whole time: each frame takes its own name, and no two frames share one
    /// — the aperture is part of the picture, so it is part of the name.
    #[test]
    fn a_keyframed_aperture_names_every_frame() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_flare(|flare| {
            for param in &mut flare.params {
                if param.id == "fstop" {
                    param.value = lumit_core::model::EffectValue::Float(ramp(2.0, 8.0, 1));
                }
            }
        });
        let doc = store.snapshot();
        r.set_deferred_flare_bakes(true);
        let queued = {
            let Some(parts) = r.parts.as_ref() else {
                return;
            };
            parts.fx.warm_flare_bake(0x0f57_09ba, &stub_bake())
        };
        if !queued {
            return; // no bake thread on this machine
        }
        let names: Vec<Option<u128>> = (0..10)
            .map(|f| r.frame_key(&doc, comp_id, f, Quality::default()))
            .collect();
        assert!(
            names.iter().all(Option::is_some),
            "every frame of an animated aperture names itself: {names:?}"
        );
        let mut distinct: Vec<u128> = names.into_iter().flatten().collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            10,
            "and each f-stop is a different picture under a different name"
        );
    }

    /// **Editing a .lens file on disk renames the frames that read it**
    /// (K-431). The bake keys on the file's CONTENT, so before this the edited
    /// prescription rebaked and drew different optics under the old file's
    /// name — a cached frame no edit or undo could ever clear.
    #[test]
    fn an_edited_lens_file_renames_frames() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("mine.lens");
        std::fs::write(
            &path,
            "name: one
focal_length: 50
",
        )
        .expect("write");
        let text = path.to_string_lossy().into_owned();
        let (store, comp_id) = doc_with_flare(|flare| {
            for param in &mut flare.params {
                if param.id == "lens_file" {
                    param.value = lumit_core::model::EffectValue::File(
                        lumit_core::model::FileParam::single(text.clone()),
                    );
                }
            }
        });
        let doc = store.snapshot();
        let before = r.frame_key(&doc, comp_id, 0, Quality::default());
        assert!(before.is_some(), "a flare reading a real file names itself");

        // A different prescription at the same path — a longer one, so the
        // name moves whatever the filesystem's clock granularity is.
        std::fs::write(
            &path,
            "name: two
focal_length: 85
surfaces:
",
        )
        .expect("rewrite");
        let after = r.frame_key(&doc, comp_id, 0, Quality::default());
        assert!(after.is_some(), "and still names itself afterwards");
        assert_ne!(
            before, after,
            "an edited prescription is a different picture and takes a different name"
        );
    }

    /// A bake that has **finished** must read as finished from an idle
    /// thread — with no frame render in between (the K-350 follow-up fix).
    ///
    /// The regression this pins: `bake_pending` used to read the in-flight
    /// set, which only a frame render's `collect` cleared — so after the
    /// bake thread finished, the worker's republish tick saw "still pending"
    /// forever, never re-made the picture, and the lens on screen stayed one
    /// change behind until the user moved the playhead. The user's words:
    /// "if I change the lens most of the time it doesn't even update until
    /// I switch frame."
    #[test]
    fn a_landed_bake_reads_as_landed_without_a_frame_render() {
        let r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        r.set_deferred_flare_bakes(true);
        let queued = {
            let Some(parts) = r.parts.as_ref() else {
                return;
            };
            parts.fx.warm_flare_bake(0xdead_beef, &stub_bake())
        };
        if !queued {
            return; // no bake thread on this machine
        }
        // The bake itself is trivial; give the thread a moment to run it.
        // Polling with a deadline rather than one sleep, so the test is fast
        // when the machine is and honest when it is loaded.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while r.flare_bake_pending() {
            assert!(
                std::time::Instant::now() < deadline,
                "a finished bake must stop reading as pending without any                  frame render — the republish tick depends on exactly this"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            r.flare_bake_generation() >= 2,
            "queued and landed both move the generation"
        );
    }

    /// An unknown comp id is a calm error, never a panic.
    #[test]
    fn unknown_comp_is_an_error() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, _comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 4, 4);
        let doc = store.snapshot();
        let err = r.render_rgba(&doc, Uuid::now_v7(), 0, 1.0);
        assert!(err.is_err(), "an unknown comp id yields an error");
    }

    /// **The drag contract.** Re-rendering the same frame of a document whose
    /// only difference is a dragged value must not decode again: the pixels are
    /// the same, only what is done with them changed. This is the whole reason
    /// the interactive path exists, so it is asserted on the decode counter
    /// rather than on timing.
    ///
    /// Moving to a different frame *must* decode, or the fast path would be
    /// serving stale pixels — so that is asserted in the same test, which means
    /// a regression that simply never decodes cannot pass it.
    #[test]
    fn a_value_drag_recomposites_without_decoding_again() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 0.0, 1.0, 1.0]), 16, 16);
        let doc = store.snapshot();
        let q = crate::plan::Quality::default();

        r.render_preview(&doc, comp_id, 0, q, 1.0).expect("first");
        let after_first = r.decoded_frames();
        assert_eq!(after_first, 1, "the first frame decodes");

        // Ten drag ticks, each a throwaway document with a provisional value —
        // exactly what a frontend hands in while a slider is held.
        let layer = doc.comp(comp_id).expect("comp").layers[0].id;
        for tick in 1..=10 {
            let comp = doc.comp(comp_id).expect("comp");
            let patched = crate::build::patch_layer_prop(
                comp,
                layer,
                lumit_core::model::TransformProp::Rotation,
                f64::from(tick) * 3.0,
            );
            let mut dragging = (*doc).clone();
            for item in &mut dragging.items {
                if let ProjectItem::Composition(c) = item {
                    if c.id == comp_id {
                        *c = patched.clone();
                    }
                }
            }
            r.render_preview(&std::sync::Arc::new(dragging.clone()), comp_id, 0, q, 1.0)
                .expect("drag tick");
        }
        assert_eq!(
            r.decoded_frames(),
            after_first,
            "ten drag ticks must decode nothing — the pixels never changed"
        );

        // A different frame is genuinely different pixels, so it decodes.
        r.render_preview(&doc, comp_id, 1, q, 1.0).expect("frame 1");
        assert_eq!(
            r.decoded_frames(),
            after_first + 1,
            "moving the playhead must decode"
        );
    }

    /// **A prepared frame reports its own size on every platform.** The dims a
    /// present path sizes its shared surface off are the texture's actual ones
    /// — the comp size times the preview scale — and reading them must not
    /// depend on a transport existing. It did: the texture had no reader
    /// outside the Windows and Linux present paths, so macOS builds failed the
    /// `-D warnings` clippy gate on a dead field (K-033).
    #[test]
    fn a_prepared_frame_reports_the_scaled_size() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 1.0, 0.0, 1.0]), 64, 32);
        let doc = store.snapshot();
        let half = crate::plan::Quality {
            auto_res: true,
            display_scale: 0.5,
            ..crate::plan::Quality::default()
        };

        let full = r
            .render_prepared(
                &doc,
                comp_id,
                0,
                crate::plan::Quality::default(),
                false,
                false,
            )
            .expect("full render");
        assert_eq!(
            full.size(),
            (64, 32),
            "full quality composites at comp size"
        );

        let coarse = r
            .render_prepared(&doc, comp_id, 0, half, false, false)
            .expect("half render");
        assert_eq!(
            coarse.size(),
            (32, 16),
            "a coarser tier shares a genuinely smaller texture"
        );
    }

    /// **The VRAM final-frame cache.** A committed frame rendered twice is
    /// composited once — the second `render_prepared` is served from the card
    /// (the hit counter proves it). A drag's provisional render must neither
    /// read the cache (it would show the pre-drag picture) nor store into it
    /// (it would poison later reads), and an owner-driven clear empties it —
    /// the hook Settings → Clear cache pulls. Note what is NOT here any more: a
    /// committed edit no longer clears anything, because the names are content
    /// hashes and an edit simply asks for different ones.
    #[test]
    fn a_cacheable_frame_is_served_from_vram_and_a_drag_never_is() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.5, 0.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let q = crate::plan::Quality::default();

        r.render_prepared(&doc, comp_id, 0, q, false, true)
            .expect("first render");
        assert_eq!(r.frame_texture_hits(), 0, "a cold frame renders");
        r.render_prepared(&doc, comp_id, 0, q, false, true)
            .expect("second render");
        assert_eq!(r.frame_texture_hits(), 1, "the revisit is served from VRAM");

        // A drag render: nothing is banked, and a repeat of it does not read the
        // cache either. Counted rather than looked up by name, because in a comp
        // this static every frame has the SAME name — a constant span hashes
        // identically (docs/06 §5.2), which is the content key doing its job.
        let held = r.frame_texture_stats().2;
        r.render_prepared(&doc, comp_id, 1, q, false, false)
            .expect("drag render");
        assert_eq!(
            r.frame_texture_stats().2,
            held,
            "provisional pixels are never banked"
        );
        r.render_prepared(&doc, comp_id, 1, q, false, false)
            .expect("drag render again");
        assert_eq!(
            r.frame_texture_hits(),
            1,
            "a drag render never reads the cache"
        );

        r.clear_frame_textures();
        assert_eq!(r.frame_texture_stats().2, 0, "Clear cache drops all");
    }

    /// **The content-keying promise, on the VRAM tier.** An edit that cannot
    /// change a pixel must not cost a render: the frame's name is a hash of what
    /// is in it, so renaming a layer or moving the work area asks for exactly the
    /// name already on the card. This is the behaviour the whole tier stack is
    /// judged on in the hand — before it, every committed edit emptied the cache
    /// and the bar went blank.
    #[test]
    fn a_picture_free_edit_still_hits_the_vram_cache() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.2, 0.4, 0.8, 1.0]), 8, 8);
        let mut doc = (*store.snapshot()).clone();
        let q = crate::plan::Quality::default();

        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("first render");
        let hits = r.frame_texture_hits();

        // Rename the layer and nudge the work area: neither is in the picture.
        if let Some(comp) = doc.comp_mut(comp_id) {
            comp.name = "renamed".into();
            comp.layers[0].name = "also renamed".into();
            comp.work_area = Some((
                lumit_core::time::CompTime(lumit_core::time::Rational::ZERO),
                lumit_core::time::CompTime(lumit_core::time::Rational::new(1, 2).unwrap()),
            ));
        }
        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("render after the picture-free edit");
        assert_eq!(
            r.frame_texture_hits(),
            hits + 1,
            "an edit that cannot change a pixel must be served from the card"
        );

        // And an edit that DOES change the picture misses, with no invalidation
        // step anywhere: the name is simply different.
        if let Some(comp) = doc.comp_mut(comp_id) {
            comp.layers[0].transform.opacity = lumit_core::anim::Property::fixed(40.0);
        }
        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("render after a real edit");
        assert_eq!(
            r.frame_texture_hits(),
            hits + 1,
            "a changed picture has a different name, so it renders"
        );
        assert_eq!(
            r.frame_texture_stats().2,
            2,
            "and the pre-edit frame is still held, so an undo is free"
        );
    }

    /// **The demotion ladder** (docs/06 §5.3, §5.1): a frame squeezed out of
    /// VRAM is read back off the card rather than dropped, and can go straight
    /// back up as a texture without being composited again. The read-back is
    /// started at eviction time and collected later, so it never makes the
    /// preview wait — this test polls until it lands.
    #[test]
    fn an_evicted_frame_comes_back_down_and_can_go_back_up() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        // A composited frame, not one put in by hand: only a frame the renderer
        // actually made is read back when it goes (one promoted UP the ladder is
        // already held below, so demoting it again would be pure traffic — the
        // rule that stops a long scrub reading the same frames off the card over
        // and over).
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 8, 8);
        let mut doc = (*store.snapshot()).clone();
        let q = crate::plan::Quality::default();
        // A budget that holds exactly one 8×8 frame, so the second picture
        // evicts the first.
        r.set_frame_texture_budget(8 * 8 * 4 + 64);
        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("composite the frame the ladder will demote");
        let first = r
            .frame_key(&std::sync::Arc::new(doc.clone()), comp_id, 0, q)
            .expect("a solid-only comp is nameable");
        assert!(r.has_frame_texture(first, false));

        // A different picture of the same frame — a dimmed solid — so the name
        // differs and the first entry is evicted rather than replaced.
        if let Some(comp) = doc.comp_mut(comp_id) {
            comp.layers[0].transform.opacity = lumit_core::anim::Property::fixed(30.0);
        }
        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("composite a second picture, evicting the first");
        assert!(!r.has_frame_texture(first, false), "the first was evicted");

        // The read-back lands within a few polls; it is running on the card.
        let mut demoted = Vec::new();
        for _ in 0..200 {
            demoted = r.poll_demotions();
            if !demoted.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(demoted.len(), 1, "the evicted frame came back down");
        assert_eq!(demoted[0].key, first, "under the name it was held by");
        assert_eq!((demoted[0].width, demoted[0].height), (8, 8));
        assert_eq!(demoted[0].rgba.len(), 8 * 8 * 4, "a whole frame of bytes");

        // And back up: the bytes become a texture again, so a demoted frame is
        // shown without re-compositing anything.
        let back = r
            .upload_frame_texture(demoted[0].promotion())
            .expect("a demoted frame can be promoted again");
        assert_eq!(back.size(), (8, 8));
        assert!(r.has_frame_texture(first, false), "held on the card again");

        // A truncated payload is refused rather than shown as garbage.
        assert!(
            r.upload_frame_texture(Promotion {
                bytes: &demoted[0].rgba[..16],
                ..demoted[0].promotion()
            })
            .is_none(),
            "a short entry is refused, never uploaded"
        );

        // And a frame that came back UP is not sent down again when it goes: it
        // is already held below, so a re-eviction costs no read-back at all.
        // (Other frames may well come down in the meantime — the one thing that
        // must not appear again is this key.)
        if let Some(comp) = doc.comp_mut(comp_id) {
            comp.layers[0].transform.opacity = lumit_core::anim::Property::fixed(70.0);
        }
        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("a third picture takes the promoted frame's place");
        assert!(
            !r.has_frame_texture(first, false),
            "the promoted frame went"
        );
        let mut came_down = Vec::new();
        for _ in 0..40 {
            came_down.extend(r.poll_demotions().into_iter().map(|d| d.key));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !came_down.contains(&first),
            "a promoted frame is not read back a second time"
        );
    }

    /// **A frame reaches the tiers below even when the cache is never full.**
    ///
    /// The regression this pins is a hole the ladder had from the start: the
    /// only way down was eviction, so a cache with a budget bigger than the
    /// session ever fills wrote *nothing* to disk. The bigger the budget the
    /// user gave it, the more certainly the disk tier stayed empty — and the
    /// symptom was silent, a cache bar green all session and blank after a
    /// restart.
    ///
    /// Here the budget is generous, nothing is ever evicted, and the frame must
    /// still come down.
    #[test]
    fn a_held_frame_is_copied_down_without_being_evicted() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 1.0, 0.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let q = crate::plan::Quality::default();
        // Room for a hundred of these frames: this cache never evicts anything.
        r.set_frame_texture_budget((8 * 8 * 4 + 64) * 100);
        r.render_prepared(&doc, comp_id, 0, q, false, true)
            .expect("bank one frame");
        let key = r
            .frame_key(&doc, comp_id, 0, q)
            .expect("a solid-only comp is nameable");
        assert!(r.has_frame_texture(key, false));

        // Nothing is parked yet, thus the backup has exactly one frame to copy.
        let none_parked = |_: u128| false;
        assert!(
            r.start_backup(&none_parked),
            "a held frame that is not on disk is worth copying down"
        );

        let mut down = Vec::new();
        for _ in 0..200 {
            down = r.poll_demotions();
            if !down.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(down.len(), 1, "the copy came down");
        assert_eq!(down[0].key, key, "under the name it is held by");
        assert_eq!(down[0].rgba.len(), 8 * 8 * 4, "a whole frame of bytes");
        assert!(
            r.has_frame_texture(key, false),
            "and the frame is still on the card — a copy, not an eviction"
        );

        // Once it is down it is not copied again, whichever way the caller
        // answers: the frame itself now knows it is held below.
        assert!(
            !r.start_backup(&none_parked),
            "a frame already copied down is not copied twice"
        );

        // And when it is finally pushed out, it goes quietly — the pixels are
        // downstairs already, so reading them a second time would be pure
        // traffic.
        r.set_frame_texture_budget(8 * 8 * 4 + 64);
        let mut doc = (*doc).clone();
        if let Some(comp) = doc.comp_mut(comp_id) {
            comp.layers[0].transform.opacity = lumit_core::anim::Property::fixed(25.0);
        }
        r.render_prepared(
            &std::sync::Arc::new(doc.clone()),
            comp_id,
            0,
            q,
            false,
            true,
        )
        .expect("a second picture takes its place");
        assert!(!r.has_frame_texture(key, false), "the first was evicted");
        let mut came_down = Vec::new();
        for _ in 0..40 {
            came_down.extend(r.poll_demotions().into_iter().map(|d| d.key));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !came_down.contains(&key),
            "a frame already backed up is not read back when it goes"
        );
    }

    /// A promotion uses a texture that the cache has finished with, in place of
    /// a new one — but only when nothing shows that texture any more.
    ///
    /// Playback goes past a promoted frame each time it comes round, and a new
    /// texture for each of them is an allocation on the card for each frame.
    /// The share count is the test of whether a texture is free, and it has to
    /// be: a write into a texture that a present still shows would put the wrong
    /// picture on the screen.
    #[test]
    fn a_free_texture_holds_the_next_promoted_frame() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let comp = Uuid::now_v7();
        let bytes = vec![0u8; 8 * 8 * 4];
        let promote = |key: u128| Promotion {
            key,
            bgra: false,
            width: 8,
            height: 8,
            bytes: &bytes,
            cost_ms: 4,
            provenance: FrameProvenance {
                comp,
                frame: key as u64,
                scale_q: 1000,
                quality: Quality::default(),
            },
        };
        // Room for exactly one 8×8 frame, so each promotion evicts the one
        // before it.
        r.set_frame_texture_budget(8 * 8 * 4 + 64);

        let first = r.upload_frame_texture(promote(1)).expect("first promotion");
        let first_texture = std::sync::Arc::as_ptr(&first.texture);

        // The first frame is evicted here, but `first` is still held — as a
        // present holds a frame it is showing. Its texture must not be written
        // over.
        let second = r
            .upload_frame_texture(promote(2))
            .expect("second promotion");
        assert!(
            !std::ptr::eq(std::sync::Arc::as_ptr(&second.texture), first_texture),
            "a texture that something still shows is never written over"
        );

        // Nothing shows the first frame now, thus the next promotion takes its
        // texture in place of making one.
        drop(first);
        let third = r.upload_frame_texture(promote(3)).expect("third promotion");
        assert!(
            std::ptr::eq(std::sync::Arc::as_ptr(&third.texture), first_texture),
            "a free texture is used again"
        );
        assert_eq!(third.size(), (8, 8));
        // And Clear cache gives the memory on the card back.
        drop(second);
        drop(third);
        r.clear_frame_textures();
        assert!(r.upload_pool.is_empty(), "a clear empties the pool as well");
    }

    /// The interactive path renders the same picture the export path does — the
    /// K-031 promise, checked on the one comp both can build without media. A
    /// solid is enough to catch a wrong background, colour pipeline or camera:
    /// those are the parts the two walks each implement separately.
    #[test]
    fn the_preview_and_export_paths_agree_on_a_solid_comp() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.2, 0.7, 0.4, 1.0]), 32, 16);
        let doc = store.snapshot();

        let (preview, pw, ph) = r
            .render_preview(&doc, comp_id, 0, crate::plan::Quality::default(), 1.0)
            .expect("preview render");
        let (export, ew, eh) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("export render");

        assert_eq!((pw, ph), (ew, eh), "both paths render at the comp's size");
        assert_eq!(
            preview, export,
            "the interactive and export paths must produce identical pixels (K-031)"
        );
    }

    /// **A parameter the music drives is the same number in both renders**
    /// (K-471 §1.3, K-031) — and it is a number, not silence.
    ///
    /// The row the matrix was missing: a Brightness on a solid, driven through
    /// a Remap by the level of a track on another layer. Two things are checked,
    /// and the second is the one that would have caught the gap this closes:
    /// the interactive and export paths agree pixel for pixel, **and** the
    /// driven picture differs from the same comp with the wire cut. Before the
    /// tap was wired the driver read nought in both renders, so the first
    /// assertion passed on a picture that had ignored the music entirely.
    #[test]
    fn the_preview_and_export_paths_agree_on_an_audio_driven_comp() {
        use lumit_core::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};

        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let dir = std::env::temp_dir().join("lumit-audio-driver-fixture");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some(tone) = lumit_media::index::tests_support::tone(&dir) else {
            eprintln!("no ffmpeg CLI: the audio-driven row is skipped");
            return;
        };

        let (cw, ch) = (32u32, 16u32);
        let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.2, 0.2, 0.2, 1.0]));

        // The music: sound and no picture.
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            id: item,
            name: "tone.flac".into(),
            media: lumit_core::model::MediaRef {
                relative_path: "tone.flac".into(),
                absolute_path: tone.to_string_lossy().into_owned(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
        }));
        let mut music = matrix_layer("Music", LayerKind::Footage { item }, cw, ch);
        music.audio_only = true;
        let music_id = music.id;

        // Brightness on the solid, wired to the level of that music. The Remap
        // is what makes the level readable as a percentage: the whole track's
        // RMS is a fraction of one, and Brightness is a percentage.
        let brightness = lumit_core::fx::instantiate("brightness").expect("the catalogue knows it");
        let brightness_id = brightness.id;
        let mut level = lumit_core::fx::instantiate("audio_level").expect("the catalogue knows it");
        for p in &mut level.params {
            if p.id == "audio" {
                p.value = lumit_core::model::EffectValue::Layer(Some(music_id));
            }
        }
        let level_id = level.id;
        let remap = lumit_core::fx::instantiate("remap").expect("the catalogue knows it");
        let remap_id = remap.id;

        let to_brightness = Edge {
            from: OutputRef::Driver {
                node: remap_id,
                port: "value".into(),
            },
            to: InputRef::Param {
                node: NodeRef::Effect(brightness_id),
                port: "brightness".into(),
            },
        };
        let graph = LayerGraph {
            nodes: vec![level, remap],
            edges: vec![
                Edge {
                    from: OutputRef::Driver {
                        node: level_id,
                        port: "amplitude".into(),
                    },
                    to: InputRef::Param {
                        node: NodeRef::Driver(remap_id),
                        port: "value".into(),
                    },
                },
                to_brightness,
            ],
            ..LayerGraph::default()
        };
        {
            let comp = doc.comp_mut(comp_id).expect("comp");
            comp.layers.push(music);
            comp.layers[0].effects = vec![brightness];
            comp.layers[0].graph = graph;
        }
        // The same comp with the last wire cut: the parameter falls back to its
        // own stored value, which is what "reads silence" looked like.
        let mut unwired = doc.clone();
        unwired.comp_mut(comp_id).expect("comp").layers[0]
            .graph
            .edges
            .pop();

        let doc = std::sync::Arc::new(doc);
        let (preview, pw, ph) = r
            .render_preview(&doc, comp_id, 0, crate::plan::Quality::default(), 1.0)
            .expect("preview render");
        let (export, ew, eh) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("export render");
        assert_eq!((pw, ph), (ew, eh), "both paths render at the comp's size");
        assert_eq!(
            preview, export,
            "a driven parameter must reach the same value in both renders (K-031)"
        );

        let (silent, _, _) = r
            .render_rgba(&std::sync::Arc::new(unwired), comp_id, 0, 1.0)
            .expect("unwired render");
        assert_ne!(
            export, silent,
            "the driver must actually read the sound — equal pixels mean it read silence"
        );
    }

    /// A Null layer draws nothing: the same comp with a Null sitting on top of
    /// it renders byte for byte identically to the comp without one. A Null is
    /// a transform to parent to, never a picture — and because it is invisible
    /// there is no other way for a user to notice it started drawing.
    #[test]
    fn a_null_layer_draws_nothing() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (cw, ch) = (32u32, 16u32);
        let colour = LinearColour([0.8, 0.1, 0.1, 1.0]);

        let (plain, plain_comp, _) = matrix_base(cw, ch, colour);
        let (without, w, h) = r
            .render_rgba(&std::sync::Arc::new(plain.clone()), plain_comp, 0, 1.0)
            .expect("render");

        let (mut with_null, null_comp, _) = matrix_base(cw, ch, colour);
        let mut null = matrix_layer("Null", LayerKind::Null, cw, ch);
        // Off-centre, so a Null that did draw could not hide behind the base.
        null.transform.position_x = Property::fixed(4.0);
        null.transform.position_y = Property::fixed(4.0);
        with_null
            .comp_mut(null_comp)
            .expect("comp")
            .layers
            .insert(0, null);
        let (with, w2, h2) = r
            .render_rgba(&std::sync::Arc::new(with_null.clone()), null_comp, 0, 1.0)
            .expect("render");

        assert_eq!((w, h), (w2, h2));
        assert_eq!(without, with, "a Null layer must contribute no pixels");
    }

    /// **Layers under a full-frame opaque solid are not rendered** (K-423), and
    /// the picture cannot tell. The same comp is rendered with the cull live
    /// and with it refused — a Null on top whose matte names the bottom layer
    /// is a reference to a layer below, which switches the cull off without
    /// adding a pixel — and the two must be byte-identical, for a solid
    /// underneath and (where ffmpeg can write the fixture) for footage. The
    /// draw list proves the cull engaged; the export path stays identical to
    /// the interactive one (K-031).
    #[test]
    fn layers_under_a_full_frame_opaque_solid_are_culled_without_changing_a_pixel() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (cw, ch) = (32u32, 16u32);
        // A reference from above to the bottom layer refuses the cull, and a
        // Null draws nothing, so the picture is the same either way.
        let refuse_cull = |doc: &mut Document, comp_id: Uuid| {
            let comp = doc.comp_mut(comp_id).expect("comp");
            let bottom = comp.layers.last().expect("a layer").id;
            let mut null = matrix_layer("Ref", LayerKind::Null, cw, ch);
            null.matte = Some(lumit_core::model::MatteRef {
                layer: bottom,
                channel: lumit_core::model::MatteChannel::Alpha,
                inverted: false,
                source: Default::default(),
            });
            comp.layers.insert(0, null);
        };
        let cover = |doc: &mut Document, comp_id: Uuid| {
            let solid = Uuid::now_v7();
            doc.items.push(ProjectItem::Solid(SolidDef {
                id: solid,
                name: "Cover".into(),
                colour: LinearColour([0.1, 0.6, 0.2, 1.0]),
                width: cw,
                height: ch,
                extra: serde_json::Map::new(),
            }));
            let layer = matrix_layer("Cover", LayerKind::Solid { def: solid }, cw, ch);
            doc.comp_mut(comp_id).expect("comp").layers.insert(0, layer);
        };

        let mut scenes: Vec<(&str, Document, Uuid)> = Vec::new();
        let (solid_doc, solid_comp, _) = matrix_base(cw, ch, LinearColour([0.8, 0.1, 0.1, 1.0]));
        scenes.push(("a solid underneath", solid_doc, solid_comp));
        let dir = std::env::temp_dir().join("lumit-occlusion-fixture");
        std::fs::create_dir_all(&dir).expect("temp dir");
        match lumit_media::index::tests_support::fixture(&dir) {
            Some(clip) => {
                let mut doc = Document::new();
                let item = Uuid::now_v7();
                doc.items.push(ProjectItem::Footage(FootageItem {
                    id: item,
                    name: "fixture.mp4".into(),
                    media: lumit_core::model::MediaRef {
                        relative_path: "fixture.mp4".into(),
                        absolute_path: clip.to_string_lossy().into_owned(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    extra: serde_json::Map::new(),
                }));
                let comp_id = push_comp(&mut doc, "Scene", cw, ch);
                let clip_layer = matrix_layer("Clip", LayerKind::Footage { item }, 320, 240);
                doc.comp_mut(comp_id).expect("comp").layers.push(clip_layer);
                scenes.push(("footage underneath", doc, comp_id));
            }
            None => eprintln!("no ffmpeg CLI: the footage row is skipped"),
        }

        for (name, mut doc, comp_id) in scenes {
            cover(&mut doc, comp_id);
            let mut refused = doc.clone();
            refuse_cull(&mut refused, comp_id);
            let (culled, refused) = (Arc::new(doc), Arc::new(refused));

            let comp = culled.comp(comp_id).expect("comp");
            assert_eq!(
                lumit_core::occlusion::occluder_index(&culled, comp, 0.0),
                Some(0),
                "{name}: the cover must be recognised as the occluder"
            );
            let refused_comp = refused.comp(comp_id).expect("comp");
            assert_eq!(
                lumit_core::occlusion::occluder_index(&refused, refused_comp, 0.0),
                None,
                "{name}: a reference to the bottom layer must refuse the cull"
            );
            let draws = |doc: &Arc<Document>, comp: &Composition| {
                let pixels = HashMap::new();
                crate::build::build_comp_draws(doc, comp, 0.0, &pixels, &mut vec![comp.id]).len()
            };
            assert_eq!(draws(&culled, comp), 1, "{name}: only the cover is built");
            assert!(
                draws(&refused, refused_comp) >= 1,
                "{name}: the refused comp builds at all"
            );

            let (with_cull, w, h) = r
                .render_rgba(&culled, comp_id, 0, 1.0)
                .unwrap_or_else(|e| panic!("{name}: culled render failed: {e}"));
            let (without_cull, w2, h2) = r
                .render_rgba(&refused, comp_id, 0, 1.0)
                .unwrap_or_else(|e| panic!("{name}: unculled render failed: {e}"));
            assert_eq!((w, h), (w2, h2));
            assert_eq!(
                with_cull, without_cull,
                "{name}: the cull must not change a pixel"
            );
            let (preview, _, _) = r
                .render_preview(&culled, comp_id, 0, crate::plan::Quality::default(), 1.0)
                .expect("preview render");
            assert_eq!(
                preview, with_cull,
                "{name}: preview and export agree (K-031)"
            );
        }
    }

    /// One layer for the matrix scenarios: full-frame span, centred over its
    /// own natural size, everything else the model's defaults.
    fn matrix_layer(name: &str, kind: LayerKind, w: u32, h: u32) -> lumit_core::model::Layer {
        lumit_core::model::Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: name.into(),
            kind,
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: centred(w, h),
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// A two-key linear ramp, for rows that need genuine animation (motion
    /// blur's sub-frame samples, the temporal re-render's held time).
    fn ramp(from: f64, to: f64, over_s: i64) -> Property {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: Rational::new(0, 1).unwrap(),
                    value: from,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
                Keyframe {
                    time: Rational::new(over_s, 1).unwrap(),
                    value: to,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        }
    }

    /// The K-031 matrix. It gated the comp-walk unification (preview vs the
    /// old `render_comp_linear`, byte for byte, before the old walk could be
    /// deleted); with one walk left it now proves that walk renders every
    /// construction deterministically — a retained-pixel recomposite and a
    /// fresh render must still agree exactly. Each row is a document the model
    /// builds without a media file; the footage rows are the test below.
    #[test]
    fn the_preview_and_export_paths_agree_across_the_matrix() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };

        let (cw, ch) = (32u32, 16u32);
        let red = LinearColour([0.8, 0.1, 0.1, 1.0]);
        let blue = LinearColour([0.1, 0.2, 0.9, 1.0]);

        // Each scenario builds its own document from scratch, so a row can
        // never lean on another's state.
        type Build = fn(u32, u32, LinearColour, LinearColour) -> (Document, Uuid, u64);
        let scenarios: Vec<(&str, Build)> = vec![
            ("stacked blends and opacity", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (_, top) = matrix_top(&mut doc, comp_id, blue);
                let comp = doc.comp_mut(comp_id).unwrap();
                let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
                l.blend = lumit_core::model::BlendMode::Multiply;
                l.transform.opacity = Property::fixed(60.0);
                l.transform.rotation = Property::fixed(25.0);
                (doc, comp_id, 0)
            }),
            ("nested precomp", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (child_doc, child_id, _) = matrix_base(16, 16, blue);
                for item in child_doc.items {
                    doc.items.push(item);
                }
                let layer = matrix_layer("Nested", LayerKind::Precomp { comp: child_id }, 16, 16);
                doc.comp_mut(comp_id).unwrap().layers.insert(0, layer);
                (doc, comp_id, 0)
            }),
            ("collapsed precomp", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (child_doc, child_id, _) = matrix_base(16, 16, blue);
                for item in child_doc.items {
                    doc.items.push(item);
                }
                let mut layer =
                    matrix_layer("Collapsed", LayerKind::Precomp { comp: child_id }, 16, 16);
                layer.switches.collapse = true;
                doc.comp_mut(comp_id).unwrap().layers.insert(0, layer);
                (doc, comp_id, 0)
            }),
            ("matte source none", |w, h, red, blue| {
                matte_doc(w, h, red, blue, lumit_core::model::LayerInputSource::None)
            }),
            ("matte source masks", |w, h, red, blue| {
                matte_doc(w, h, red, blue, lumit_core::model::LayerInputSource::Masks)
            }),
            ("matte source effects and masks", |w, h, red, blue| {
                matte_doc(
                    w,
                    h,
                    red,
                    blue,
                    lumit_core::model::LayerInputSource::EffectsAndMasks,
                )
            }),
            ("adjustment layer with an effect", |w, h, red, _blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let mut adj = matrix_layer("Adjust", LayerKind::Adjustment, w, h);
                adj.effects
                    .push(lumit_core::fx::instantiate("invert").unwrap());
                doc.comp_mut(comp_id).unwrap().layers.insert(0, adj);
                (doc, comp_id, 0)
            }),
            ("per-layer motion blur", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (_, top) = matrix_top(&mut doc, comp_id, blue);
                let comp = doc.comp_mut(comp_id).unwrap();
                comp.motion_blur = lumit_core::model::MotionBlur {
                    enabled: true,
                    shutter_angle: 180.0,
                    shutter_phase: -90.0,
                    samples: 8,
                };
                let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
                l.switches.motion_blur = true;
                l.transform.rotation = ramp(0.0, 180.0, 1);
                (doc, comp_id, 15)
            }),
            ("posterize time holds the stack below", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (_, top) = matrix_top(&mut doc, comp_id, blue);
                let comp = doc.comp_mut(comp_id).unwrap();
                let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
                l.transform.rotation = ramp(0.0, 180.0, 1);
                let mut adj = matrix_layer("Hold", LayerKind::Adjustment, w, h);
                adj.effects
                    .push(lumit_core::fx::instantiate("posterize_time").unwrap());
                comp.layers.insert(0, adj);
                (doc, comp_id, 15)
            }),
            // The anti-aliasing row (K-274, docs/impl/anti-aliasing.md §5,
            // test 3): the count is a PROJECT property, so both walks read the
            // same one — an export that anti-aliased differently from the
            // preview is exactly what this matrix exists to catch. A rotated
            // layer, because a rotated edge is what the setting changes.
            ("anti-aliasing on a rotated layer", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                doc.anti_aliasing = lumit_core::model::AntiAliasing::X4;
                let (_, top) = matrix_top(&mut doc, comp_id, blue);
                let comp = doc.comp_mut(comp_id).unwrap();
                let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
                l.transform.rotation = Property::fixed(17.0);
                (doc, comp_id, 0)
            }),
            // A **driven parameter** (K-471): the top layer's blur radius comes
            // off a Wiggle rather than off its keyframes, so both walks have to
            // resolve the same driver graph at the same layer time and get the
            // same number. A driver that read a clock, or that depended on
            // which render ran first, would show up here as two different
            // pictures — which is the whole of §2.2's determinism promise.
            ("a wiggle-driven blur radius", |w, h, red, blue| {
                use lumit_core::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (_, top) = matrix_top(&mut doc, comp_id, blue);

                let blur = lumit_core::fx::instantiate("blur").unwrap();
                let blur_id = blur.id;
                let mut wiggle = lumit_core::fx::instantiate("wiggle").unwrap();
                for p in &mut wiggle.params {
                    if p.id == "amount" {
                        p.value = lumit_core::model::EffectValue::Float(Property::fixed(6.0));
                    }
                    if p.id == "frequency" {
                        p.value = lumit_core::model::EffectValue::Float(Property::fixed(3.0));
                    }
                }
                let wiggle_id = wiggle.id;

                let comp = doc.comp_mut(comp_id).unwrap();
                let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
                l.effects = vec![blur];
                l.graph = LayerGraph {
                    nodes: vec![wiggle],
                    edges: vec![Edge {
                        from: OutputRef::Driver {
                            node: wiggle_id,
                            port: "value".into(),
                        },
                        to: InputRef::Param {
                            node: NodeRef::Effect(blur_id),
                            port: "radius".into(),
                        },
                    }],
                    layout: Vec::new(),
                    exposed: Vec::new(),
                };
                // A frame partway in, so the wobble is somewhere other than its
                // starting value.
                (doc, comp_id, 7)
            }),
            ("camera over a 3d layer", |w, h, red, blue| {
                let (mut doc, comp_id, _) = matrix_base(w, h, red);
                let (_, top) = matrix_top(&mut doc, comp_id, blue);
                let comp = doc.comp_mut(comp_id).unwrap();
                let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
                l.switches.three_d = true;
                l.transform.rotation_y = Property::fixed(35.0);
                l.transform.position_z = Property::fixed(40.0);
                let camera = matrix_layer(
                    "Camera",
                    LayerKind::Camera {
                        zoom: Property::fixed(f64::from(h) * 2.0),
                        solve_link: None,
                    },
                    w,
                    h,
                );
                comp.layers.insert(0, camera);
                (doc, comp_id, 0)
            }),
        ];

        for (name, build) in scenarios {
            let (doc, comp_id, frame) = build(cw, ch, red, blue);
            let store = DocumentStore::new(doc);
            let doc = store.snapshot();
            let (preview, pw, ph) = r
                .render_preview(&doc, comp_id, frame, crate::plan::Quality::default(), 1.0)
                .unwrap_or_else(|e| panic!("{name}: preview render failed: {e}"));
            let (export, ew, eh) = r
                .render_rgba(&doc, comp_id, frame, 1.0)
                .unwrap_or_else(|e| panic!("{name}: export render failed: {e}"));
            assert_eq!(
                (pw, ph),
                (ew, eh),
                "{name}: the two paths render at different sizes"
            );
            assert_eq!(
                preview, export,
                "{name}: the interactive and export paths must be bit-identical (K-031)"
            );
        }
    }

    /// **A wire reaches the picture** (K-471 §2.1).
    ///
    /// The matrix above proves the two walks agree; this proves there is
    /// something to agree *about*. The same comp is rendered three times: with
    /// no graph, with a Remap driving the blur radius to twenty, and with the
    /// driver bypassed. The driven frame differs from the other two, and the
    /// bypassed frame is the undriven one exactly — which is what says the
    /// substitution happens where it claims to and stops when the `B` badge
    /// says so.
    #[test]
    fn a_driven_radius_changes_the_picture_and_a_bypassed_driver_does_not() {
        use lumit_core::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };

        let (cw, ch) = (32u32, 16u32);
        let red = LinearColour([0.8, 0.1, 0.1, 1.0]);
        let blue = LinearColour([0.1, 0.2, 0.9, 1.0]);

        let build = |graph: Option<bool>| -> (Document, Uuid) {
            let (mut doc, comp_id, _) = matrix_base(cw, ch, red);
            let (_, top) = matrix_top(&mut doc, comp_id, blue);
            let mut blur = lumit_core::fx::instantiate("blur").unwrap();
            for p in &mut blur.params {
                if p.id == "radius" {
                    p.value = lumit_core::model::EffectValue::Float(Property::fixed(0.0));
                }
            }
            let blur_id = blur.id;

            let mut remap = lumit_core::fx::instantiate("remap").unwrap();
            for p in &mut remap.params {
                let v = match p.id.as_str() {
                    "value" | "in_high" => 1.0,
                    "in_low" | "out_low" => 0.0,
                    "out_high" => 20.0,
                    _ => continue,
                };
                p.value = lumit_core::model::EffectValue::Float(Property::fixed(v));
            }
            let remap_id = remap.id;

            let comp = doc.comp_mut(comp_id).unwrap();
            let l = comp.layers.iter_mut().find(|l| l.id == top).unwrap();
            l.transform.rotation = Property::fixed(20.0);
            l.effects = vec![blur];
            if let Some(enabled) = graph {
                remap.enabled = enabled;
                l.graph = LayerGraph {
                    nodes: vec![remap],
                    edges: vec![Edge {
                        from: OutputRef::Driver {
                            node: remap_id,
                            port: "value".into(),
                        },
                        to: InputRef::Param {
                            node: NodeRef::Effect(blur_id),
                            port: "radius".into(),
                        },
                    }],
                    layout: Vec::new(),
                    exposed: Vec::new(),
                };
            }
            (doc, comp_id)
        };

        let render = |r: &mut HeadlessRenderer, graph: Option<bool>| -> Vec<u8> {
            let (doc, comp_id) = build(graph);
            let doc = DocumentStore::new(doc).snapshot();
            r.render_rgba(&doc, comp_id, 0, 1.0)
                .expect("the comp renders")
                .0
        };

        let plain = render(&mut r, None);
        let driven = render(&mut r, Some(true));
        let bypassed = render(&mut r, Some(false));

        assert_ne!(
            plain, driven,
            "a wire driving the radius to twenty must change the picture"
        );
        assert_eq!(
            plain, bypassed,
            "a bypassed driver hands the parameter back to its keyframes"
        );
    }

    /// A retimed footage layer beneath an accumulation motion blur adjustment
    /// with Force on all layers renders the footage, whichever interpolation
    /// the retime uses. Pinned because the manual's harness once reported a
    /// solid black frame for exactly this stack; the footage is held across the
    /// samples (docs/impl/temporal-rerender.md §2), so no smear is expected,
    /// but a picture is.
    #[test]
    fn a_retimed_layer_under_forced_accumulation_mb_is_not_black() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let dir = std::env::temp_dir().join("lumit-matrix-fixture");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(&dir) else {
            eprintln!("skipping: no ffmpeg CLI to write the footage fixture");
            return;
        };
        use lumit_core::retime::Interpolation;
        let rows: Vec<(&str, Option<Property>, Interpolation)> = vec![
            ("no retime", None, Interpolation::Nearest),
            (
                "retime nearest",
                Some(lumit_core::model::Layer::identity_retime(
                    Rational::ZERO,
                    Rational::new(2, 1).unwrap(),
                )),
                Interpolation::Nearest,
            ),
            (
                "retime blend",
                Some(lumit_core::model::Layer::identity_retime(
                    Rational::ZERO,
                    Rational::new(2, 1).unwrap(),
                )),
                Interpolation::Blend,
            ),
        ];
        for (name, retime, interpolation) in rows {
            let mut doc = Document::new();
            let item = Uuid::now_v7();
            doc.items
                .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                    id: item,
                    name: "fixture.mp4".into(),
                    media: lumit_core::model::MediaRef {
                        relative_path: "fixture.mp4".into(),
                        absolute_path: clip.to_string_lossy().into_owned(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    extra: serde_json::Map::new(),
                }));
            let comp_id = Uuid::now_v7();
            let mut clip_layer = matrix_layer("Clip", LayerKind::Footage { item }, 320, 240);
            clip_layer.retime = retime;
            clip_layer.interpolation = interpolation;
            let mut adjust = matrix_layer("Adjust", LayerKind::Adjustment, 320, 240);
            let mut mb = lumit_core::fx::instantiate("accumulation_mb").expect("registered");
            for p in &mut mb.params {
                match p.id.as_str() {
                    "force_all" => p.value = lumit_core::model::EffectValue::Bool(true),
                    "samples" => {
                        p.value = lumit_core::model::EffectValue::Float(Property::fixed(4.0))
                    }
                    _ => {}
                }
            }
            adjust.effects = vec![mb];
            doc.items.push(ProjectItem::Composition(Composition {
                id: comp_id,
                name: "Scene".into(),
                width: 320,
                height: 240,
                frame_rate: FrameRate::new(30, 1).unwrap(),
                duration: Duration(Rational::new(2, 1).unwrap()),
                background: LinearColour::BLACK,
                work_area: None,
                layers: vec![adjust, clip_layer],
                markers: Vec::new(),
                motion_blur: lumit_core::model::MotionBlur::default(),
                extra: serde_json::Map::new(),
            }));
            let store = DocumentStore::new(doc);
            let doc = store.snapshot();
            let (rgba, _, _) = r
                .render_rgba(&doc, comp_id, 10, 1.0)
                .unwrap_or_else(|e| panic!("{name}: render failed: {e}"));
            assert!(
                rgba.chunks_exact(4)
                    .any(|px| px[0] > 8 || px[1] > 8 || px[2] > 8),
                "{name}: a retimed layer under Force on all layers rendered black"
            );
        }
    }

    /// The footage rows of the K-031 matrix: plain footage, Retime blend and
    /// Retime flow — the rows where the two walks run genuinely different
    /// decode machinery, so they are the ones the swap most needs proven.
    /// Skips (with a note) when no ffmpeg CLI is present to write the fixture.
    #[test]
    fn the_preview_and_export_paths_agree_on_footage() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let dir = std::env::temp_dir().join("lumit-matrix-fixture");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(&dir) else {
            eprintln!("skipping: no ffmpeg CLI to write the footage fixture");
            return;
        };

        use lumit_core::retime::{FlowParams, Interpolation};
        // Half speed as the Retime property expresses it (K-249): two seconds
        // of layer time reading one of source. The interpolation policy rides
        // beside it on the layer rather than inside it.
        let half_speed = || {
            lumit_core::model::Layer::identity_retime(Rational::ZERO, Rational::new(2, 1).unwrap())
        };
        let rows: Vec<(&str, Option<lumit_core::anim::Property>, Interpolation, u64)> = vec![
            ("plain footage", None, Interpolation::Nearest, 10),
            ("retime blend", Some(half_speed()), Interpolation::Blend, 7),
            (
                "retime flow",
                Some(half_speed()),
                Interpolation::Flow(FlowParams::default()),
                7,
            ),
        ];

        for (name, retime, interpolation, frame) in rows {
            let mut doc = Document::new();
            let item = Uuid::now_v7();
            doc.items
                .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                    id: item,
                    name: "fixture.mp4".into(),
                    media: lumit_core::model::MediaRef {
                        relative_path: "fixture.mp4".into(),
                        absolute_path: clip.to_string_lossy().into_owned(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    extra: serde_json::Map::new(),
                }));
            let comp_id = Uuid::now_v7();
            let mut layer = matrix_layer("Clip", LayerKind::Footage { item }, 320, 240);
            layer.retime = retime;
            layer.interpolation = interpolation;
            doc.items.push(ProjectItem::Composition(Composition {
                id: comp_id,
                name: "Scene".into(),
                width: 320,
                height: 240,
                frame_rate: FrameRate::new(30, 1).unwrap(),
                duration: Duration(Rational::new(2, 1).unwrap()),
                background: LinearColour::BLACK,
                work_area: None,
                layers: vec![layer],
                markers: Vec::new(),
                motion_blur: lumit_core::model::MotionBlur::default(),
                extra: serde_json::Map::new(),
            }));

            let store = DocumentStore::new(doc);
            let doc = store.snapshot();
            let (preview, pw, ph) = r
                .render_preview(&doc, comp_id, frame, crate::plan::Quality::default(), 1.0)
                .unwrap_or_else(|e| panic!("{name}: preview render failed: {e}"));
            let (export, ew, eh) = r
                .render_rgba(&doc, comp_id, frame, 1.0)
                .unwrap_or_else(|e| panic!("{name}: export render failed: {e}"));
            assert_eq!(
                (pw, ph),
                (ew, eh),
                "{name}: the two paths render at different sizes"
            );
            assert_eq!(
                preview, export,
                "{name}: the interactive and export paths must be bit-identical (K-031)"
            );
        }
    }

    /// A document with one comp holding a full-frame solid of `colour`.
    fn matrix_base(w: u32, h: u32, colour: LinearColour) -> (Document, Uuid, Uuid) {
        let mut doc = Document::new();
        let solid = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: solid,
            name: "Base".into(),
            colour,
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            id: comp_id,
            name: "Scene".into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![matrix_layer("Base", LayerKind::Solid { def: solid }, w, h)],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (doc, comp_id, solid)
    }

    /// A second, smaller solid item plus a layer of it at the top of the stack.
    fn matrix_top(doc: &mut Document, comp_id: Uuid, colour: LinearColour) -> (Uuid, Uuid) {
        let solid = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: solid,
            name: "Top".into(),
            colour,
            width: 12,
            height: 10,
            extra: serde_json::Map::new(),
        }));
        let layer = matrix_layer("Top", LayerKind::Solid { def: solid }, 12, 10);
        let layer_id = layer.id;
        if let Some(comp) = doc.comp_mut(comp_id) {
            // Index 0 = top of the stack.
            comp.layers.insert(0, layer);
        }
        (solid, layer_id)
    }

    /// **What the engine drops, the driver gets back** (K-295).
    ///
    /// The failure this pins is not a slow leak: it is memory that comes back
    /// only when something unrelated happens. Dropping a texture or a buffer
    /// marks it destroyed; the driver reclaims it on the device's next
    /// maintain, and an engine that renders into a cache on a worker thread —
    /// never presenting, often idle — asks for one only by accident. Reported
    /// from a Mac twice, the second time caught mid-act: 5 000 live buffers and
    /// 6 GB, then 8 buffers and 2.9 GB moments later because a panel was
    /// opened.
    ///
    /// **This test earns its keep on macOS**, where the reclamation actually
    /// went wrong and where no allocator report exists to see it — the counts
    /// are what every backend keeps. It renders far more frames than the cache
    /// can hold, so the great majority are evicted and dropped, and then asks
    /// the driver what it still has.
    ///
    /// It asks *twice*, a batch apart, and that is the measurement: how many
    /// objects a backend rests on differs between drivers by a factor of
    /// several, but a driver that never hands a dropped frame back grows by one
    /// object per frame, whichever driver it is. So the gate is that the second
    /// batch leaves the resting set where the first did — not a count tuned to
    /// whichever backend happened to run when it was written.
    #[test]
    fn what_the_engine_drops_the_driver_gets_back() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        // A budget of a few frames, so nearly every render below is evicted.
        r.set_frame_texture_budget(32 * 1024 * 1024);
        let (cw, ch) = (960u32, 540u32);
        let (doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.8, 0.1, 0.1, 1.0]));
        let store = DocumentStore::new(doc);
        let doc = store.snapshot();

        let (textures_before, buffers_before) = r.gpu_live_objects();
        /// Frames per batch: still several times what the budget above can
        /// hold, so the great majority of them are evicted and dropped.
        ///
        /// Two batches of sixty rather than two of a hundred and twenty, so the
        /// whole test does the same hundred and twenty renders it always did.
        /// The second batch is a *comparison*, not extra load, and on a backend
        /// that is leaking the extra load is not free: at two hundred and forty
        /// the Windows runner stopped asserting and started dying
        /// (`STATUS_STACK_BUFFER_OVERRUN`), which measures nothing.
        const BATCH: u64 = 60;
        let render_batch = |r: &mut HeadlessRenderer, batch: u64| {
            for i in 0..BATCH {
                // A name of its own per frame: every render is a new entry, so
                // the store must evict, exactly as a long session makes it.
                let name = batch * BATCH + i;
                let _ = r
                    .render_prepared_named(
                        &doc,
                        comp_id,
                        0,
                        Quality::default(),
                        true,
                        Some(name as u128),
                    )
                    .expect("render");
                // The worker's own turn does this once a loop; the whole point
                // is that it is what makes the dropping stick.
                r.reclaim_gpu();
            }
        };
        // The reading is of the engine *at rest*, and at rest means the card
        // has finished: work is submitted and runs later, so a CPU that has run
        // ahead of it is holding every frame the card has not reached yet, and
        // a non-blocking reclaim cannot free those however many times it is
        // called. Reading after one is reading the backlog — which is why the
        // first version of this measurement grew with the frame count on Metal
        // and D3D12 and stayed flat on the software rasteriser, where the CPU
        // never gets ahead.
        //
        // So the measurement waits. What is still held once the queue is empty
        // is what is genuinely still held, and that is the only number a leak
        // can be read off.
        let settle = |r: &mut HeadlessRenderer| {
            r.settle_gpu();
            r.gpu_live_objects()
        };
        render_batch(&mut r, 0);
        let (textures_one, buffers_one) = settle(&mut r);
        render_batch(&mut r, 1);
        let (textures, buffers) = settle(&mut r);

        // What the engine holds at rest — the frames still in the card's cache,
        // the pooled upload textures, the shared present targets, and one
        // frame's intermediates — is a *backend's* number, not a fact about
        // this engine: 18 textures and 8 buffers on the software rasteriser
        // against 2 and 5 before the first batch, several times that on Metal
        // and on D3D12, both of which keep more of their own bookkeeping alive
        // between maintains. Pinning one of those numbers is what this test did
        // first, and it is why it failed on macOS at 65 having passed at 63 the
        // run before: it was measuring the backend rather than the leak.
        //
        // The leak has a shape no backend changes. Memory that is dropped and
        // never handed back grows by one object per frame for ever — that is
        // what "5 000 live buffers" was — so the resting set after the second
        // batch is the resting set after the first, whatever that set happens
        // to be on this driver. A little slack for what a busier one defers;
        // nothing like the batch of frames that went through it.
        let slack = BATCH / 4;
        assert!(
            textures <= textures_one + slack,
            "a second batch of {BATCH} frames must not leave a texture each \
             behind: {textures_before} before, {textures_one} after one batch, \
             {textures} after two"
        );
        assert!(
            buffers <= buffers_one + slack,
            "nor a buffer each: {buffers_before} before, {buffers_one} after \
             one batch, {buffers} after two"
        );
        // And the card's cache is the thing that decides how many frames are
        // held, not the number of frames that have been made.
        let (used, budget, _) = r.frame_texture_stats();
        assert!(
            used <= budget,
            "the cache stays inside its budget: {used} of {budget}"
        );
    }

    /// **A frame is one command buffer, however many layers it has.**
    ///
    /// Every pass in `lumit-gpu` used to make its own encoder and submit it, so
    /// a frame cost the driver one round trip per layer and per effect —
    /// measured 2026-07-31 at `layers + 2`. All of a frame's passes are in
    /// order on one queue, so they are encoded once and handed over once.
    ///
    /// The gate is the *count*, not a stopwatch. A submit is a round trip whose
    /// cost does not depend on the card, so the number is the honest measure and
    /// it runs anywhere — including on the software rasteriser CI uses, where a
    /// timing would prove nothing (docs/16-ROADMAP.md standing rules).
    ///
    /// What is asserted is the **shape**: the count does not grow with the layer
    /// count. A fixed budget would be a fragile thing to pin, but "adding thirty
    /// layers adds no submissions" is exactly the property that was lost.
    ///
    /// The count is read off **this renderer's own context**. It used to be a
    /// process-wide counter, which quietly made this test a measurement of
    /// whatever else the suite happened to be rendering at the same moment:
    /// green here, red on CI, where there are cores enough for two GPU tests to
    /// overlap (docs/13 §7.0).
    #[test]
    fn a_frame_submits_once_however_many_layers_it_has() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (cw, ch) = (32u32, 16u32);

        // One render's submissions, for a comp with `extra` layers over the base.
        let mut submits_for = |extra: usize| -> u64 {
            let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.8, 0.1, 0.1, 1.0]));
            for _ in 0..extra {
                matrix_top(&mut doc, comp_id, LinearColour([0.1, 0.2, 0.9, 1.0]));
            }
            let store = DocumentStore::new(doc);
            let doc = store.snapshot();
            // A first render warms every lazily-built pipeline and cache, so
            // what the second one submits is the steady state.
            let _ = r.render_rgba(&doc, comp_id, 0, 1.0).unwrap();
            let before = r.gpu.submits_so_far();
            let _ = r.render_rgba(&doc, comp_id, 0, 1.0).unwrap();
            r.gpu.submits_so_far() - before
        };

        let one = submits_for(0);
        let many = submits_for(31);
        assert_eq!(
            one, many,
            "a frame's submissions must not grow with its layers: \
             1 layer submitted {one}, 32 layers submitted {many}"
        );
        // And the constant is small — the walk's one buffer plus the read-back
        // the export path ends with, not a per-pass tail hiding under the
        // equality above.
        assert!(
            one <= 4,
            "a frame should cost a handful of submissions, not {one}"
        );
    }

    /// A consumer layer matted by a hidden source carrying a mask and an
    /// effect, with the matte's sampling mode chosen per row (K-142).
    fn matte_doc(
        w: u32,
        h: u32,
        red: LinearColour,
        blue: LinearColour,
        source: lumit_core::model::LayerInputSource,
    ) -> (Document, Uuid, u64) {
        let mut doc = Document::new();
        let red_solid = Uuid::now_v7();
        let blue_solid = Uuid::now_v7();
        for (id, name, colour, sw, sh) in [
            (red_solid, "Red", red, w, h),
            (blue_solid, "Blue", blue, 12u32, 12u32),
        ] {
            doc.items.push(ProjectItem::Solid(SolidDef {
                id,
                name: name.into(),
                colour,
                width: sw,
                height: sh,
                extra: serde_json::Map::new(),
            }));
        }
        let mut matte_layer = matrix_layer("Matte", LayerKind::Solid { def: blue_solid }, 12, 12);
        matte_layer.switches.visible = false;
        matte_layer
            .masks
            .push(lumit_core::mask::Mask::rectangle(0.0, 0.0, 6.0, 12.0));
        matte_layer
            .effects
            .push(lumit_core::fx::instantiate("invert").unwrap());
        let mut consumer = matrix_layer("Red", LayerKind::Solid { def: red_solid }, w, h);
        consumer.matte = Some(lumit_core::model::MatteRef {
            layer: matte_layer.id,
            channel: lumit_core::model::MatteChannel::Alpha,
            inverted: false,
            source,
        });
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            id: comp_id,
            name: "Scene".into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![consumer, matte_layer],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (doc, comp_id, 0)
    }

    /// An unknown comp id on the interactive path is a calm error, and it must
    /// not disturb the pixels retained for a comp that *does* exist.
    #[test]
    fn an_unknown_comp_is_a_calm_error_on_the_preview_path() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let q = crate::plan::Quality::default();
        r.render_preview(&doc, comp_id, 0, q, 1.0).expect("render");
        let decodes = r.decoded_frames();

        assert!(r.render_preview(&doc, Uuid::now_v7(), 0, q, 1.0).is_err());
        // The good comp still re-composites from its retained pixels.
        r.render_preview(&doc, comp_id, 0, q, 1.0)
            .expect("still fine");
        assert_eq!(r.decoded_frames(), decodes);
    }

    /// Not a correctness test — a stopwatch, run by hand:
    /// `cargo test -p lumit-render --release -- --ignored --nocapture preview_cost`
    ///
    /// It exists because a Dart-side measurement of this cannot be trusted: the
    /// widget-test harness settles in 20 ms slices, so anything measured through
    /// it reports the polling granularity rather than the render.
    #[test]
    #[ignore = "timing, not correctness"]
    fn preview_cost() {
        let Ok(mut renderer) = HeadlessRenderer::new() else {
            lumit_gpu::no_adapter();
            return;
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.2, 0.4, 0.8, 1.0]), 1920, 1080);
        let doc = store.snapshot();

        for (label, scale) in [("full", 1.0f32), ("fit-0.42", 0.42), ("quarter", 0.25)] {
            let quality = Quality {
                draft: false,
                auto_res: scale < 1.0,
                display_scale: scale,
                divisor: 1,
            };
            // Warm: the first render builds pipelines and probes.
            let _ = renderer.render_preview(&doc, comp_id, 0, quality, scale);

            let n = 30u32;
            let started = std::time::Instant::now();
            for frame in 0..n {
                let out = renderer.render_preview(&doc, comp_id, u64::from(frame), quality, scale);
                assert!(out.is_ok(), "{label} frame {frame} failed");
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
            println!("PREVIEW {label:>10} scale={scale:<5} {each:>7.2} ms/frame");
        }
    }

    /// The profiler's two promises at the seam the Viewer actually drives
    /// (docs/13 §7.1): an unwatched render says nothing at all, and a watched
    /// one reports progress that only ever moves forwards and ends at the
    /// presenting stage.
    #[test]
    fn a_watched_render_reports_its_progress_and_an_unwatched_one_says_nothing() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let seen: std::sync::Arc<std::sync::Mutex<Vec<crate::profile::FrameProgress>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = std::sync::Arc::clone(&seen);
        r.set_progress_sink(Some(std::sync::Arc::new(move |p| {
            if let Ok(mut seen) = into.lock() {
                seen.push(p);
            }
        })));
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 8, 8);
        let doc = store.snapshot();

        // Playback's case: a sink is installed but this frame is not watched.
        let _ = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert!(
            seen.lock().expect("reports").is_empty(),
            "an unwatched frame — every frame of playback — reports nothing"
        );

        r.watch_frames(true);
        let _ = r.render_rgba(&doc, comp_id, 1, 1.0).expect("render");
        r.watch_frames(false);
        let reports = seen.lock().expect("reports");
        assert!(!reports.is_empty(), "a watched frame describes itself");
        assert!(
            reports.iter().all(|p| p.frame == 1),
            "every report names the frame it is about"
        );
        let mut last = -1.0_f32;
        for report in reports.iter() {
            assert!(
                report.fraction >= last,
                "progress went backwards: {last} then {}",
                report.fraction
            );
            last = report.fraction;
        }
        assert!(matches!(
            reports.last().map(|p| p.stage),
            Some(crate::profile::RenderStage::Presenting)
        ));
    }

    /// **Editing the last effect of a layer re-runs only that effect, and the
    /// picture is byte-for-byte the cold one** (K-421, K-031).
    ///
    /// End to end through the draw builder: a solid's stack is named from its
    /// colour, size and masks, so after a committed render the blur's output
    /// is held, and a render with the exposure changed runs one kernel. The
    /// warm picture must equal what a renderer that has never seen the comp
    /// makes of the same document — the export path is that cold renderer.
    #[test]
    fn editing_the_last_effect_serves_the_held_prefix_and_matches_a_cold_render() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.8, 0.3, 0.1, 1.0]), 16, 16);
        let layer_id = store.snapshot().comp(comp_id).expect("comp").layers[0].id;
        let blur = lumit_core::fx::instantiate("blur").expect("blur exists");
        let exposure = |stops: f64| {
            let mut e = lumit_core::fx::instantiate("exposure").expect("exposure exists");
            for p in &mut e.params {
                if p.id == "stops" {
                    p.value = lumit_core::model::EffectValue::Float(
                        lumit_core::anim::Property::fixed(stops),
                    );
                }
            }
            e
        };
        store
            .commit(lumit_core::Op::SetLayerEffects {
                comp: comp_id,
                layer: layer_id,
                effects: vec![blur.clone(), exposure(0.0)],
            })
            .expect("effects on");

        r.keep_effect_outputs(true);
        let _ = r
            .render_rgba(&store.snapshot(), comp_id, 0, 1.0)
            .expect("render");
        let (_, (runs, hits)) = r.effect_cache_stats();
        assert_eq!((runs, hits), (2, 0), "a cold frame runs the whole stack");

        store
            .commit(lumit_core::Op::SetLayerEffects {
                comp: comp_id,
                layer: layer_id,
                effects: vec![blur, exposure(1.0)],
            })
            .expect("the last effect edited");
        let doc = store.snapshot();
        let warm = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        let (_, (runs, hits)) = r.effect_cache_stats();
        assert_eq!(
            (runs, hits),
            (3, 1),
            "the blur's output was held; only the exposure ran"
        );

        let mut cold = HeadlessRenderer::new().expect("a second renderer");
        let cold = cold.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert_eq!(warm, cold, "a warm preview is the picture an export makes");

        r.clear_frame_textures();
        assert_eq!(
            r.effect_cache_stats().0 .2,
            0,
            "Clear cache empties the intermediates with the frames"
        );
    }

    /// **A precomp's frames are cached as one unit** (K-422, K-031).
    ///
    /// A nested comp is realised once and then served by its own name: a
    /// parent edit, and a second parent frame the nested comp is static
    /// across, both serve it held; an edit inside it realises it again; the
    /// warm picture is byte-for-byte what a cold renderer (the export path)
    /// makes; a collapsed Precomp is never cached, since its inner draws
    /// composite against the parent's stack; and Clear cache empties it.
    #[test]
    fn a_nested_comp_is_realised_once_and_served_by_its_own_name() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let red = LinearColour([0.8, 0.1, 0.1, 1.0]);
        let blue = LinearColour([0.1, 0.1, 0.8, 1.0]);
        let (mut doc, comp_id, _) = matrix_base(16, 16, red);
        let (child_doc, child_id, child_solid) = matrix_base(16, 16, blue);
        for item in child_doc.items {
            doc.items.push(item);
        }
        let layer = matrix_layer("Nested", LayerKind::Precomp { comp: child_id }, 16, 16);
        let layer_id = layer.id;
        doc.comp_mut(comp_id).unwrap().layers.insert(0, layer);
        let store = DocumentStore::new(doc);

        r.keep_effect_outputs(true);
        let _ = r
            .render_rgba(&store.snapshot(), comp_id, 0, 1.0)
            .expect("render");
        assert_eq!(r.nested_frame_counts(), (1, 0), "a cold frame realises it");

        // A parent-only edit: the Precomp layer turns, the nested comp does not.
        store
            .commit(lumit_core::Op::SetTransformProperty {
                comp: comp_id,
                layer: layer_id,
                prop: lumit_core::model::TransformProp::Rotation,
                animation: lumit_core::anim::Animation::Static(30.0),
            })
            .expect("rotated");
        let doc = store.snapshot();
        let warm = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert_eq!(r.nested_frame_counts(), (1, 1), "served by its own name");
        let _ = r.render_rgba(&doc, comp_id, 1, 1.0).expect("render");
        assert_eq!(
            r.nested_frame_counts(),
            (1, 2),
            "a second parent frame the nested comp is static across is served too"
        );

        let mut cold = HeadlessRenderer::new().expect("a second renderer");
        let cold = cold.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert_eq!(warm, cold, "a warm preview is the picture an export makes");

        // An edit inside the nested comp renames its frame.
        store
            .commit(lumit_core::Op::SetSolidDef {
                def: child_solid,
                name: "Base".into(),
                colour: LinearColour([0.1, 0.8, 0.1, 1.0]),
                width: 16,
                height: 16,
            })
            .expect("recoloured");
        let inner = r
            .render_rgba(&store.snapshot(), comp_id, 0, 1.0)
            .expect("render");
        assert_eq!(
            r.nested_frame_counts(),
            (2, 2),
            "an inner edit realises it again"
        );
        assert_ne!(inner, warm, "and the picture changed");

        // A collapsed Precomp is spliced into the parent, never named.
        store
            .commit(lumit_core::Op::SetLayerCollapse {
                comp: comp_id,
                layer: layer_id,
                collapse: true,
            })
            .expect("collapsed");
        let doc = store.snapshot();
        let _ = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        let _ = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert_eq!(
            r.nested_frame_counts(),
            (2, 2),
            "a collapsed precomp is never cached"
        );

        r.clear_frame_textures();
        assert_eq!(r.effect_cache_stats().0 .2, 0, "Clear cache empties it");
    }

    /// **A parent edit does not decode the footage inside a held precomp**
    /// (K-422), and the picture it serves is the cold one. The planner skips
    /// the nested comp's jobs on the store's word, and the realiser then finds
    /// the texture the planner pinned — so a frame rendered with no nested
    /// pixels in hand is still the right frame. Skips without an ffmpeg CLI to
    /// write the fixture.
    #[test]
    fn a_parent_edit_does_not_decode_the_footage_inside_a_held_precomp() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let dir = std::env::temp_dir().join("lumit-matrix-fixture");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(&dir) else {
            eprintln!("skipping: no ffmpeg CLI to write the footage fixture");
            return;
        };
        let mut doc = Document::new();
        let item = Uuid::now_v7();
        doc.items
            .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                id: item,
                name: "fixture.mp4".into(),
                media: lumit_core::model::MediaRef {
                    relative_path: "fixture.mp4".into(),
                    absolute_path: clip.to_string_lossy().into_owned(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
            }));
        let (inner_doc, inner_id, _) = matrix_base(32, 24, LinearColour([0.0, 0.0, 0.0, 0.0]));
        for it in inner_doc.items {
            doc.items.push(it);
        }
        doc.comp_mut(inner_id).unwrap().layers =
            vec![matrix_layer("Clip", LayerKind::Footage { item }, 32, 24)];
        let (outer_doc, comp_id, _) = matrix_base(32, 24, LinearColour([0.2, 0.2, 0.2, 1.0]));
        for it in outer_doc.items {
            doc.items.push(it);
        }
        let pre = matrix_layer("Nested", LayerKind::Precomp { comp: inner_id }, 32, 24);
        let pre_id = pre.id;
        doc.comp_mut(comp_id).unwrap().layers.insert(0, pre);
        let store = DocumentStore::new(doc);
        let q = crate::plan::Quality::default();

        r.keep_effect_outputs(true);
        r.render_preview(&store.snapshot(), comp_id, 3, q, 1.0)
            .expect("cold");
        let decoded = r.retained.as_ref().map_or(0, |r| r.jobs.len());
        assert_eq!(decoded, 1, "the nested footage is what this comp decodes");

        store
            .commit(lumit_core::Op::SetTransformProperty {
                comp: comp_id,
                layer: pre_id,
                prop: lumit_core::model::TransformProp::Rotation,
                animation: lumit_core::anim::Animation::Static(20.0),
            })
            .expect("rotated");
        let doc = store.snapshot();
        let (warm, w, h) = r.render_preview(&doc, comp_id, 3, q, 1.0).expect("warm");
        assert_eq!(
            r.nested_frame_counts(),
            (1, 1),
            "served from the pinned texture"
        );
        let decoded = r.retained.as_ref().map_or(0, |r| r.jobs.len());
        assert_eq!(decoded, 0, "and the plan asked for no decode at all");

        let mut cold = HeadlessRenderer::new().expect("a second renderer");
        let (cold, cw, ch) = cold.render_preview(&doc, comp_id, 3, q, 1.0).expect("cold");
        assert_eq!((w, h), (cw, ch));
        assert_eq!(
            warm, cold,
            "the held nested frame is the picture a cold walk makes"
        );
    }

    /// A measured frame lands its milliseconds on the right rows: the layer
    /// that was drawn, and the effect instance inside it that ran. Without the
    /// ids threaded from the resolve through the draw list, this is where a
    /// wrong (or absent) attribution shows.
    #[test]
    fn a_measured_frame_names_the_layer_and_the_effect_it_timed() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 16, 16);
        // One real effect on the one layer, so there is something to attribute.
        let blur = lumit_core::fx::instantiate("blur").expect("blur exists");
        let (layer_id, effect_id) = {
            let doc = store.snapshot();
            let comp = doc.comp(comp_id).expect("comp");
            (comp.layers[0].id, blur.id)
        };
        store
            .commit(lumit_core::Op::SetLayerEffects {
                comp: comp_id,
                layer: layer_id,
                effects: vec![blur],
            })
            .expect("the effect goes on the layer");

        let profiles: std::sync::Arc<std::sync::Mutex<Vec<crate::profile::FrameProfile>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = std::sync::Arc::clone(&profiles);
        r.set_profile_sink(Some(std::sync::Arc::new(move |p| {
            if let Ok(mut got) = into.lock() {
                got.push(p);
            }
        })));

        let doc = store.snapshot();
        // Unmeasured first: a sink alone must not start costing anything.
        let _ = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert!(profiles.lock().expect("profiles").is_empty());

        r.measure_frames(true);
        let _ = r.render_rgba(&doc, comp_id, 2, 1.0).expect("render");
        r.measure_frames(false);

        let got = profiles.lock().expect("profiles");
        let profile = got.last().expect("the measured frame reported");
        assert_eq!(profile.frame, 2);
        assert_eq!(profile.layers.len(), 1, "one layer in the comp, one row");
        let layer = &profile.layers[0];
        assert_eq!(layer.layer, layer_id, "the row is the layer that drew");
        assert_eq!(
            layer.effects.iter().map(|e| e.effect).collect::<Vec<_>>(),
            vec![effect_id],
            "the effect's own instance id carries its cost"
        );
        assert!(
            profile.total_ms >= layer.ms,
            "the frame cannot cost less than the layer inside it"
        );
        assert!(
            layer.ms >= 0.0 && layer.effects[0].ms >= 0.0,
            "measured times are real durations"
        );
    }

    /// **Measuring gives the batching up, deliberately.**
    ///
    /// A frame's passes are recorded into one command buffer and submitted at
    /// the end, so a fence taken mid-walk would wait on a queue that has not
    /// been handed over and time nothing real. A *measured* frame therefore
    /// flushes at each layer and each effect before it fences — which is the
    /// cost the stopwatch already declares (K-276: measuring waits for the card
    /// at each layer, which is why it is opt-in and never runs during playback).
    ///
    /// So the property is the opposite of the unmeasured one: an unmeasured
    /// frame's submissions do not grow with its layers, and a measured frame's
    /// do. Asserting both together is what stops a future change from
    /// "optimising" the flush away and silently turning the render-time column
    /// into a measure of how long Lumit takes to *describe* a layer.
    #[test]
    fn a_measured_frame_hands_its_work_over_layer_by_layer() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (cw, ch) = (32u32, 16u32);
        let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.8, 0.1, 0.1, 1.0]));
        for _ in 0..15 {
            matrix_top(&mut doc, comp_id, LinearColour([0.1, 0.2, 0.9, 1.0]));
        }
        let store = DocumentStore::new(doc);
        let doc = store.snapshot();
        // A frame is only measured when the switch is on *and* the numbers have
        // somewhere to go, so the sink is part of turning measuring on.
        let seen: std::sync::Arc<std::sync::Mutex<Vec<crate::profile::FrameProfile>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = std::sync::Arc::clone(&seen);
        r.set_profile_sink(Some(std::sync::Arc::new(move |p| {
            if let Ok(mut got) = into.lock() {
                got.push(p);
            }
        })));
        // Warm the lazily-built pipelines first, so neither count below
        // includes one-off setup.
        let _ = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");

        let before = r.gpu.submits_so_far();
        let _ = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        let unmeasured = r.gpu.submits_so_far() - before;

        r.measure_frames(true);
        let before = r.gpu.submits_so_far();
        let _ = r.render_rgba(&doc, comp_id, 1, 1.0).expect("render");
        let measured = r.gpu.submits_so_far() - before;
        r.measure_frames(false);

        assert!(
            !seen.lock().expect("profiles").is_empty(),
            "the frame was supposed to be measured; without that this proves nothing"
        );
        assert!(
            measured > unmeasured,
            "a measured frame must hand its work over as it goes, or its \
             numbers are encoding time rather than GPU time \
             (measured {measured}, unmeasured {unmeasured})"
        );
        assert!(
            unmeasured <= 4,
            "an ordinary frame is one command buffer plus its read-back, \
             not {unmeasured}"
        );
    }

    /// **A precomp set as a track matte must actually gate the layer** (K-268).
    ///
    /// The regression: a comp has no pixels until it is rendered, so the draw
    /// builder's `pixels_for` answered None for a Precomp matte source and the
    /// whole matte quietly disappeared — the consumer drew everywhere, as if
    /// no matte had been set. K-266 fixed the same hole for the *layer-input*
    /// mattes (a flare source, a DoF depth pass); the track matte, which is
    /// how everyone actually reaches for a precomp matte, still had it.
    ///
    /// The scene: a full-frame red solid matted by a hidden precomp layer whose
    /// own 16×16 blue solid covers the LEFT half of a 32×16 comp. Red survives
    /// where the precomp has alpha and nowhere else.
    #[test]
    fn a_precomp_track_matte_gates_the_layer() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (cw, ch) = (32u32, 16u32);
        let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.8, 0.1, 0.1, 1.0]));
        let (child_doc, child_id, _) = matrix_base(16, 16, LinearColour([0.1, 0.2, 0.9, 1.0]));
        for item in child_doc.items {
            doc.items.push(item);
        }
        // The matte itself is hidden, as a matte source always is.
        let mut matte = matrix_layer("Matte", LayerKind::Precomp { comp: child_id }, 16, 16);
        matte.switches.visible = false;
        let matte_id = matte.id;
        {
            let comp = doc.comp_mut(comp_id).unwrap();
            comp.layers[0].matte = Some(lumit_core::model::MatteRef {
                layer: matte_id,
                channel: lumit_core::model::MatteChannel::Alpha,
                inverted: false,
                source: lumit_core::model::LayerInputSource::default(),
            });
            comp.layers.push(matte);
        }

        let (rgba, w, h) = r
            .render_rgba(&std::sync::Arc::new(doc.clone()), comp_id, 0, 1.0)
            .expect("render");
        assert_eq!((w, h), (cw, ch));
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3])
        };
        // Left half: inside the precomp's opaque square, so the red shows.
        let (lr, _lg, _lb, la) = at(8, 8);
        assert!(lr > 150, "red should survive under the matte, got {lr}");
        assert_eq!(la, 255, "and it should be opaque there");
        // Right half: the precomp is transparent there, so nothing is drawn —
        // this is the pixel that stayed red while the matte was being dropped.
        let (rr, rg, rb, _ra) = at(24, 8);
        assert!(
            rr < 30 && rg < 30 && rb < 30,
            "outside the precomp matte the layer must be gated out, got {:?}",
            (rr, rg, rb)
        );
    }

    /// **An effect ON a Precomp layer must keep its px@comp parameters where
    /// they were put when the preview renders at a reduced resolution**
    /// (K-268, the twin of K-266's adjustment-layer fix).
    ///
    /// The regression: the stack of a Precomp layer resolves against the nested
    /// comp's full width (factor 1) but runs on the nested comp's *preview*
    /// raster, so every px@comp parameter — a Transform's offset here, a
    /// flare's light or a blur radius in the wild — landed further across the
    /// picture the coarser the preview got. Preview-only drift: full resolution
    /// was always right.
    ///
    /// The scene: a 32×32 precomp of solid white, offset 8 px right by a
    /// Transform effect on the precomp layer. Eight of thirty-two is a quarter
    /// of the frame at every resolution, so the same fractions are empty and
    /// filled at Full and at Half.
    #[test]
    fn an_effect_on_a_precomp_layer_keeps_its_pixels_under_half_preview() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let (cw, ch) = (32u32, 32u32);
        let white = LinearColour([1.0, 1.0, 1.0, 1.0]);
        let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.0, 0.0, 0.0, 1.0]));
        let (child_doc, child_id, _) = matrix_base(cw, ch, white);
        for item in child_doc.items {
            doc.items.push(item);
        }
        // The base black solid only exists to give matrix_base a comp; the
        // precomp layer covers it entirely, so what is measured is the
        // precomp's own white, shifted.
        let mut nested = matrix_layer("Nested", LayerKind::Precomp { comp: child_id }, cw, ch);
        let mut fx = lumit_core::fx::instantiate("transform").unwrap();
        for p in &mut fx.params {
            let v = match p.id.as_str() {
                "position_x" => 8.0,
                "anchor_x" | "anchor_y" | "position_y" | "rotation" => 0.0,
                _ => continue,
            };
            p.value = lumit_core::model::EffectValue::Float(Property::fixed(v));
        }
        nested.effects.push(fx);
        // Index 0 is the top of the stack, over the black base.
        doc.comp_mut(comp_id).unwrap().layers.insert(0, nested);

        // The white starts a quarter of the way across, at both resolutions.
        for (label, scale) in [("full", 1.0f32), ("half", 0.5f32)] {
            let quality = Quality {
                auto_res: scale < 1.0,
                display_scale: scale,
                ..Quality::default()
            };
            let (rgba, w, h) = r
                .render_preview(
                    &std::sync::Arc::new(doc.clone()),
                    comp_id,
                    0,
                    quality,
                    scale,
                )
                .expect("render");
            let at = |fx: f32| {
                let x = ((w as f32 * fx) as u32).min(w - 1);
                let y = h / 2;
                let i = ((y * w + x) * 4) as usize;
                rgba[i]
            };
            assert!(
                at(0.125) < 40,
                "{label}: the first eighth is behind the offset, got {}",
                at(0.125)
            );
            assert!(
                at(0.375) > 200,
                "{label}: three eighths across is inside the shifted picture, got {}",
                at(0.375)
            );
        }
    }
}
