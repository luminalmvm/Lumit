use std::{eprintln, println, sync::mpsc::Receiver};

use crate::api::composition::BridgePlaybackMode;
use flutter_rust_bridge::frb;
use lumit_core::model::EffectInstance;
use lumit_render::{HeadlessRenderer, PreviewEngine};

// The quality policy is v0's, shared rather than copied: two implementations of
// "what does a scale of 0.5 mean for the decode" would drift, and the two
// frontends would then decode at different sizes for the same on-screen scale.
use crate::render::quality_for;
use uuid::Uuid;

// Each frame type is only constructed by its own platform's `publish_frame`, so
// importing both unconditionally would warn on one of them in every build.
// Windows and macOS share the handle-shaped frame: an opaque integer naming a
// surface (an NT handle there, an `IOSurfaceID` here) plus its size, which is
// why they share this import, the `publish_zero_copy` body and the Dart
// `RenderedSharedTexture` case (K-195).
#[cfg(any(
    all(windows, feature = "shared-texture"),
    all(target_os = "macos", feature = "shared-texture-macos")
))]
use crate::api::state::BridgeSharedFrameInfo;
#[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
use crate::api::state::BridgeSharedFrameInfoLinux;

use crate::api::{
    composition::CompositionReference,
    layer::LayerReference,
    project::ProjectReference,
    state::{WorkerResponse, WorkerResponseStream},
    BridgeError,
};

#[frb(ignore)]
pub struct WorkerState {
    /// The realtime preview-tier controller (K-030/K-171). Held so the worker
    /// can feed it measured render costs and read the tier back, which is not
    /// wired yet — see docs/TODO.md, "Bridge".
    #[allow(dead_code)]
    pub preview_engine: PreviewEngine,
    /// The session's renderer, owned outright by this thread — no lock, because
    /// nothing else touches it. Every `publish_frame` variant reads it.
    pub renderer: HeadlessRenderer,
    pub project: ProjectReference,
    /// Playback, when it is running. `None` means the worker is idle and blocks
    /// waiting for something to do.
    playback: Option<Playback>,
    /// The decode-ahead thread (docs/impl/playback-scheduler.md §5): playback
    /// posts the source decodes coming frames will need, and files the results
    /// into the renderer's cache, so decode runs alongside compositing rather
    /// than before it.
    prefetcher: crate::prefetch::Prefetcher,
    /// Where the user is looking — the comp, frame and scale last shown — the
    /// idle cache-fill's anchor (docs/06 §5.5).
    last_shown: Option<(CompositionReference, u64, f32)>,
    /// The disk tier (docs/06 §5.4) and its IO thread. Owned here because this
    /// is the thread that has both halves of every hand-off: the renderer whose
    /// evictions fall to disk, and the frame keys to file them under.
    disk: lumit_render::diskio::DiskIo,
    /// Disk frames asked for and not yet arrived: the position each was asked
    /// for (a frame off disk carries only its name, and putting it back on the
    /// card records where it sits), and when it was asked — every-frame
    /// playback gives a young ask a bounded moment to land before compositing
    /// the frame anyway ([`crate::playback::wait_for_disk`]).
    disk_wanted: std::collections::HashMap<u128, DiskWant>,
    /// The frame-name memo ([`crate::names::NameCache`]): a name is a hash of
    /// the whole composition at that frame, and the bar, the look-ahead and
    /// the fill all ask for the same ones — each is computed once per edit.
    names: crate::names::NameCache,
    /// The disk budget last applied, the clear count last honoured, and the
    /// location epoch last opened — all arriving as atomics from the settings
    /// ops (see [`crate::framecache::disk`]).
    applied_disk_budget: u64,
    seen_disk_clears: u64,
    seen_disk_location: (u64, Option<std::path::PathBuf>),
    /// The VRAM budget last applied and the clear-request count last honoured
    /// (both arrive as atomics from the settings ops — see
    /// [`crate::framecache::vram`]).
    applied_vram_budget: usize,
    seen_vram_clears: u64,
    /// `(used, entries)` last published for the VRAM meter, so an unchanged
    /// cache publishes nothing.
    published_vram: (u64, u64),
    /// What the cache bar's published strip was computed from, so an unchanged
    /// world is not hashed again: what the bar asked for, the document revision,
    /// and each tier's own change counter.
    published_bar: Option<BarFingerprint>,
    /// The strip as last published, and how far the refinement pass has got
    /// through it — see [`publish_cache_bar`]. Kept between turns so a long
    /// composition converges to per-frame truth instead of staying sampled.
    bar_strip: Vec<u8>,
    bar_refined_to: u64,
    /// True when a frame was painted straight into the strip since the last
    /// publish ([`mark_banked`]) — what tells the publish to nudge the
    /// frontend even though the sweep itself wrote nothing new.
    bar_dirty: bool,
    /// When the strip was last published — see [`BAR_MIN_INTERVAL`].
    bar_published_at: std::time::Instant,
    /// True when the idle fill has nothing left to do (everything near the
    /// playhead is held, or the budget is full). Cleared whenever the anchor,
    /// the document or the budget moves.
    fill_exhausted: bool,
    /// True when every frame held on the card is on disk as well, so the idle
    /// backup has nothing to copy ([`idle_backup`]). Cleared whenever a frame
    /// is banked or the disk tier is re-opened, since either can make work.
    backup_exhausted: bool,
    /// When the last request arrived — the fill waits out a ~200 ms lull
    /// after it (docs/06 §5.5), so a scrub in progress is never contended.
    last_request: std::time::Instant,
    /// The flare-bake generation this worker has already reacted to (K-350).
    /// When the renderer's moves past it, a bake has been queued or has landed
    /// — and a landing is the moment the picture on screen stops being the
    /// right one, because the frame showing was drawn with the lens before it.
    bakes_seen: u64,
    /// The one soloed-layer render the dropper is reading, against the
    /// `(comp, frame, layer, generation)` it was made for — see
    /// [`sample_layer_alone`]. One entry, because the dropper only ever reads
    /// one layer at a time and a pointer drag asks for the same one on every
    /// move.
    layer_sample: Option<LayerSample>,
    /// A frame the user asked for while the render-time column was measuring,
    /// that a tier served instead of a composite — so it has no numbers yet.
    /// The idle turn composites it once more, measured, and discards the
    /// picture (K-420: serve the hit, measure afterwards). One slot: only the
    /// frame being looked at is worth the numbers.
    pending_measure: Option<(Uuid, u64, lumit_render::Quality)>,
}

/// One outstanding ask to the disk tier: where the frame sits (for the upload
/// that follows) and when it was asked (for the bounded grace).
#[frb(ignore)]
struct DiskWant {
    provenance: lumit_render::FrameProvenance,
    asked: std::time::Instant,
}

/// Name one frame through the worker's memo: computed at most once per
/// document revision however many consumers ask, served as a lookup after
/// that. [`lumit_render::HeadlessRenderer::presync_items`] must have run for
/// this document **and this composition** first — it probes what that comp can
/// show, and an unprobed source makes the frame unnameable (`None`), never
/// wrongly named.
#[frb(ignore)]
fn frame_name(
    state: &mut WorkerState,
    document: &std::sync::Arc<lumit_core::Document>,
    revision: u64,
    comp: Uuid,
    frame: u64,
    quality: lumit_render::Quality,
) -> Option<u128> {
    let WorkerState {
        renderer, names, ..
    } = state;
    names.get_or_compute(revision, comp, frame, quality.tag(), || {
        renderer.frame_key_presynced(document, comp, frame, quality)
    })
}

/// One soloed-layer render, held for the dropper (see [`sample_layer_alone`]).
#[frb(ignore)]
struct LayerSample {
    /// `(comp, frame, layer, document revision)` — what this render is of. The
    /// revision is what retires it when an edit lands: the frame cache has no
    /// generation counter any more (K-214 names every frame by its content), and
    /// a soloed render is of a document nobody else holds, thus it cannot be
    /// named the same way.
    stamp: (Uuid, u64, Uuid, u64),
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Whose turn it is to build a renderer (K-434). Held only across
/// [`HeadlessRenderer::new`] — never across a render, a lock on document state,
/// or anything that can await — so it orders GPU device creation and nothing
/// else. See the note at its one use in [`worker_loop`] for why the order
/// matters.
///
/// This is the one lock in the engine deliberately held across driver work,
/// against docs/14 §"A lock MUST NOT be held across … an FFI call". The rule
/// exists so a slow call cannot stall a thread waiting for *data*; this guards
/// no data, and the slow call is the very thing being ordered — a queue for it,
/// written as a mutex because that is what a queue of one is. K-434 records the
/// deviation and what it buys.
static BUILDING_RENDERER: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Whether a renderer is still worth building for this project (K-434).
///
/// Asked once, after the worker's turn to build has come round: a project
/// closed while it waited has nothing left to draw and no one left to ask, so
/// the device would be built only to be dropped. `state` is the liveness test —
/// [`crate::api::project::ProjectReference::close`] forgets the project, and
/// every call through the reference answers `InvalidProject` from then on.
#[frb(ignore)]
fn worth_building_for(project: &ProjectReference) -> bool {
    project.state().is_ok()
}

/// How often the cache bar's strip may be recomputed. Building it names every
/// frame of the composition, which is a hash apiece — cheap per frame, worth
/// bounding across a long one. A tenth of a second is far finer than the eye
/// needs on a progress stripe and leaves the worker's core to the fill.
const BAR_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

/// The same, while playback is running.
///
/// This walk shares the thread that renders frames, and during playback that
/// thread has a deadline: every millisecond spent naming frames for the stripe
/// is a millisecond the next frame does not have. Half a second still fills the
/// bar visibly as playback lays frames down — a stripe is not something read at
/// frame precision — and it cuts the work to a third of what an idle editor,
/// which has the whole thread to itself, is happy to spend.
const BAR_MIN_INTERVAL_PLAYING: std::time::Duration = std::time::Duration::from_millis(500);

/// The most frames named in the bar's **first** pass over a composition. A
/// longer one is sampled every `frames / this` frames, each sample filling its
/// stride, so the whole stripe has an answer immediately rather than filling in
/// from the left. The refinement pass below then replaces those samples with
/// per-frame truth.
const BAR_MAX_SAMPLES: u64 = 1024;

/// How many frames the refinement pass names per turn. The first pass gives the
/// whole stripe a coarse answer; this walks it in chunks, replacing each sample
/// with the frames it stood for, so a composition of any length reaches per-frame
/// truth within a second or two of standing still — and no single turn costs more
/// than the first pass did.
const BAR_REFINE_PER_TURN: u64 = 1024;

/// The same, while playback is running. The sweep shares the thread that
/// renders frames; with the name memo ([`crate::names::NameCache`]) most of a
/// sweep is lookups, but right after an edit every name is a full comp hash,
/// and a thousand of those in one turn is several frames of deadline. Small
/// here — the strip still refreshes end to end within a few publish intervals
/// on any composition a bar can usefully draw.
const BAR_REFINE_PER_TURN_PLAYING: u64 = 256;

/// The coarser preview scales worth probing for the bar's dimmed state: the
/// adaptive tiers the realtime controller actually drops to (Half, Third,
/// Quarter — [`crate::realtime::tier_scale`]). Under content keying the scale is
/// inside the name, so "held at *some* coarser scale" cannot be read off a hash;
/// these are the scales frames genuinely get cached at, which is what the dimmed
/// state exists to report.
const BAR_COARSE_TIERS: [f32; 3] = [0.5, 1.0 / 3.0, 0.25];

/// What a published cache-bar strip was computed from. Recomputed only when one
/// of these moves, so an editor sitting still hashes nothing.
#[frb(ignore)]
#[derive(PartialEq, Eq, Clone, Copy)]
struct BarFingerprint {
    comp: Uuid,
    frames: u64,
    scale_q: u16,
    revision: u64,
    /// The VRAM cache's change counter — it moves on every insert, clear and
    /// resize, which is what catches a cache at its budget swapping one frame
    /// for another (both totals stay put while the holdings change).
    vram_version: u64,
    ram_entries: u64,
    disk_entries: u64,
}

/// Apply cross-thread cache controls, move frames between the tiers, and keep
/// the meters and the cache bar fresh — run once per worker loop turn, cheap
/// when nothing changed (a handful of atomic loads). `stream` carries the
/// cache-bar's redraw nudge (see [`publish_cache_bar`]).
#[frb(ignore)]
fn sync_caches(state: &mut WorkerState, stream: &mut WorkerResponseStream) {
    // Nothing here clears a tier because the document changed. It used to: the
    // frames were named by position, so a committed edit did not rename any of
    // them and the only safe answer was to throw them all away. They are named
    // by content now (K-178, docs/06 §5.2), so an edit renames exactly the frames
    // it changed and leaves the rest addressable — which is what keeps the cache
    // bar green through a rename, and what makes an undo instantly valid.
    let budget = crate::framecache::vram::budget();
    if budget != state.applied_vram_budget {
        state.applied_vram_budget = budget;
        // One VRAM budget, shared: a quarter holds the per-effect and nested
        // intermediates (K-421, K-422), the rest holds finished frames. Without
        // the split the intermediates sat outside the number the user set.
        let intermediates = budget / 4;
        state.renderer.set_effect_cache_budget(intermediates);
        state
            .renderer
            .set_frame_texture_budget(budget - intermediates);
        state.fill_exhausted = false;
    }
    // What the cache is holding to, read back from the cache itself rather than
    // from the wish above — so the meter cannot claim a budget the renderer
    // never took. Both stores count: the frames and the intermediates.
    crate::framecache::vram::publish_applied(
        state.renderer.frame_texture_stats().1 + state.renderer.effect_cache_stats().0 .1,
    );
    let clears = crate::framecache::vram::clears();
    if clears != state.seen_vram_clears {
        state.seen_vram_clears = clears;
        state.renderer.clear_frame_textures();
        state.fill_exhausted = false;
    }
    crate::framecache::publish_comp_decodes(state.renderer.decoded_frames());
    // The decoded-frame pool's share of the memory report (K-294). Published on
    // the same turn as the rest, so the numbers a report adds up were all read
    // at one moment rather than across a second of drift.
    let (decoded_bytes, decoders) = state.renderer.decode_memory();
    crate::framecache::decode::publish(decoded_bytes as u64, decoders as u64);
    crate::framecache::disk::publish_pending_parks(state.disk.pending_parks() as u64);
    // Hand back what this turn dropped (K-295). A frame that has been evicted,
    // a read-back that has been taken, an intermediate the compositor finished
    // with: all of them are only *marked* destroyed when they are dropped, and
    // the driver reclaims them on the device's next maintain. Rendering into a
    // cache on a worker thread never asks for one, so without this line they
    // sat un-freed until something else happened to poll — which is how the
    // editor reached tens of gigabytes while idle.
    //
    // Non-blocking, and once a turn: it drains what has already finished.
    state.renderer.reclaim_gpu();

    // The driver's own accounting. The byte figures are Vulkan and D3D12 only
    // — Metal keeps none — so the live-object counts ride with them: those
    // every backend keeps, and they are what says whether a dropped frame was
    // actually destroyed (K-294).
    let (allocated, reserved) = state.renderer.gpu_allocator_bytes().unwrap_or((0, 0));
    let (textures, buffers) = state.renderer.gpu_live_objects();
    crate::framecache::gpu::publish(allocated, reserved, textures, buffers);
    crate::framecache::gpu::publish_unified(state.renderer.unified_memory());
    let (used, _, entries) = state.renderer.frame_texture_stats();
    if (used as u64, entries as u64) != state.published_vram {
        state.published_vram = (used as u64, entries as u64);
        crate::framecache::vram::publish(used as u64, entries as u64);
        // What the card holds has changed, thus there may be something new to
        // copy down. Cheaper than asking the backup itself, which would walk
        // every held frame to find out.
        state.backup_exhausted = false;
    }

    sync_disk(state);
    drain_demotions(state);
    collect_disk_loads(state);
    publish_cache_bar(state, stream);
}

/// Keep the disk tier pointed at the right folder and inside its budget, and
/// mirror what it holds (docs/06 §5.4).
#[frb(ignore)]
fn sync_disk(state: &mut WorkerState) {
    let (epoch, location) = crate::framecache::disk::location();
    let root = disk_root(state, &location);
    if (epoch, root.clone()) != state.seen_disk_location {
        state.seen_disk_location = (epoch, root.clone());
        crate::framecache::disk::publish_root(
            root.as_ref().map(|r| r.to_string_lossy().into_owned()),
        );
        _ = state
            .disk
            .tx
            .send(lumit_render::diskio::Cmd::SetRoot(root.clone()));
        // A different folder holds different frames, so there may be something
        // to promote again — and everything held is unparked as far as the new
        // folder is concerned, thus there is a backup to make.
        state.fill_exhausted = false;
        state.backup_exhausted = false;
    }
    let budget = crate::framecache::disk::budget();
    if budget != state.applied_disk_budget {
        state.applied_disk_budget = budget;
        _ = state
            .disk
            .tx
            .send(lumit_render::diskio::Cmd::SetCap(budget));
    }
    let clears = crate::framecache::disk::clears();
    if clears != state.seen_disk_clears {
        state.seen_disk_clears = clears;
        _ = state.disk.tx.send(lumit_render::diskio::Cmd::Clear);
    }
    let (disk_used, disk_entries) = state.disk.stats();
    crate::framecache::disk::publish(disk_used, disk_entries);
}

/// Where this project's parked frames belong, for the location the user chose.
///
/// `None` means the tier stays off — only possible on a platform with no home
/// directory at all, since an unsaved project falls back to the application's
/// own cache folder rather than losing the tier (a project caches from the
/// moment it is created; the document's id is in the `.lum` and survives every
/// save, so its frames are still there tomorrow).
#[frb(ignore)]
fn disk_root(
    state: &WorkerState,
    location: &crate::framecache::disk::Location,
) -> Option<std::path::PathBuf> {
    use crate::framecache::disk::Location;
    let (doc_id, own, path) = {
        let project = state.project.state().ok()?;
        let project = project.read().ok()?;
        let document = project.store.snapshot();
        (
            document.id,
            document.cache_location.clone(),
            project.path.clone(),
        )
    };
    // The project's own answer wins where it has one: a project told to cache on
    // a scratch drive, or beside itself, should do that whatever the application
    // is set to (docs/06 §5.4).
    let location = match own {
        Some(lumit_core::model::CacheLocation::AppData) => Location::AppData,
        Some(lumit_core::model::CacheLocation::BesideProject) => Location::BesideProject,
        Some(lumit_core::model::CacheLocation::Custom { folder }) if !folder.is_empty() => {
            Location::Custom(std::path::PathBuf::from(folder))
        }
        // A custom location with no folder in it is not a location.
        Some(lumit_core::model::CacheLocation::Custom { .. }) => Location::AppData,
        None => location.clone(),
    };
    match location {
        Location::AppData => lumit_project::frame_cache_dir(doc_id),
        Location::BesideProject => match path.as_deref() {
            Some(path) => lumit_render::diskio::sidecar_root(path),
            // Nowhere to sit beside yet.
            None => lumit_project::frame_cache_dir(doc_id),
        },
        Location::Custom(root) => match path.as_deref() {
            Some(path) => lumit_render::diskio::cache_root_for(path, Some(&root)),
            None => Some(root.join(format!("{doc_id}-cache"))),
        },
    }
}

/// Collect the frames the graphics card has finished handing back, and put them
/// where they belong: in memory, and parked on disk (docs/06 §5.3's ladder).
///
/// Both are handed over rather than chained — a frame goes to disk on the way
/// down, not when memory later forgets it, so an editor that crashes has still
/// banked what it rendered.
#[frb(ignore)]
fn drain_demotions(state: &mut WorkerState) {
    for mut demoted in state.renderer.poll_demotions() {
        // One allocation for both tiers. The frame goes to memory and to disk at
        // the same time, and it is 8 MB at 1080p, thus a copy for each tier was
        // the most costly part of a demotion.
        let bytes = std::sync::Arc::new(std::mem::take(&mut demoted.rgba));
        // `park` rather than a bare send: it refuses a frame already on its way
        // down and refuses everything once the queue is full, which is what
        // keeps a write-behind queue from becoming a memory leak (K-277). A
        // refusal costs this frame its place on disk and nothing else — it is
        // still on the card and in memory, and it will be offered again.
        //
        // What it cost and what size it was made at go with it, so the disk
        // tier's cap can weigh it against its neighbours rather than taking
        // whatever was written first (docs/06 §5.3).
        _ = state.disk.park(
            demoted.key,
            demoted.width,
            demoted.height,
            demoted.bgra,
            bytes.clone(),
            demoted.cost_ms,
            demoted.provenance.scale_q,
        );
        crate::framecache::put_demoted(demoted.key, &demoted, bytes);
    }
}

/// Put frames that have come back off disk onto the graphics card, so a promoted
/// frame is shown without compositing anything (docs/06 §5.1: "promotes
/// disk→RAM→VRAM ahead of the playhead").
#[frb(ignore)]
fn collect_disk_loads(state: &mut WorkerState) {
    let loaded: Vec<_> = state.disk.loaded.try_iter().collect();
    for frame in loaded {
        let Some(want) = state.disk_wanted.remove(&frame.hash) else {
            // Nobody is waiting for it any more (a comp switch, a clear); the
            // frame is still on disk and will be asked for again if wanted.
            continue;
        };
        // A share stays in memory as well as going up to the card: when the
        // comp is bigger than the VRAM budget, the next pass over this frame
        // otherwise read the same file again — every pass, for ever — and the
        // IO thread's rate became the playback rate.
        let bytes = std::sync::Arc::new(frame.bytes);
        crate::framecache::put_loaded(
            frame.hash,
            frame.width,
            frame.height,
            frame.bgra,
            DISK_PROMOTION_COST_MS,
            want.provenance,
            bytes.clone(),
        );
        let promoted = state
            .renderer
            .upload_frame_texture(lumit_render::Promotion {
                key: frame.hash,
                bgra: frame.bgra,
                width: frame.width,
                height: frame.height,
                bytes: &bytes,
                // Dear enough to hold on to: a frame that reached disk was worth
                // reading back, and re-rendering it is what this saved.
                cost_ms: DISK_PROMOTION_COST_MS,
                provenance: want.provenance,
            });
        if promoted.is_some() {
            state.fill_exhausted = false;
            mark_banked(
                state.published_bar,
                &mut state.bar_strip,
                &mut state.bar_dirty,
                want.provenance.comp,
                want.provenance.frame,
                want.provenance.scale_q,
            );
        }
    }
}

/// The recompute cost a promoted frame is credited with. It is not measured —
/// the render that made it happened in another session, possibly on another day
/// — so it is stated: a frame that earned its way to disk is dear enough that
/// the store should not throw it out ahead of a trivial one.
const DISK_PROMOTION_COST_MS: u32 = 16;

/// How many coming frames' disk copies are asked for before the first render
/// turn of a run (`start_playback`). Sized past the deepest ring (16) with
/// room over for the IO thread to be mid-queue; each ask is one message, so
/// over-asking costs a few reads at worst, never a stall.
const DISK_PRE_ASK: u64 = 32;

/// Paint one just-banked frame straight into the worker's strip, when the
/// strip being shown is of that composition at that scale.
///
/// This is what turns frames green WHILE playback lays them down. The sweep
/// walks forward from the playhead and wraps, so the frames playback just
/// banked — always just *behind* the playhead — are the last it reaches, and
/// the stripe sat visibly unchanged until a pause let the sweep catch up. The
/// bank itself knows exactly which frame it filed; telling the strip directly
/// costs one array write against the sweep's comp hash per frame.
///
/// Takes the fields rather than the whole worker state, because two of its
/// callers run while the playback state is borrowed and the borrow checker is
/// right to insist the paths stay disjoint.
#[frb(ignore)]
fn mark_banked(
    published: Option<BarFingerprint>,
    strip: &mut [u8],
    dirty: &mut bool,
    comp: Uuid,
    frame: u64,
    scale_q: u16,
) {
    let Some(fingerprint) = published else {
        return;
    };
    if fingerprint.comp != comp || fingerprint.scale_q != scale_q {
        return;
    }
    let Ok(slot) = usize::try_from(frame) else {
        return;
    };
    if let Some(value) = strip.get_mut(slot) {
        if *value != 2 {
            *value = 2;
            *dirty = true;
        }
    }
}

/// Compute and publish the cache bar's per-frame strip (docs/06 §5.6).
///
/// The bar leaves a note saying which composition, how many frames and what
/// preview scale it is drawing; this names each of those frames and asks the
/// three tiers whether they hold it. The interface never touches a cache itself
/// — it could not, since naming a frame needs the renderer's probe results, and
/// hashing hundreds of frames is not work for the thread that paints.
///
/// **Two passes, because the two things the bar owes are in tension.** It owes an
/// answer for the whole composition straight away — a stripe that fills in from
/// one end looks like the *cache* filling in from one end — and it owes the truth
/// per frame, which on a long composition is tens of thousands of hashes. So the
/// first pass samples the whole strip, one frame per stride standing for its
/// neighbours, and a refinement pass then walks it in bounded chunks replacing
/// each sample with the frames it stood for. A composition short enough to name
/// in one go has a stride of one, and its first pass *is* the truth.
///
/// The refinement walk starts at the frame last shown and wraps, so the part of
/// the bar the user is actually looking at is the part that firms up first.
#[frb(ignore)]
fn publish_cache_bar(state: &mut WorkerState, stream: &mut WorkerResponseStream) {
    let Some((comp_id, frames, scale_q)) = crate::framecache::bar::wanted() else {
        return;
    };
    if frames == 0 {
        return;
    }
    let interval = if state.playback.is_some() {
        BAR_MIN_INTERVAL_PLAYING
    } else {
        BAR_MIN_INTERVAL
    };
    if state.bar_published_at.elapsed() < interval {
        return;
    }
    let (document, revision) = {
        let Ok(project) = state.project.state() else {
            return;
        };
        let Ok(project) = project.read() else {
            return;
        };
        (project.store.snapshot(), project.store.revision())
    };
    let (ram_entries, disk_entries) = (crate::framecache::stats().2 as u64, state.disk.stats().1);
    let fingerprint = BarFingerprint {
        comp: comp_id,
        frames,
        scale_q,
        revision,
        vram_version: state.renderer.frame_texture_version(),
        ram_entries,
        disk_entries,
    };
    let was = state.published_bar;
    let changed = was != Some(fingerprint);
    // Nothing has moved and the strip is already true per frame: nothing to do.
    // Note the second half — an unchanged world is exactly when the refinement
    // pass gets to make progress, so this cannot simply return on `!changed`.
    if !changed && state.bar_refined_to >= frames {
        return;
    }
    if document.comp(comp_id).is_none() {
        return;
    }
    // Whether every frame's *name* may have changed, as against merely which of
    // them are held. A different composition, length, scale or document revision
    // renames frames, so the strip means nothing and is rebuilt; a frame merely
    // arriving in a tier leaves the names alone, so the strip stands and only its
    // values need refreshing.
    let renamed = was.is_none_or(|old| {
        (old.comp, old.frames, old.scale_q, old.revision) != (comp_id, frames, scale_q, revision)
    });
    state.published_bar = Some(fingerprint);
    state.bar_published_at = std::time::Instant::now();

    let bgra = zero_copy_wants_bgra();
    let scale = f32::from(scale_q) / 1000.0;
    let quality = quality_for(scale);
    let stride = frames.div_ceil(BAR_MAX_SAMPLES).max(1);
    let playing = state.playback.is_some();
    let per_turn = if playing {
        BAR_REFINE_PER_TURN_PLAYING
    } else {
        BAR_REFINE_PER_TURN
    };
    // Every name below is of this one snapshot: probe it once, so the memo's
    // misses are hashes and nothing else (see `frame_name`).
    state.renderer.presync_items(&document, comp_id);

    // Naming one frame needs the renderer, the document and the three tiers; the
    // walk over frames needs none of them. Split so the walk can be tested
    // without a graphics card (see `bar_strip_tests`).
    //
    // The strip is taken out of the worker for the duration: naming a frame wants
    // the whole of `state`, and holding a borrow of one of its fields across that
    // is what the borrow checker is for.
    let mut strip = std::mem::take(&mut state.bar_strip);
    // What the strip held going in, so the publish below knows whether this
    // turn's sweep changed anything the frontend should redraw for. Direct
    // paints since the last publish are already in `strip`; `bar_dirty` is
    // what remembers those.
    let before = strip.clone();
    // A sweep in progress finishes before any restart. During playback the
    // holdings move constantly — every promoted frame is an insert — and
    // restarting the sweep on each movement meant it started from zero at
    // every publish and never converged: the whole strip was renamed from
    // scratch, on the render thread, for as long as playback promoted frames.
    // Only a COMPLETED sweep restarts to pick up moved holdings; renamed
    // frames rebuild outright below regardless.
    let mut refined_to = if changed && state.bar_refined_to >= frames {
        0
    } else {
        state.bar_refined_to
    };
    let rebuild = renamed || strip.len() != frames as usize;
    let anchor = match &state.last_shown {
        Some((comp, frame, _)) if comp.id == comp_id => *frame % frames,
        _ => 0,
    };
    {
        let mut tier_of = |frame: u64| {
            frame_tier(
                state, &document, revision, comp_id, frame, quality, scale, bgra, playing,
            )
        };
        if rebuild {
            let sampled = sample_bar_strip(frames, stride, &mut tier_of);
            strip = sampled.tiers;
            refined_to = sampled.refined_to;
        } else {
            // Names are the same but holdings may have moved: sweep again from
            // the anchor, keeping the strip on screen while it refreshes.
            refined_to = refine_bar_strip(&mut strip, anchor, refined_to, per_turn, &mut tier_of);
        }
    }
    state.bar_strip = strip;
    state.bar_refined_to = refined_to;
    let pixels_changed = state.bar_dirty || state.bar_strip != before;
    state.bar_dirty = false;
    crate::framecache::bar::publish(comp_id, scale_q, state.bar_strip.clone());
    // Nudge the frontend when the strip's PIXELS moved. The bar widget redraws
    // only when it hears this (`cacheChanged` in Dart) — and until now nothing
    // said it during playback, so the stripe stayed exactly as it was at the
    // press of play and only caught up at the pause, when the idle fill's own
    // nudges resumed.
    if pixels_changed {
        _ = stream.add(WorkerResponse::CacheFilled);
    }
}

/// What the bar should draw for one frame: `0` nothing, `1` held coarser, `2`
/// held at this scale, `3` on disk coarser, `4` on disk at this scale. Playable
/// beats promotable — a frame both held and parked reads as held.
///
/// Names go through the memo ([`frame_name`]); the caller has presynced the
/// document. `fast` is set while playback runs: the walk then shares the
/// render thread's deadline, so a frame parked at this scale is reported as
/// such without paying the three coarser probes (a comp hash apiece on a memo
/// miss) — the one answer that changes is the rare frame both parked at scale
/// and held coarser, which reads blue for a publish instead of dimmed green.
#[allow(clippy::too_many_arguments)]
#[frb(ignore)]
fn frame_tier(
    state: &mut WorkerState,
    document: &std::sync::Arc<lumit_core::Document>,
    revision: u64,
    comp: Uuid,
    frame: u64,
    quality: lumit_render::Quality,
    scale: f32,
    bgra: bool,
    fast: bool,
) -> u8 {
    let mut on_disk_at_scale = false;
    if let Some(key) = frame_name(state, document, revision, comp, frame, quality) {
        if state.renderer.has_frame_texture(key, bgra) || crate::framecache::contains(key) {
            return 2;
        }
        on_disk_at_scale = state.disk.contains(key);
    }
    if fast && on_disk_at_scale {
        return 4;
    }
    let mut on_disk_coarser = false;
    for factor in BAR_COARSE_TIERS {
        let coarser = quality_for(scale * factor);
        let Some(key) = frame_name(state, document, revision, comp, frame, coarser) else {
            continue;
        };
        if state.renderer.has_frame_texture(key, bgra) || crate::framecache::contains(key) {
            return 1;
        }
        on_disk_coarser |= state.disk.contains(key);
    }
    if on_disk_at_scale {
        4
    } else if on_disk_coarser {
        3
    } else {
        0
    }
}

/// A freshly sampled strip and how much of it counts as exact.
#[frb(ignore)]
struct SampledStrip {
    tiers: Vec<u8>,
    refined_to: u64,
}

/// The first pass: name one frame per `stride` and let it stand for the frames it
/// skipped, so the whole stripe has an answer at once (see
/// [`publish_cache_bar`]). A stride of one names everything, and says so by
/// reporting itself fully refined.
///
/// A skipped run is only painted when its sample is held: an uncached sample
/// leaves its neighbours as nothing, which is what they are until something says
/// otherwise. The reverse — painting a whole stride green off one held frame and
/// correcting it later — would flash cache the user does not have.
#[frb(ignore)]
fn sample_bar_strip(frames: u64, stride: u64, tier_of: &mut dyn FnMut(u64) -> u8) -> SampledStrip {
    let stride = stride.max(1);
    let mut tiers = vec![0u8; frames as usize];
    let mut sample = 0u64;
    while sample < frames {
        let tier = tier_of(sample);
        if tier != 0 {
            let end = (sample + stride).min(frames);
            for slot in &mut tiers[sample as usize..end as usize] {
                *slot = tier;
            }
        }
        sample += stride;
    }
    let refined_to = if stride == 1 { frames } else { 0 };
    SampledStrip { tiers, refined_to }
}

/// One turn of the refinement pass: name up to `per_turn` more frames, starting
/// `refined_to` steps on from `anchor` and wrapping, and write each answer into
/// its own slot. Returns how far the sweep has now got.
///
/// Wrapping from the anchor rather than walking from frame zero is what puts the
/// part of the bar under the playhead first in the queue — on a long composition
/// the difference is whether the region you are looking at firms up now or in a
/// few seconds.
#[frb(ignore)]
fn refine_bar_strip(
    tiers: &mut [u8],
    anchor: u64,
    refined_to: u64,
    per_turn: u64,
    tier_of: &mut dyn FnMut(u64) -> u8,
) -> u64 {
    let frames = tiers.len() as u64;
    if frames == 0 {
        return 0;
    }
    let end = refined_to.saturating_add(per_turn).min(frames);
    for step in refined_to..end {
        let frame = (anchor + step) % frames;
        let tier = tier_of(frame);
        if let Some(slot) = tiers.get_mut(frame as usize) {
            *slot = tier;
        }
    }
    end
}

/// Whether this build's zero-copy transport wants BGRA (Windows and macOS) or
/// RGBA (Linux, and any build without one). The channel order is part of a
/// cached texture's identity, so every consumer has to ask the same question.
#[frb(ignore)]
fn zero_copy_wants_bgra() -> bool {
    cfg!(any(
        all(windows, feature = "shared-texture"),
        all(target_os = "macos", feature = "shared-texture-macos")
    ))
}

/// Put a frame playback will want soon onto the graphics card **before** it is
/// due — the "ahead of the playhead" half of docs/06 §5.1.
///
/// # Why each rung needs its own lead time
///
/// The ring renders ahead of the clock, thus a frame is composited before it is
/// shown. What was *not* ahead of anything was the trip up the ladder. Both
/// lower rungs were climbed at the moment the frame was wanted, inside the
/// render turn:
///
/// * **From memory** the climb is an upload, and an upload is quick — but it
///   happened on the worker thread in the middle of the turn that had to
///   produce the frame, so it was paid out of that frame's budget rather than
///   out of the slack the ring exists to bank.
/// * **From disk** the climb is a message to another thread, and the bytes come
///   back one or two turns of the loop later. Asked for at the moment the frame
///   was due, they always arrived too late for it, and playback composited the
///   frame from the beginning instead — so a span sitting in a file was worth
///   nothing to the pass that went over it.
///
/// This does both in advance, over the same look-ahead window whose source
/// decodes are already posted. By the time the ring reaches the frame it is a
/// hit on the card and no composite happens at all.
///
/// Nothing here waits. A copy that has not arrived is simply not there yet,
/// and the ordinary path composites as it always did. The caller names the
/// frame (through the memo, [`frame_name`]) — a frame that cannot be named yet
/// is not held anywhere under any name, so there is nothing to line up.
///
/// Returns whether it did work the caller should count: an upload happened.
/// Asking the disk for a copy is a message to another thread and costs this one
/// nothing, thus it does not count.
#[frb(ignore)]
fn line_up_frame(
    renderer: &mut lumit_render::HeadlessRenderer,
    disk: &lumit_render::diskio::DiskIo,
    disk_wanted: &mut std::collections::HashMap<u128, DiskWant>,
    key: u128,
    provenance: lumit_render::FrameProvenance,
) -> bool {
    let bgra = zero_copy_wants_bgra();
    if renderer.has_frame_texture(key, bgra) {
        // Already where it needs to be.
        return false;
    }
    // Memory first: it is one upload away, which is cheaper than a file and far
    // cheaper than a composite. Doing it now means the render turn finds a hit.
    if let Some(held) = crate::framecache::held(key) {
        // Only the order it came down in can go back up: the other order would
        // show with red and blue swapped.
        if held.bgra == bgra {
            return renderer
                .upload_frame_texture(lumit_render::Promotion {
                    key,
                    bgra,
                    width: held.width,
                    height: held.height,
                    bytes: &held.bytes,
                    cost_ms: held.cost_ms,
                    provenance,
                })
                .is_some();
        }
    }
    if !wants_disk_lead(
        false,
        crate::framecache::contains(key),
        disk.contains(key),
        disk_wanted.contains_key(&key),
    ) {
        return false;
    }
    disk_wanted.insert(
        key,
        DiskWant {
            provenance,
            asked: std::time::Instant::now(),
        },
    );
    _ = disk
        .tx
        .send(lumit_render::diskio::Cmd::Load { hash: key, bgra });
    false
}

/// Whether a coming frame is worth a read off disk.
///
/// Only one of the four answers leads to a read: the frame is on disk, and no
/// tier above holds it, and nobody has asked for it yet. A read in any other
/// case is IO for a frame that is already there, or a second copy of a read that
/// is already running.
#[frb(ignore)]
fn wants_disk_lead(on_card: bool, in_memory: bool, on_disk: bool, already_asked: bool) -> bool {
    on_disk && !on_card && !in_memory && !already_asked
}

/// The quality one **still** frame is made and named under — a scrub, a drag
/// preview, or the republish after a lens bake.
///
/// It is the scale the caller asked for, and deliberately **not** scaled by
/// the adaptive tier (K-372). The tier is playback's own instrument: it buys a
/// cheaper composite so a run keeps time, and [`playback_quality`] applies it
/// where that trade is being made. Nothing still is being paced.
///
/// The idle fill and the display path must derive this the same way or the
/// fill banks frames the scrub cannot find, so they both come here.
#[frb(ignore)]
fn still_quality(scale: f32) -> lumit_render::Quality {
    quality_for(scale)
}

/// The quality one **playback** frame is made and named under: the run's scale,
/// coarsened by the adaptive tier when the run is Adaptive (K-186).
///
/// `tier` is passed rather than read so the trade is visible at the call site,
/// where the cost it explains is measured — and so the distinction from
/// [`still_quality`] can be tested without a global.
#[frb(ignore)]
fn playback_quality(scale: f32, mode: BridgePlaybackMode, tier: u32) -> lumit_render::Quality {
    let effective = if matches!(mode, BridgePlaybackMode::Adaptive) {
        scale * crate::realtime::tier_scale(tier)
    } else {
        scale
    };
    quality_for(effective)
}

/// Get one frame ready to show, taking the cheapest route the tiers allow
/// (docs/06 §5.1: VRAM first, then promote from the tiers below, and only then
/// composite).
///
/// The order is the ladder itself:
///
/// 1. **Already on the card** — [`HeadlessRenderer::render_prepared`] answers
///    from its own cache without compositing.
/// 2. **Held in memory** — uploaded straight back into a texture, which is a
///    fraction of a composite and the reason the RAM tier exists at all. During
///    playback this has usually happened already, in advance
///    ([`line_up_frame`]); what is left here is the frame nobody looked ahead
///    for — a scrub, or the first frame of a pass.
/// 3. **Parked on disk** — *asked for*, never waited on. A disk read plus
///    decompression is not something to hold the preview open for, so the frame
///    is composited now and the copy off disk lands a turn or two later, in time
///    for the next visit. This is what makes reopening a project warm up as you
///    scrub rather than only where the fill has reached. Playback does not wait
///    for that second visit: it asks for the coming frames in advance
///    ([`line_up_frame`]), thus a parked span plays from the card.
/// 4. **Composited**, and banked on the way past.
#[frb(ignore)]
fn prepare_frame(
    state: &mut WorkerState,
    document: &std::sync::Arc<lumit_core::Document>,
    comp: Uuid,
    frame: u64,
    quality: lumit_render::Quality,
    bgra: bool,
    cacheable: bool,
) -> Result<lumit_render::PreparedFrame, String> {
    let name = cacheable
        .then(|| state.renderer.frame_key(document, comp, frame, quality))
        .flatten();
    // A held frame is served even while the render-time column is measuring
    // (K-420). A frame promoted from memory cost a copy, not a composite, so
    // it has no per-layer numbers to give; the worker notes that below and
    // composites it again for the numbers when it is next idle, rather than
    // making the user wait for a picture the bar already shows as held.
    let measuring = state.renderer.measuring();
    if let Some(key) = name.filter(|key| !state.renderer.has_frame_texture(*key, bgra)) {
        let provenance = lumit_render::FrameProvenance {
            comp,
            frame,
            scale_q: lumit_render::preview_scale_q(quality),
            quality,
        };
        match crate::framecache::held(key) {
            // Only the order it came down in can go back up: a frame read in the
            // other channel order would show with red and blue swapped, so it is
            // left for the composite below.
            Some(held) if held.bgra == bgra => {
                if let Some(prepared) =
                    state
                        .renderer
                        .upload_frame_texture(lumit_render::Promotion {
                            key,
                            bgra,
                            width: held.width,
                            height: held.height,
                            bytes: &held.bytes,
                            cost_ms: held.cost_ms,
                            provenance,
                        })
                {
                    mark_banked(
                        state.published_bar,
                        &mut state.bar_strip,
                        &mut state.bar_dirty,
                        comp,
                        frame,
                        provenance.scale_q,
                    );
                    if measuring {
                        state.pending_measure = Some((comp, frame, quality));
                    }
                    return Ok(prepared);
                }
            }
            Some(_) => {}
            None => {
                if state.disk.contains(key) && !state.disk_wanted.contains_key(&key) {
                    state.disk_wanted.insert(
                        key,
                        DiskWant {
                            provenance,
                            asked: std::time::Instant::now(),
                        },
                    );
                    _ = state
                        .disk
                        .tx
                        .send(lumit_render::diskio::Cmd::Load { hash: key, bgra });
                }
            }
        }
    }
    // Named once, above: hashing the composition again here would be the same
    // walk twice per frame.
    let hits_before = state.renderer.frame_texture_hits();
    let prepared = state
        .renderer
        .render_prepared_named(document, comp, frame, quality, bgra, name);
    // Answered from the card: no numbers were made, so note the frame for the
    // idle measure (K-420).
    if measuring && state.renderer.frame_texture_hits() > hits_before {
        state.pending_measure = Some((comp, frame, quality));
    }
    // A nameable frame that rendered was banked in the same breath: the strip
    // hears it now rather than when the sweep next comes past.
    if prepared.is_ok() && name.is_some() {
        mark_banked(
            state.published_bar,
            &mut state.bar_strip,
            &mut state.bar_dirty,
            comp,
            frame,
            lumit_render::preview_scale_q(quality),
        );
    }
    prepared
}

/// Composite the frame a tier served while the column was measuring, measured
/// this time, and throw the picture away (K-420: serve the hit, measure
/// afterwards).
///
/// The picture on screen is already right — it came from a tier, and a tier
/// holds exactly what a composite would make — so nothing is published here.
/// Only the numbers are wanted, and the profile sink carries those as they
/// are made. Rendered unnamed, so it neither displaces the held frame nor
/// banks a second copy of it. Nothing is measured unless the column still
/// wants numbers, and a frame whose composition has gone is simply dropped.
#[frb(ignore)]
fn measure_pending(state: &mut WorkerState) {
    let Some((comp, frame, quality)) = state.pending_measure.take() else {
        return;
    };
    if !crate::profiling::wanted() {
        return;
    }
    let Ok(document) = state.project.state() else {
        return;
    };
    let Ok(document) = document.read() else {
        return;
    };
    let document = document.store.snapshot();
    measure_frame(&mut state.renderer, &document, comp, frame, quality);
}

/// The measured composite itself — apart from the worker's state so a test
/// can drive it with a renderer alone.
#[frb(ignore)]
fn measure_frame(
    renderer: &mut HeadlessRenderer,
    document: &std::sync::Arc<lumit_core::Document>,
    comp: Uuid,
    frame: u64,
    quality: lumit_render::Quality,
) {
    renderer.measure_frames(true);
    // The result is the numbers, which the profile sink has already carried
    // off; the texture is dropped. A fault here is a fault the scrub will
    // report in its own right.
    _ = renderer.render_prepared_named(
        document,
        comp,
        frame,
        quality,
        zero_copy_wants_bgra(),
        None,
    );
    renderer.measure_frames(false);
}

/// Copy ONE held frame down to disk while the editor is idle — so a session
/// that never fills the card's cache still leaves something for tomorrow.
///
/// # Why this exists
///
/// A frame used to reach the disk tier by one route only: it was pushed out of
/// the card's cache, read back on the way down, and parked. That route needs the
/// cache to be **full**. Give it a budget bigger than a session ever fills —
/// 10 GB on a roomy card — and it is never full, nothing is ever pushed out, and
/// nothing is ever written to disk. The tier whose whole purpose is to make
/// tomorrow start warm stayed empty, and the *more* memory the user gave the
/// cache the more certainly it stayed empty. Exactly the wrong way round, and
/// invisible: the cache bar was green all session, and green again as nothing
/// after a restart.
///
/// So the ladder has a second way down, used only when there is time to spare.
/// The frame stays on the card and keeps serving the Viewer; a copy goes to
/// memory and to disk. One frame per turn, and never more than the read-backs
/// already in flight allow, so this can never compete with the picture.
///
/// It runs on the same lull as the fill but is *not* gated on the fill being
/// finished: on a long composition the fill has frames to make for as long as
/// the budget lasts, and waiting for it to run out would mean waiting for ever.
#[frb(ignore)]
fn idle_backup(state: &mut WorkerState) {
    // Nowhere to put them: the disk tier is off (no project folder yet, or no
    // home directory). Nothing to do until that changes, which `sync_disk`
    // reports by clearing the flag.
    if state.seen_disk_location.1.is_none() {
        state.backup_exhausted = true;
        return;
    }
    // Two disjoint borrows of the worker's state: the renderer walks its held
    // frames, the disk mirror answers which of them are already parked.
    //
    // **Already parked OR already on its way** (K-277). A frame counts as
    // parked only once the write has finished, so asking `contains` alone made
    // every frame in the write queue look like one that had never been
    // offered: this loop wakes every couple of milliseconds, so it read the
    // same frames off the card and queued them again and again, each copy a
    // whole frame of memory behind a thread already behind. That is how the
    // application reached tens of gigabytes while sitting idle.
    let disk = &state.disk;
    if !state
        .renderer
        .start_backup(&|hash| disk.contains(hash) || disk.is_pending(hash))
    {
        state.backup_exhausted = true;
    }
}

/// Render ONE uncached frame near the playhead into the VRAM frame cache —
/// Make the shown frame again when a Lens flare's bake has landed (K-350).
///
/// While a bake is in flight the Viewer keeps showing the lens before it — that
/// is the whole point, a wait instead of a freeze — but nothing else would ever
/// ask for that frame again, so without this the picture would stay one lens
/// behind until the user moved something. Nothing happens on any other tick:
/// the generation only moves when a bake is queued or lands, so an idle editor
/// with no flare in it does not so much as compare two numbers twice.
#[frb(ignore)]
fn republish_after_bake(state: &mut WorkerState, stream: &mut WorkerResponseStream) {
    let now = state.renderer.flare_bake_generation();
    if now == state.bakes_seen {
        return;
    }
    // Still baking: the number moved because one was *queued*. Wait for it —
    // republishing now would make another provisional frame.
    if state.renderer.flare_bake_pending() {
        return;
    }
    state.bakes_seen = now;
    // The fill was stopped while the frames were unnameable; there is
    // something to bank again.
    state.fill_exhausted = false;
    let Some((comp_ref, frame, scale)) = state.last_shown.clone() else {
        return;
    };
    let Ok(project) = state.project.state() else {
        return;
    };
    let Ok(document) = project.read().map(|held| held.store.snapshot()) else {
        return;
    };
    drop(project);
    publish_frame(
        state,
        comp_ref.id,
        frame,
        scale,
        &document,
        stream,
        BridgePlaybackMode::EveryFrame,
        true,
    );
}

/// the idle-time background fill (docs/06 §5.5, forward-biased per
/// [`crate::playback::fill_order`]). One frame per call so a request arriving
/// mid-fill waits at most one render; sets `fill_exhausted` when there is
/// nothing (or no room) left, so an idle editor stops spending the GPU.
#[frb(ignore)]
fn idle_fill(state: &mut WorkerState, stream: &mut WorkerResponseStream) {
    // A non-neutral Viewer view makes every frame unnameable (K-314), so the
    // fill would render frame after frame and bank none of them, never reach
    // the end of its list, and never stop — GPU work for nothing, for as long
    // as the exposure is off zero. There is nothing to fill while the picture
    // is not the composite, so the fill is simply finished.
    if !state.renderer.display_view().is_neutral() {
        state.fill_exhausted = true;
        return;
    }
    // Same shape while a Lens flare's bake is being made (K-350): every frame
    // is unnameable until it lands, so filling would render and bank nothing.
    // `republish_after_bake` sets the fill going again when it does.
    if state.renderer.flare_bake_pending() {
        state.fill_exhausted = true;
        return;
    }
    let Some((comp_ref, anchor, scale)) = state.last_shown.clone() else {
        state.fill_exhausted = true;
        return;
    };
    let (document, revision) = {
        let Ok(document) = state.project.state() else {
            state.fill_exhausted = true;
            return;
        };
        let Ok(document) = document.read() else {
            state.fill_exhausted = true;
            return;
        };
        (document.store.snapshot(), document.store.revision())
    };
    let Some(comp) = document.comp(comp_ref.id) else {
        state.fill_exhausted = true;
        return;
    };
    let frames = comp
        .frame_rate
        .frame_at(lumit_core::time::CompTime(comp.duration.0))
        .max(1) as u64;
    // The work area bounds the fill when one is set (§5.5); else the comp.
    // Both ends are taken through `max(0)` before they are cast: a work area
    // from an older project file may sit outside the comp, and a negative frame
    // number cast unsigned is not a small number, it is an enormous one.
    let (first, last) = match comp.work_area {
        Some((a, b)) => (
            comp.frame_rate.frame_at(a).max(0) as u64,
            (comp.frame_rate.frame_at(b).max(0) as u64).min(frames - 1),
        ),
        None => (0, frames - 1),
    };
    // The same derivation the display path uses, so a frame the fill banks is
    // one a scrub can find (K-372).
    let quality = still_quality(scale);
    let bgra = zero_copy_wants_bgra();
    let (_, budget, _) = state.renderer.frame_texture_stats();
    let (cw, ch) = (comp.width, comp.height);
    let s = scale.clamp(0.05, 1.0);
    let frame_bytes = ((cw as f32 * s) as usize).max(1) * ((ch as f32 * s) as usize).max(1) * 4;
    // A budget that cannot hold one frame is a budget the fill cannot use: the
    // frame would evict itself the moment it landed.
    if budget < frame_bytes {
        state.fill_exhausted = true;
        return;
    }
    // **The fill does not stop at the card.** It used to keep a window around
    // the playhead, as many frames as the card's budget holds, and stop there
    // — which meant playback looping a work area longer than the window
    // re-rendered its far side every pass, while the worker sat idle. Now the
    // walk carries on: a frame rendered once the card is full pushes the
    // card's stalest frame out, and an eviction is a read-back into the RAM
    // tier (and on to disk). The eviction decision stays with the LRU, which
    // drops the stalest and cheapest first. What the walk leaves behind is the
    // card full and the rest of the work area held below it, so a loop that
    // fits in VRAM plus RAM plays warm from end to end. The reach is bounded
    // by both budgets, so the walk never cycles frames through disk.
    //
    // A frame held below is **climbed only while the card has room**. Once it
    // is full, promoting one would push another down — and the next turn
    // would promote that one, for ever. A frame in memory is warm enough; it
    // goes up when playback asks for it.
    let (_, ram_budget, _, _, _) = crate::framecache::stats();
    // ponytail: the RAM budget is shared with every other comp and scale; a
    // fill that counts only its own frames can over-reach by what they hold.
    let reach = (budget + ram_budget) / frame_bytes;
    // Frames the disk holds are asked for as the walk passes them, and the walk
    // carries on. A load is a message to another thread, thus queueing several
    // costs nothing here — and by the time the fill comes round again they have
    // arrived and gone onto the card, which is what makes a re-opened project
    // warm up by *reading* rather than by rendering everything a second time.
    // The walk's names are all of this one snapshot: probe it once, then
    // each name is computed at most once per edit (see `frame_name`).
    state.renderer.presync_items(&document, comp_ref.id);
    for frame in crate::playback::fill_order(anchor, first, last).take(reach) {
        // Naming the frame is what tells the fill whether there is anything to
        // do — and under content keying the name is the same one every tier files
        // it under, so a frame already held anywhere is skipped without a render.
        if let Some(key) = frame_name(state, &document, revision, comp_ref.id, frame, quality) {
            if state.renderer.has_frame_texture(key, bgra) {
                continue;
            }
            // **Held below: climb it, do not make it again.** The fill has no
            // deadline — that is what makes it the fill — thus a frame that
            // already exists in memory or in a file must never be composited
            // afresh. `prepare_frame` below would do that for a parked frame:
            // it asks for the copy and composites anyway, which is right when a
            // frame is due *now* and wrong here. After a re-opened project this
            // is the difference between reading a session's cache back and
            // rendering the whole of it a second time.
            let in_memory = crate::framecache::contains(key);
            if in_memory || state.disk.contains(key) {
                let (used, budget, _) = state.renderer.frame_texture_stats();
                let room = used + frame_bytes <= budget;
                // A full card climbs nothing, from memory or from disk: a
                // promotion would push another frame down, and the next turn
                // would promote that one, round and round through the disk
                // for ever. Frames below stay where they are until playback
                // or a scrub asks for them.
                if !room {
                    continue;
                }
                let provenance = lumit_render::FrameProvenance {
                    comp: comp_ref.id,
                    frame,
                    scale_q: lumit_render::preview_scale_q(quality),
                    quality,
                };
                let uploaded = line_up_frame(
                    &mut state.renderer,
                    &state.disk,
                    &mut state.disk_wanted,
                    key,
                    provenance,
                );
                // An upload is this turn's work, exactly as a render would be:
                // a request arriving mid-fill then waits for one frame, not for
                // a window's worth. Asking the disk costs this thread nothing,
                // so the walk carries on queueing those.
                if uploaded {
                    mark_banked(
                        state.published_bar,
                        &mut state.bar_strip,
                        &mut state.bar_dirty,
                        comp_ref.id,
                        frame,
                        provenance.scale_q,
                    );
                    _ = stream.add(WorkerResponse::CacheFilled);
                    state.fill_exhausted = false;
                    return;
                }
                continue;
            }
        }
        match prepare_frame(state, &document, comp_ref.id, frame, quality, bgra, true) {
            // Tell the frontend, or the fill is invisible: the cache bar only
            // redraws when it hears something, and a fill shows no frame.
            Ok(_) => _ = stream.add(WorkerResponse::CacheFilled),
            // A comp that will not render must not be retried in a loop.
            Err(_) => state.fill_exhausted = true,
        }
        return;
    }
    // Nothing left for the fill to *make*. Copies may still be on their way up
    // from disk, and the fill does not wait for them by spinning: walking the
    // window again means naming every frame in it again, which is real work to
    // do while the answer is on another thread. What wakes it instead is the
    // copy landing — `collect_disk_loads` clears this the moment one reaches
    // the card, and the fill then finds it held and moves on to the next.
    state.fill_exhausted = true;
}

#[frb(ignore)]
pub enum WorkerRequest {
    RenderComp(RenderCompRequest),
    RenderCompWithPreview(RenderCompRequestWithPreview),
    TraceScope(RenderScopeRequest),
    /// Read the pixels under the dropper (docs/07 §6.1).
    SamplePixels(SamplePixelsRequest),
    /// Start playing. The worker paces itself from here until it is stopped or
    /// runs off the end.
    Play(PlayRequest),
    /// Stop playing. Harmless when nothing is playing.
    StopPlayback,
    /// Set the whole of how the Viewer is looking, in one message: exposure
    /// and tone map (K-314) plus whether the comp's background colour is left
    /// out of the composite while the transparency grid is up (K-352). A
    /// *setting*, not a picture: it changes how every frame from here on is
    /// made and display-encoded and nothing about the document. Preview only —
    /// an export builds its own renderer, which nobody sends this to. One
    /// message rather than one per control, so the renderer can never hold
    /// half a look.
    SetViewerLook {
        stops: f64,
        tone_map: bool,
        transparent_background: bool,
        /// The region of interest as comp fractions `[u0, v0, u1, v1]`
        /// (K-362), or `None` for the whole frame. It rides the look message
        /// rather than getting one of its own for the same reason the
        /// background flag does: the renderer must never hold half a look, and
        /// this is a way of viewing like the rest of it.
        region: Option<[f32; 4]>,
    },
}

/// Start playback of `comp` at `from`.
///
/// **Why the worker plays rather than the frontend driving it.** Playback is a
/// decision made once per frame — which frame is next, is the clock ahead of us,
/// is this mode allowed to skip — and every one of those needs the render cost
/// of the frame just finished. The frontend has none of that. It used to guess:
/// a Flutter `Ticker` polled the audio clock each vsync, worked out a frame, and
/// asked for it, with a hand-rolled in-flight counter to stop the requests
/// piling up. That is a scheduler living on the far side of an FFI boundary from
/// everything it needs to schedule against. The frontend now says "play from
/// here" and paints what arrives (K-181).
#[frb(ignore)]
pub struct PlayRequest {
    pub comp: CompositionReference,
    pub from: u64,
    pub mode: BridgePlaybackMode,
    pub scale: f32,
    /// The document the mix is to be built from, snapshotted where play was
    /// asked for rather than read on this thread — the mix must be of the comp
    /// as it was when the button was pressed. The sound itself is started here,
    /// after the pre-roll ([`Playback::pre_roll_done`]).
    pub audio: std::sync::Arc<lumit_core::Document>,
}

/// Playback in progress: what is being played, and where it has got to.
///
/// The scheduler shape (docs/impl/playback-scheduler.md §5): renders run AHEAD
/// of the clock into `ring`, a bounded queue of finished frames still on the
/// graphics card, and each is PRESENTED — one cheap GPU copy — only when it is
/// due. The slack is the point: a span of cheap or cached frames fills the
/// ring, and an expensive frame then spends the banked time instead of
/// stalling the picture. How far ahead is `capacity()`, adapted from the
/// measured p95 render cost. Dropping this struct (stop, seek, a new play)
/// drops the ring and every in-flight frame with it — the cancellation edge.
// ponytail: renders are still serial on this one worker thread, so cancellation
// latency is bounded by one frame's render, not the impl note's 15 ms. Epoch
// tokens inside the render walk (and the worker pool they exist for) are the
// upgrade, docs/impl/playback-scheduler.md §1-2.
/// How many banked frames count as a full pre-roll, and how long the sound is
/// ever made to wait for them (docs/impl/playback-scheduler.md §5).
const PRE_ROLL_FRAMES: usize = 3;
const PRE_ROLL_BUDGET: std::time::Duration = std::time::Duration::from_millis(150);

#[frb(ignore)]
struct Playback {
    comp: CompositionReference,
    /// The frame to render next.
    next: u64,
    /// The last frame of the composition — playback ends after it.
    last: u64,
    mode: BridgePlaybackMode,
    scale: f32,
    /// The composition's rate, for turning a clock reading into a frame.
    fps: f64,
    /// Where playback started, and when — the wall clock's baseline for as long
    /// as no mix is loaded to be master instead.
    from: u64,
    started: std::time::Instant,
    /// When the last frame was shown — the gap between presents is what the
    /// audio chase judges the picture's rate by. `None` before the first
    /// present of a run.
    last_presented: Option<std::time::Instant>,
    /// When the next every-frame present falls due — a GRID, stepped by one
    /// comp period per present ([`crate::playback::next_present_due`]), so
    /// loop overhead never compounds into a slower rate. `None` before the
    /// first present of a run; adaptive mode paces on the clock instead.
    next_present_due: Option<std::time::Instant>,
    /// Frames rendered ahead of the clock, oldest first, waiting to be shown.
    ring: std::collections::VecDeque<(u64, lumit_render::PreparedFrame)>,
    /// Recent render costs, sizing the ring (`capacity()`).
    costs: crate::playback::CostWindow,
    /// The highest frame whose source decodes have been posted to the
    /// decode-ahead thread this run. A watermark, not a set: playback frames
    /// only move forward, so "post everything from here to there once" is the
    /// whole bookkeeping.
    prefetched_to: Option<u64>,
    /// The mix waiting for the sound to be started, once the picture has
    /// banked enough to start with it (the pre-roll,
    /// [`Self::pre_roll_done`]). `None` once the sound has been started, or
    /// when this run never had any.
    pending_audio: Option<std::sync::Arc<lumit_core::Document>>,
    /// True while the sound is stopped **because the picture is not keeping the
    /// composition's rate**, as against stopped because the user asked.
    /// Every-frame playback stops the track rather than let it run over a
    /// picture that has fallen out of time with it (K-171), and this is what
    /// remembers to start it again — see [`chase_audio`].
    audio_held_for_picture: bool,
    /// How many pictures in a row have gone out at the composition's rate, this
    /// one included. Reset by one late picture. The sound needs
    /// [`AUDIO_REALTIME_FRAMES`] of these before it starts again.
    on_time_run: u32,
    /// How many frames the last [`Self::advance`] had to jump over to catch the
    /// clock. Zero while playback is keeping up.
    ///
    /// **This is the honest measure of "we cannot keep up", and the only one
    /// available.** The worker can time its own render and hand-off, but that is
    /// not the whole bill: decoding the pixels into an image, painting them, and
    /// whatever else the frontend does per frame all happen after the worker has
    /// let go, and it can never see them. Skipping is the *symptom* of all of it
    /// at once — if the clock has moved past a frame we have not drawn yet, the
    /// round trip cost more than its budget, wherever the time went.
    skipped: u64,
    /// What recent presents cost — the copy plus the waits inside
    /// [`present_ring_frame`] — beside `costs`' renders. The pace report's
    /// numbers (see [`Self::report_pace`]).
    present_costs: crate::playback::CostWindow,
    /// When the pace was last printed, so the report stays sparse.
    pace_reported: std::time::Instant,
}

impl Playback {
    /// Print where a playback frame's time is going, at most every couple of
    /// seconds — the stopwatch docs/TODO.md's "measure first" asks for before
    /// the shared-texture present is rebuilt around GPU-side sync. The present
    /// number is the one that decides it: it is the copy PLUS the full-queue
    /// wait (`Maintain::Wait`) plus the D3D11 hop, so a present p95 of a
    /// fraction of a millisecond says the rebuild would buy nothing, and one
    /// of several says it is the next job. One line per interval is far below
    /// the console traffic the worker already allows itself for errors.
    fn report_pace(&mut self) {
        const PACE_REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
        if self.pace_reported.elapsed() < PACE_REPORT_EVERY {
            return;
        }
        self.pace_reported = std::time::Instant::now();
        let ms = |cost: Option<f64>| cost.map_or(0.0, |secs| secs * 1000.0);
        println!(
            "Playback pace: present p95 {:.2} ms, render p95 {:.2} ms, ring {}, budget {:.2} ms",
            ms(self.present_costs.p95()),
            ms(self.costs.p95()),
            self.ring.len(),
            1000.0 / self.fps.max(1.0),
        );
    }

    /// Where playback has actually got to, in seconds.
    ///
    /// The audio clock is master once a mix is loaded; until then — while it is
    /// still decoding, or on a machine with no sound device — the wall clock
    /// stands in, so silence never stops the picture.
    fn elapsed_seconds(&self) -> f64 {
        match clock_seconds() {
            Some(seconds) => seconds,
            None => self.started.elapsed().as_secs_f64() + self.from as f64 / self.fps,
        }
    }

    /// How many frames ahead of the clock to render — the ring's capacity,
    /// adapted from the measured p95 render cost (the impl note's pinned
    /// formula, [`crate::playback::lookahead_frames`]).
    fn capacity(&self) -> usize {
        crate::playback::lookahead_frames(self.costs.p95(), self.fps)
    }

    /// Whether the sound may start: the ring holds a few frames, or the
    /// pre-roll budget is spent.
    ///
    /// **Why the sound waits at all.** Starting the audio stream at the moment
    /// play is pressed means it runs while the first frame is still being
    /// composited — the sound is already a few tens of milliseconds in before
    /// there is anything to see, and in adaptive mode the picture then *skips*
    /// to catch the clock up, so a press of play began with a jump. Filling the
    /// ring first (docs/impl/playback-scheduler.md §5) starts the two together.
    ///
    /// The budget is the other half: a comp too heavy to bank three frames
    /// quickly must not sit in silence waiting: at 150 ms the sound starts
    /// regardless and the picture does what it can.
    fn pre_roll_done(&self, queued: usize) -> bool {
        queued >= PRE_ROLL_FRAMES.min(self.capacity()).max(1)
            || self.started.elapsed() >= PRE_ROLL_BUDGET
    }

    /// Which queued frame to present now — an index into `queued` (the ring's
    /// frame numbers, oldest first) — or `None` while nothing is due yet.
    ///
    /// **This is what keeps playback at the composition's rate.** Renders are
    /// free to run ahead into the ring; the PRESENT is what the user sees, so
    /// the present is what paces. Without this gate a comp cheaper than
    /// realtime would play as fast as the renderer managed — the frontend's
    /// `Ticker` used to supply the pacing for free by only asking once per
    /// vsync, and losing it made a 60 fps comp play at several hundred.
    ///
    /// * **Every-frame** shows every frame in order (the mode's promise), so it
    ///   is always the front — but no sooner than its due time on the present
    ///   grid ([`crate::playback::next_present_due`]). It may fall behind (a
    ///   heavy comp plays slow); it is never allowed to run ahead, however
    ///   full the cache fills the ring (K-171: "replays at full speed" means
    ///   the comp's own rate). The grid, not a stopwatch from the last actual
    ///   present: a stopwatch added every scrap of loop lateness to every
    ///   frame, so a 60 fps comp could never actually play at 60.
    /// * **Adaptive** keeps time: the NEWEST queued frame the clock has
    ///   reached (docs/impl/playback-scheduler.md §4). The caller drops the
    ///   older entries — the clock has passed them, and showing them would
    ///   mean playing late pictures instead of the current one.
    fn present_choice(&self, queued: &[u64]) -> Option<usize> {
        if queued.is_empty() {
            return None;
        }
        match self.mode {
            BridgePlaybackMode::EveryFrame => match self.next_present_due {
                Some(due) if std::time::Instant::now() < due => None,
                _ => Some(0),
            },
            BridgePlaybackMode::Adaptive => {
                let clock = self.elapsed_seconds();
                queued
                    .iter()
                    .rposition(|&frame| frame as f64 / self.fps <= clock)
            }
        }
    }

    /// How long until the ring's front is due to present, or `None` when it is
    /// due now (or nothing is queued). The worker sleeps this out — in short
    /// slices, so a stop arriving mid-wait is still acted on promptly — when
    /// the ring is full and there is nothing else to do.
    fn wait_until_present(&self, queued: &[u64]) -> Option<std::time::Duration> {
        let &front = queued.first()?;
        match self.mode {
            BridgePlaybackMode::EveryFrame => {
                let due = self.next_present_due?;
                let now = std::time::Instant::now();
                (due > now).then(|| due - now)
            }
            BridgePlaybackMode::Adaptive => {
                let due = front as f64 / self.fps;
                let clock = self.elapsed_seconds();
                (due > clock).then(|| std::time::Duration::from_secs_f64(due - clock))
            }
        }
    }

    /// The next frame to render, or `None` when playback has run off the end.
    ///
    /// The mode difference, and the policy that used to live in Dart:
    ///
    /// * **Every-frame** never skips — that is the mode's entire promise, since
    ///   the point of it is to render and cache every frame at full quality
    ///   however long that takes (K-171). It simply counts.
    /// * **Adaptive** keeps time, so it never schedules a frame the clock has
    ///   already passed — it jumps to where playback actually is. Running
    ///   *ahead* of the clock is fine now (that is what the ring is for);
    ///   how far ahead is [`Self::capacity`]'s business, not this one's.
    fn advance(&mut self) -> Option<u64> {
        if self.next > self.last {
            return None;
        }
        let frame = match self.mode {
            BridgePlaybackMode::EveryFrame => self.next,
            BridgePlaybackMode::Adaptive => {
                let wanted = (self.elapsed_seconds() * self.fps).floor().max(0.0) as u64;
                // Never go backwards. A clock reading behind the frame just
                // drawn — a resync, or a mix loading part-way through — would
                // otherwise play a short stretch twice.
                wanted.max(self.next)
            }
        };
        self.skipped = frame.saturating_sub(self.next);
        if frame > self.last {
            self.next = frame;
            return None;
        }
        self.next = frame + 1;
        Some(frame)
    }

    /// What the last frame really cost, for the realtime controller.
    ///
    /// `busy` is what the worker itself measured — render plus hand-off. When
    /// playback is keeping up that is the honest number and lets the tier climb
    /// back. When frames are being skipped it is an *under*-estimate by
    /// definition: the skip proves the round trip took longer than its budget,
    /// and the part the worker cannot see is exactly the part that made it so.
    /// One skipped frame means the last one took about two budgets, two means
    /// about three, and so on — which is the cost to report if the tier is ever
    /// to come down over work the worker is blind to.
    fn observed_cost(&self, busy: f64) -> f64 {
        let budget = 1.0 / self.fps;
        if self.skipped == 0 {
            busy
        } else {
            (self.skipped + 1) as f64 * budget
        }
    }
}

/// Start the sound for `comp` at `start` seconds, from the snapshot playback
/// captured. A build with no media support has nothing to start.
#[frb(ignore)]
fn start_audio(comp: Uuid, start: f64, document: Option<std::sync::Arc<lumit_core::Document>>) {
    let Some(document) = document else {
        return;
    };
    #[cfg(feature = "media")]
    crate::audio::play(comp, start, document);
    #[cfg(not(feature = "media"))]
    let _ = (comp, start, document);
}

/// Where the sound has got to, in seconds, or `None` when there is no mix to
/// follow. The audio module's own clock — read here rather than in Dart so the
/// frame it implies is chosen next to the renderer that has to make it.
#[frb(ignore)]
fn clock_seconds() -> Option<f64> {
    #[cfg(feature = "media")]
    {
        let (seconds, playing, loaded) = crate::audio::clock();
        (loaded && playing).then_some(seconds)
    }
    #[cfg(not(feature = "media"))]
    None
}

/// Stop the sound the moment the picture is not running at the composition's
/// rate, and start it again — level with the picture — once the picture has held
/// that rate for a while. Every-frame playback's half of A/V sync (K-171).
///
/// # What is measured, and why it is not the clock
///
/// The gap between one picture going out and the next **is** the rate the user
/// is watching. Nothing says as directly whether the sound can run beside it,
/// and two earlier rules that measured something else both failed:
///
/// * *How far the sound is in front of the picture.* It leaves the sound running
///   for half a second over a picture that has already stopped keeping time.
/// * *How many finished frames are waiting.* Frames are usually waiting at the
///   moment a picture goes out, even when the run as a whole is far slower than
///   the composition's rate. The sound therefore started again on the very next
///   picture, stopped, started, and to the ear never stopped at all — it
///   stuttered instead.
///
/// # The two answers are unalike on purpose
///
/// One late picture stops the sound at once, because sound over a picture that
/// is not keeping time is the fault being repaired. Starting it again takes
/// [`AUDIO_REALTIME_FRAMES`] pictures in a row on time, because one picture that
/// happens to land on time says nothing about the next. Stop on the evidence of
/// one; start on the evidence of many.
///
/// When it does start it starts **at the picture**: the sound is moved to the
/// frame on screen first. The sound stops in front of the picture, thus starting
/// it where it stopped would play a moment the picture has not reached — and
/// after a long stall that moment can be past the end of the composition.
#[frb(ignore)]
fn chase_audio(playback: &mut Playback, frame: u64, since_present: Option<std::time::Duration>) {
    if held_clock_seconds().is_none() {
        // No mix at all: nothing to stop and nothing to start.
        playback.audio_held_for_picture = false;
        playback.on_time_run = 0;
        return;
    }
    let fps = playback.fps.max(1.0);
    let period = std::time::Duration::from_secs_f64(1.0 / fps);
    // The first picture of a run has nothing to be measured against, thus it
    // counts as on time rather than as evidence of a slow one.
    let on_time = since_present.is_none_or(|gap| gap <= on_time_limit(period));
    playback.on_time_run = if on_time {
        playback.on_time_run.saturating_add(1)
    } else {
        0
    };
    match audio_chase(
        playback.audio_held_for_picture,
        on_time,
        playback.on_time_run,
    ) {
        AudioChase::Hold => {
            playback.audio_held_for_picture = true;
            crate::api::audio::audio_pause();
        }
        AudioChase::Start => {
            playback.audio_held_for_picture = false;
            crate::api::audio::audio_seek(frame as f64 / fps);
            resume_audio();
        }
        AudioChase::Leave => {}
    }
}

/// What to do with the sound this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[frb(ignore)]
enum AudioChase {
    /// Stop it: the picture is not keeping the composition's rate.
    Hold,
    /// Start it, at the frame on screen: the picture has held that rate.
    Start,
    /// Neither.
    Leave,
}

/// The rule [`chase_audio`] applies, with no sound device in it so the rule can
/// be tested on its own.
///
/// `on_time` is whether this picture came at the composition's rate. `run` is
/// how many have come at that rate without a break, this one included.
#[frb(ignore)]
fn audio_chase(held: bool, on_time: bool, run: u32) -> AudioChase {
    if !on_time {
        // One late picture is enough to stop it.
        return if held {
            AudioChase::Leave
        } else {
            AudioChase::Hold
        };
    }
    if held && run >= AUDIO_REALTIME_FRAMES {
        AudioChase::Start
    } else {
        AudioChase::Leave
    }
}

/// How much later than the composition's frame period a picture may arrive and
/// still count as on time.
///
/// A quarter again is past the unevenness of the worker loop and well short of a
/// rate anybody would call correct: at 24 fps it makes 52 ms the limit, which is
/// 19 pictures a second. Slower than that and the sound stops.
const LATE_ENOUGH_TO_STOP: f64 = 1.25;

/// The floor under the quarter-period allowance, and why proportional alone is
/// wrong: at 120 fps a quarter of the period is two milliseconds, which is
/// inside the jitter of an ordinary scheduler wake — the sound stopped over
/// pictures that were holding the rate to the eye. The ear judges A/V slip in
/// milliseconds, not in frames, so the allowance never shrinks below a fixed
/// few of them however fast the composition runs.
const MIN_AUDIO_SLACK: std::time::Duration = std::time::Duration::from_millis(5);

/// The gap between two pictures beyond which the later one counts late:
/// the comp period plus a quarter of it, floored at [`MIN_AUDIO_SLACK`].
#[frb(ignore)]
fn on_time_limit(period: std::time::Duration) -> std::time::Duration {
    period
        + period
            .mul_f64(LATE_ENOUGH_TO_STOP - 1.0)
            .max(MIN_AUDIO_SLACK)
}

/// How many pictures in a row must arrive on time before the sound starts again.
///
/// Eight is a third of a second at 24 fps: enough that one picture landing on
/// time by chance does not start the sound, short enough that a run which has
/// genuinely recovered is not left silent.
const AUDIO_REALTIME_FRAMES: u32 = 8;

/// Where the sound has got to **whether or not it is running** — the number
/// [`clock_seconds`] hides once the sound stops.
///
/// Every-frame playback stops the sound when the picture falls behind, and the
/// clock then holds its position. Deciding when the picture has caught up needs
/// exactly that held position, thus it cannot be read through a function that
/// answers `None` for a stopped clock. `None` here means there is no mix at all.
#[frb(ignore)]
fn held_clock_seconds() -> Option<f64> {
    #[cfg(feature = "media")]
    {
        let (seconds, _playing, loaded) = crate::audio::clock();
        loaded.then_some(seconds)
    }
    #[cfg(not(feature = "media"))]
    None
}

/// Start the sound again where it stopped (see [`crate::audio::resume`]).
#[frb(ignore)]
fn resume_audio() {
    #[cfg(feature = "media")]
    crate::audio::resume();
}

pub struct RenderCompRequest {
    pub comp: CompositionReference,
    pub frame: u64,
    /// Which of the two playback behaviours this render is for.
    pub mode: BridgePlaybackMode,
    /// The on-screen scale of the Viewer, 1.0 meaning "shown at comp
    /// resolution". Below 1.0 the frame is being displayed smaller than the comp,
    /// so it is decoded smaller too — see [`crate::render::quality_for`].
    pub scale: f32,
}

/// A render of one frame with part of `layer` substituted — the live-drag path.
///
/// Both overrides are optional and independent, so the one request shape serves
/// an effect drag and a transform drag rather than each growing its own worker
/// message. `None` means "leave that part of the layer as the document has it".
/// A scope trace of one frame — the Scopes panel's request.
///
/// It renders the comp to CPU pixels and bins them on the GPU, whichever
/// publish path the Viewer is on: the zero-copy paths never read pixels back, so
/// the trace cannot borrow the Viewer's frame and asks for its own. That is why
/// the panel throttles rather than tracing every frame.
#[frb(ignore)]
pub struct RenderScopeRequest {
    pub comp: CompositionReference,
    pub frame: u64,
    pub scale: f32,
    /// Which trace: the codes `lumit_render` reads — 0 waveform, 1 parade,
    /// 2 vectorscope, 3 histogram.
    pub kind: u32,
    /// Background, trace, then the R, G and B channel tints, each `[r, g, b]`.
    pub colours: [[u8; 3]; 5],
}

/// One read of the pixels under the dropper — the magnifier's request.
///
/// **Why the worker answers it and not a plain synchronous call.** The pixels
/// only exist where the renderer does, and the renderer is owned outright by
/// this thread (no lock, by design). A sync call would either have to render on
/// Dart's UI isolate or reach across a lock at the one place docs/14 forbids
/// one. So the dropper asks the way the Scopes panel asks, and paints what comes
/// back.
#[frb(ignore)]
pub struct SamplePixelsRequest {
    pub comp: CompositionReference,
    pub frame: u64,
    pub scale: f32,
    /// Where to read, as a fraction of the picture: `(0, 0)` its top-left,
    /// `(1, 1)` its bottom-right. **Not a pixel** — see [`sample_pixels`] for
    /// why the caller cannot name one.
    pub u: f64,
    pub v: f64,
    /// The window's side length in pixels, forced odd and capped at
    /// [`MAX_WINDOW`]. Bigger than the magnifier's own grid on purpose: the
    /// frontend follows the pointer inside what it already has, and asks again
    /// only when the pointer nears the edge of it.
    pub window: u32,
    /// Read this layer *alone* instead of the composite — what a depth pick
    /// does, so a hidden depth pass (which never shows in the composite) can
    /// still be read. `None` samples the composite.
    pub layer: Option<LayerReference>,
}

/// The largest window one read may carry: 129×129 pixels, 66 KiB.
///
/// Chosen to be worth a read — a pointer can travel sixty pixels in any
/// direction before the frontend needs another one — while staying far below
/// the size at which a pixel payload stops being a reading and becomes a frame
/// transport (a 1080p frame is 8 MiB, and 8.8 ms in the codec: K-183).
#[frb(ignore)]
pub const MAX_WINDOW: u32 = 129;

#[frb(ignore)]
pub struct RenderCompRequestWithPreview {
    pub comp: CompositionReference,
    pub frame: u64,
    pub scale: f32,
    pub layer: LayerReference,
    pub effects: Option<Vec<EffectInstance>>,
    pub transform: Option<crate::api::layer::BridgeTransform>,
    /// A text layer's document, while it is being typed (K-225). The Type tool
    /// writes the layer once, when the edit ends; this is what keeps the
    /// picture in step in the meantime without an undo step per keystroke.
    pub text: Option<crate::api::assets::BridgeTextDocument>,
    /// A layer's whole paint list, while one of its strokes is being dragged
    /// (K-239). The same reason as `text` above: a stroke's opacity is one op
    /// per drag, not one per tick, so the picture is kept in step by previewing
    /// rather than by writing.
    pub paint: Option<Vec<crate::api::layer::BridgeStroke>>,
    /// One clip's retime map, while its envelope point is being dragged
    /// (K-247). The same reason as `text` and `paint`: a re-speed is one op
    /// per drag, not one per tick, and a retime decides *which frame* is
    /// decoded — so without this the picture simply does not move until the
    /// pointer is let go, which is the one edit where watching it matters
    /// most.
    pub clip_retime: Option<(Uuid, crate::api::effect::BridgeScalar)>,
    /// The layer's own Retime map (K-197), while a key of it is being dragged
    /// in the graph editor. Exactly `clip_retime`'s reason, for the property
    /// rather than a clip: the map decides which source frame is decoded, so
    /// the drag cannot ride the retained pixels and the provisional map has to
    /// reach the render plan.
    pub retime: Option<crate::api::effect::BridgeScalar>,
    /// A shape layer's whole art list, while one of its items is being dragged
    /// (K-239). The same reason as `paint` above.
    pub contents: Option<Vec<crate::api::layer::BridgeShapeItem>>,
    /// A layer's whole mask list, while one of them is being dragged (K-240).
    /// The same reason as `paint` and `contents` above.
    pub masks: Option<Vec<crate::api::layer::BridgeMask>>,
}

/// Ask the operating system for 1 ms sleep granularity (Windows; a no-op
/// elsewhere, where sleeps are already fine-grained).
///
/// The worker paces presents with short sleeps, and Windows rounds a sleep UP
/// to the system timer's next tick — by default ~15.6 ms apart. That is twice
/// a 120 fps frame period: every sleep-then-present overshot its due time, the
/// present grid re-anchored, and the achieved rate fell to whatever the timer
/// allowed (~85 of 120 in practice; at 60 fps the grid held but presents
/// jittered by several milliseconds, which is what kept stopping the sound).
/// One millisecond is the granularity every media application requests; the
/// setting is process-wide and lives for the process, exactly as it does in
/// any NLE. A refusal is harmless — pacing then leans on the spin window
/// alone.
#[cfg(windows)]
#[frb(ignore)]
fn raise_timer_resolution() {
    // SAFETY: `timeBeginPeriod` touches no memory owned by this program; it
    // adjusts a scheduler setting and reports acceptance or refusal in its
    // return value, which is ignored because a refusal leaves nothing to do.
    unsafe {
        let _ = windows::Win32::Media::timeBeginPeriod(1);
    }
}

#[cfg(not(windows))]
#[frb(ignore)]
fn raise_timer_resolution() {}

#[frb(ignore)]
pub fn run_worker(project: ProjectReference, stream: WorkerResponseStream) {
    let (send_to_worker, receive_from_app) = std::sync::mpsc::channel::<WorkerRequest>();

    {
        let Ok(state) = project.state() else {
            eprintln!("No such project; not starting the render worker");
            return;
        };
        let Ok(mut state) = state.write() else {
            eprintln!("Project state poisoned; not starting the render worker");
            return;
        };

        state.sender = Some(send_to_worker);
    }

    std::thread::spawn(move || worker_loop(project, receive_from_app, stream));
}

#[frb(ignore)]
fn worker_loop(
    project: ProjectReference,
    receiver: Receiver<WorkerRequest>,
    stream: WorkerResponseStream,
) {
    println!("Worker thread started");
    let mut stream = stream;
    raise_timer_resolution();

    // **One renderer is built at a time, and none at all for a project that
    // has already gone** (K-434). Building one means a GPU device and every
    // pipeline the compositor needs — seconds of driver work where there is no
    // warm shader cache — and it cannot be interrupted once started. The
    // editor never notices this lock, because it has one project open and so
    // one worker; a process that opens projects faster than they build does,
    // and it is the reason for both halves:
    //
    // * Serialised, the peak is one device under construction rather than one
    //   per project opened in the meantime. Twenty at once is not faster than
    //   twenty in turn — the driver serialises them anyway — and it exhausted
    //   the card, at which point every later device request failed and the
    //   Viewer went blank for projects that were perfectly healthy.
    // * Once the lock is ours, the project may have been closed while we
    //   waited. `state()` answers `InvalidProject` the moment it is, and a
    //   renderer for a project nobody can ask anything of is a device built to
    //   be dropped — so the queue drains at once instead of building each one.
    //
    // The frb test suite is where this shows: it makes a project per test and
    // draws in most of them, so a file of ninety tests asked for ninety
    // devices inside a few seconds. Poisoning is nothing to fail over: the
    // lock guards no data, only the turn-taking.
    let building = BUILDING_RENDERER.lock().unwrap_or_else(|e| e.into_inner());
    if !worth_building_for(&project) {
        // Quietly: a project closing while its worker waited its turn is the
        // ordinary end of a session, not a fault.
        return;
    }
    // No renderer means no Viewer, but the editor itself stays usable — the
    // worker just stops instead of taking the process down with it.
    let mut renderer = match HeadlessRenderer::new() {
        Ok(renderer) => renderer,
        Err(err) => {
            eprintln!("Could not create the renderer, stopping the worker: {err}");
            return;
        }
    };
    drop(building);
    // This is the *Viewer's* renderer, so a Lens flare's bake is made beside
    // the frame rather than inside it (K-350): picking a lens shows the lens
    // before it and swaps the new one in when the optics are done, instead of
    // stopping the picture for about half a second. The exporter builds its
    // own renderer and never asks for this, so an export still bakes exactly
    // and is bit-for-bit what it was.
    renderer.set_deferred_flare_bakes(true);

    // The two profiler sinks (docs/13 §7.1), installed for the session and fed
    // from inside a render — which is why they take a *clone* of the reply
    // stream rather than borrowing the one the loop below writes through: a
    // report is raised while the render still holds the thread, long before
    // control returns to a place that could hand it the borrow.
    //
    // Which frames actually use them is decided per request (`watch_frames` /
    // `measure_frames`): a scrub describes itself, a playing frame does not.
    {
        let progress_stream = stream.clone();
        renderer.set_progress_sink(Some(std::sync::Arc::new(
            move |p: lumit_render::FrameProgress| {
                _ = progress_stream.add(WorkerResponse::RenderProgress(
                    crate::api::state::BridgeRenderProgress {
                        frame: p.frame,
                        stage: p.stage.code(),
                        fraction: f64::from(p.fraction),
                        // The engine never sends the last word: a frame that faults
                        // would then leave a bar standing for ever. The worker ends
                        // every bar it started, below.
                        done: false,
                    },
                ));
            },
        )));
        let profile_stream = stream.clone();
        renderer.set_profile_sink(Some(std::sync::Arc::new(
            move |p: lumit_render::FrameProfile| {
                // One line per switching on, never per frame — see
                // `profiling::announce_first`.
                crate::profiling::announce_first(p.frame, p.layers.len(), p.total_ms);
                _ = profile_stream.add(WorkerResponse::FrameProfile(profile_of(&p)));
            },
        )));
    }

    let mut state = WorkerState {
        project,
        renderer,
        preview_engine: PreviewEngine::default(),
        playback: None,
        prefetcher: crate::prefetch::Prefetcher::default(),
        last_shown: None,
        disk: lumit_render::diskio::spawn(),
        disk_wanted: std::collections::HashMap::new(),
        names: crate::names::NameCache::default(),
        // Zero and "never opened", so the first sync applies whatever the
        // settings hold and opens the folder for the project that is loaded —
        // see the note on `applied_vram_budget` below.
        applied_disk_budget: 0,
        seen_disk_clears: crate::framecache::disk::clears(),
        seen_disk_location: (u64::MAX, None),
        // NOT the wish's current value. A fresh renderer's cache holds the
        // built-in default, and the settings' value is usually already in that
        // atomic by the time a worker starts — restored at launch, or left there
        // by the previous project. Seeding this from it therefore claimed the
        // budget was applied when it never had been, and the cache stayed at
        // its 512 MiB default for the whole session while Settings read 8 GB.
        // Zero means "nothing applied yet", so the first sync applies whatever
        // the wish is.
        applied_vram_budget: 0,
        seen_vram_clears: crate::framecache::vram::clears(),
        published_vram: (0, 0),
        published_bar: None,
        bar_strip: Vec::new(),
        bar_refined_to: 0,
        bar_dirty: false,
        bar_published_at: std::time::Instant::now() - BAR_MIN_INTERVAL,
        fill_exhausted: true,
        backup_exhausted: true,
        last_request: std::time::Instant::now(),
        bakes_seen: 0,
        layer_sample: None,
        pending_measure: None,
    };

    loop {
        sync_caches(&mut state, &mut stream);

        // While playing the worker has work of its own, so it must not block on
        // the channel — it takes whatever has arrived and gets on with the next
        // frame. Idle, it waits — indefinitely in spirit, but waking after a
        // 200 ms lull to fill the cache around the playhead (docs/06 §5.5),
        // then on a short leash while that filling is productive so it
        // proceeds briskly yet yields to any request within one frame's
        // render. With nothing left to fill the wake does no work at all, so
        // an editor sitting still spins no core worth speaking of.
        let request = if state.playback.is_some() {
            match receiver.try_recv() {
                Ok(request) => Some(request),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Receiver disconnected, stopping the worker");
                    return;
                }
            }
        } else {
            // Idle work of any kind means come back soon; with none left, wait
            // long enough that an idle editor costs nothing.
            let wait = if state.fill_exhausted && state.backup_exhausted {
                std::time::Duration::from_millis(200)
            } else {
                std::time::Duration::from_millis(2)
            };
            match receiver.recv_timeout(wait) {
                Ok(request) => Some(request),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // A lens finished baking: the picture on screen was drawn
                    // with the lens before it, so make it again (K-350). This
                    // is what turns the old half-second freeze into a wait —
                    // the frame the user is looking at keeps its old flare and
                    // is replaced the moment the new optics are ready.
                    republish_after_bake(&mut state, &mut stream);
                    let lull =
                        state.last_request.elapsed() >= std::time::Duration::from_millis(200);
                    if lull {
                        // The numbers for the frame on screen, if a tier served
                        // it unmeasured (K-420) — before the fill, so the column
                        // fills one idle turn after the picture.
                        measure_pending(&mut state);
                        if !state.fill_exhausted {
                            idle_fill(&mut state, &mut stream);
                        }
                        // Alongside the fill, not after it. On a long
                        // composition the fill has frames to make for as long as
                        // the budget lasts, thus "when the fill is finished"
                        // would mean "never" — and never is how long the disk
                        // tier stayed empty.
                        if !state.backup_exhausted {
                            idle_backup(&mut state);
                        }
                    }
                    None
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    eprintln!("Receiver disconnected, stopping the worker");
                    return;
                }
            }
        };

        if let Some(request) = request {
            state.last_request = std::time::Instant::now();
            // No second sync before serving. There used to be one, and it
            // mattered: a commit landing while the worker was parked in `recv`
            // retired every held frame, and the sync at the top of the turn had
            // already run — so the request the commit provoked was answered from
            // the caches that commit had just invalidated, and the Viewer kept
            // the pre-edit picture until something else moved the playhead. With
            // content-hash names there is no invalidation to be on the wrong side
            // of: the edited document asks for different names and misses.
            handle_requests(request, &receiver, &mut state, &mut stream);
        }

        play_one_frame(&mut state, &mut stream);
    }
}

/// One turn of the playback scheduler, if playback is running
/// (docs/impl/playback-scheduler.md §5).
///
/// Each turn does at most one piece of work — present a due frame, or render
/// one frame ahead into the ring, or sleep a short bounded slice — so a stop
/// or a seek arriving mid-playback is seen between pieces rather than after
/// the whole run. Renders and presents are decoupled: renders fill the ring as
/// fast as the machine allows (up to `capacity()` frames ahead), presents pace
/// against the clock, and the ring between them is the slack that lets one
/// expensive frame spend what the cheap frames before it banked.
#[frb(ignore)]
fn play_one_frame(state: &mut WorkerState, stream: &mut WorkerResponseStream) {
    // File whatever the decode-ahead thread has finished into the renderer's
    // cache, so the renders below find their source pixels already decoded.
    for done in state.prefetcher.drain() {
        state.renderer.preload_decoded(
            done.item,
            done.frame,
            done.target_width,
            done.width,
            done.height,
            done.rgba,
        );
    }

    // The pre-roll: the sound starts once the picture has something banked to
    // start alongside it (or the budget is spent), not at the press of play.
    if let Some(playback) = &mut state.playback {
        if playback.pending_audio.is_some() && playback.pre_roll_done(playback.ring.len()) {
            let document = playback.pending_audio.take();
            let start = playback.from as f64 / playback.fps;
            // The clock's baseline is now, not when the request arrived: the
            // pre-roll's own milliseconds are not playback time, and counting
            // them would have adaptive skip straight over the frames just
            // banked.
            playback.started = std::time::Instant::now();
            start_audio(playback.comp.id, start, document);
        }
    }

    // Present first: at a frame boundary the due picture goes out BEFORE the
    // next render is started, so an expensive render never delays a present
    // that was already payable.
    if let Some(playback) = &mut state.playback {
        let queued: Vec<u64> = playback.ring.iter().map(|(frame, _)| *frame).collect();
        if let Some(chosen) = playback.present_choice(&queued) {
            // Everything before the chosen entry arrived too late — adaptive's
            // clock has passed it (every-frame always chooses the front, so
            // this drops nothing there). Rendered but never shown; the frame
            // cache keeps the work.
            let Some((frame, prepared)) = playback.ring.drain(..=chosen).last() else {
                return;
            };
            // How long since the last picture went out, measured before this
            // one is stamped: the gap IS the frame rate the user is watching,
            // and it is what says whether the sound can run alongside.
            let now = std::time::Instant::now();
            let since_present = playback.last_presented.map(|at| now - at);
            playback.last_presented = Some(now);
            if matches!(playback.mode, BridgePlaybackMode::EveryFrame) {
                // Step the present grid: the next frame is due one comp period
                // after this one was SCHEDULED, not after it went out, so the
                // rate holds however untidy each individual present is.
                let period = std::time::Duration::from_secs_f64(1.0 / playback.fps.max(1.0));
                playback.next_present_due = Some(crate::playback::next_present_due(
                    playback.next_present_due,
                    now,
                    period,
                ));
            }
            // Playback moves the playhead: keep the idle fill's anchor with
            // it, so a stop resumes filling from where the user actually is.
            state.last_shown = Some((playback.comp.clone(), frame, playback.scale));
            state.fill_exhausted = false;
            if matches!(playback.mode, BridgePlaybackMode::EveryFrame) {
                chase_audio(playback, frame, since_present);
            }
            let present_started = std::time::Instant::now();
            present_ring_frame(&mut state.renderer, frame, &prepared, stream);
            // What the hand-off cost, and the sparse pace line it feeds — the
            // number that says whether the present's full-queue wait is worth
            // rebuilding (docs/TODO.md, "measure first").
            let Some(playback) = &mut state.playback else {
                return;
            };
            playback
                .present_costs
                .push(present_started.elapsed().as_secs_f64());
            playback.report_pace();
            return;
        }
    }

    let Some(playback) = &mut state.playback else {
        return;
    };

    // Render ahead while the ring has room and frames remain.
    if playback.ring.len() < playback.capacity() {
        if playback.next <= playback.last {
            let (document, revision) = {
                let Ok(document) = state.project.state() else {
                    return;
                };
                let Ok(document) = document.read() else {
                    return;
                };
                (document.store.snapshot(), document.store.revision())
            };
            // The adaptive tier applies at RENDER time — the whole point of a
            // coarser tier is a cheaper composite (K-186), so it must be in
            // force while the frame is made, not when it is shown. Read before
            // the render so the cost can be attributed to it afterwards.
            let tier = crate::realtime::tier();
            let quality = playback_quality(playback.scale, playback.mode, tier);
            let comp_id = playback.comp.id;
            // BGRA on the Windows shared-texture path (ANGLE only opens BGRA
            // surfaces); RGBA everywhere else.
            let bgra = zero_copy_wants_bgra();
            // Every name asked for below — the disk grace, the look-ahead —
            // is of this one snapshot: probe it once, so the memo's misses
            // are hashes and nothing else (see `frame_name`).
            state.renderer.presync_items(&document, comp_id);
            // Every-frame only: when the NEXT frame's bytes are on their way
            // up from disk, hold the composite a bounded moment — the copy is
            // far cheaper than making the frame again, and every-frame
            // promises every frame, not any particular arrival time
            // ([`crate::playback::wait_for_disk`]). Adaptive keeps chasing
            // its clock instead.
            if matches!(playback.mode, BridgePlaybackMode::EveryFrame) {
                let peek = playback.next;
                let name =
                    state
                        .names
                        .get_or_compute(revision, comp_id, peek, quality.tag(), || {
                            state
                                .renderer
                                .frame_key_presynced(&document, comp_id, peek, quality)
                        });
                if let Some(key) = name {
                    if !state.renderer.has_frame_texture(key, bgra)
                        && !crate::framecache::contains(key)
                    {
                        let asked_ago =
                            state.disk_wanted.get(&key).map(|want| want.asked.elapsed());
                        if crate::playback::wait_for_disk(asked_ago) {
                            // A sliver of sleep, not a spin: the copy is
                            // collected at the top of the next turn, and a
                            // stop arriving mid-wait is still seen promptly.
                            std::thread::sleep(std::time::Duration::from_micros(500));
                            return;
                        }
                        if asked_ago.is_some() {
                            // The read never came (the file has gone from
                            // under the session): composite below, and stop
                            // counting the ask as pending.
                            state.disk_wanted.remove(&key);
                        }
                    }
                }
            }
            if let Some(frame) = playback.advance() {
                // Post the COMING frames' source decodes to the decode-ahead
                // thread before this frame's render occupies the loop, so those
                // decodes and this composite run at the same time. The watermark
                // posts each frame once per run; an adaptive skip jumps it
                // forward with the playhead.
                let ahead_to = frame
                    .saturating_add(crate::playback::PREFETCH_AHEAD)
                    .min(playback.last);
                let from = playback
                    .prefetched_to
                    .map_or(frame + 1, |posted| posted + 1)
                    .max(frame + 1);
                for future in from..=ahead_to {
                    let wants = state
                        .renderer
                        .prefetch_wants(&document, comp_id, future, quality);
                    for want in wants {
                        state.prefetcher.request(want);
                    }
                    // And climb the tiers for the coming frames at the same
                    // time: a frame held in memory goes up to the card now, a
                    // parked one is asked for now — a read off disk takes a
                    // turn or two of the loop, thus a frame asked for when it
                    // is shown always comes too late and is composited again.
                    let name = state.names.get_or_compute(
                        revision,
                        comp_id,
                        future,
                        quality.tag(),
                        || {
                            state
                                .renderer
                                .frame_key_presynced(&document, comp_id, future, quality)
                        },
                    );
                    if let Some(key) = name {
                        let provenance = lumit_render::FrameProvenance {
                            comp: comp_id,
                            frame: future,
                            scale_q: lumit_render::preview_scale_q(quality),
                            quality,
                        };
                        let uploaded = line_up_frame(
                            &mut state.renderer,
                            &state.disk,
                            &mut state.disk_wanted,
                            key,
                            provenance,
                        );
                        if uploaded {
                            mark_banked(
                                state.published_bar,
                                &mut state.bar_strip,
                                &mut state.bar_dirty,
                                comp_id,
                                future,
                                provenance.scale_q,
                            );
                        }
                    }
                }
                if ahead_to >= from {
                    playback.prefetched_to = Some(ahead_to);
                }
                let started = std::time::Instant::now();
                let rendered = prepare_frame(
                    state, &document, comp_id, frame, quality, bgra,
                    // Committed document: a warm span plays from the VRAM cache
                    // and every rendered frame warms it for the next pass.
                    true,
                );
                let cost = started.elapsed().as_secs_f64();
                // `prepare_frame` borrowed the whole worker, so the playback state
                // has to be picked up again to file the result.
                let Some(playback) = &mut state.playback else {
                    return;
                };
                match rendered {
                    Ok(prepared) => {
                        playback.ring.push_back((frame, prepared));
                        playback.costs.push(cost);
                        // Tell the realtime controller what that frame cost, so
                        // playback can drop to a coarser preview when this machine
                        // cannot hold the composition's rate (K-171). Here because
                        // this is the only place that knows both halves: what the
                        // worker measured, and whether the clock has run away from
                        // it regardless (`observed_cost`).
                        if matches!(playback.mode, BridgePlaybackMode::Adaptive) {
                            crate::realtime::observe(
                                playback.observed_cost(cost),
                                playback.fps,
                                crate::realtime::tier_scale(tier),
                            );
                        }
                    }
                    Err(err) => {
                        // A frame that will not render stops playback rather than
                        // spinning on it — the alternative is a silent loop burning
                        // a core on a comp that cannot be drawn.
                        eprintln!("Playback stopped: {err}");
                        state.playback = None;
                        _ = stream.add(WorkerResponse::PlaybackEnded);
                    }
                }
                return;
            }
        }
        // Nothing left to schedule: playback ends once the ring has drained.
        if playback.ring.is_empty() {
            state.playback = None;
            _ = stream.add(WorkerResponse::PlaybackEnded);
            return;
        }
    }

    // Ring full (or everything is rendered) and nothing due: wait, in slices
    // capped well below a frame so a stop or a seek arriving mid-wait is still
    // acted on promptly — the loop simply comes back round.
    let queued: Vec<u64> = playback.ring.iter().map(|(frame, _)| *frame).collect();
    if let Some(wait) = playback.wait_until_present(&queued) {
        // The last stretch before the due time is spun, not slept: an OS sleep
        // is only as fine as the system timer, and oversleeping the due time by
        // one timer tick is a whole frame at 100 fps. The spin is bounded by
        // the same 2 ms, so a stop arriving mid-wait is still seen promptly.
        const SPIN: std::time::Duration = std::time::Duration::from_millis(2);
        if wait > SPIN {
            std::thread::sleep((wait - SPIN).min(std::time::Duration::from_millis(4)));
        } else {
            let due = std::time::Instant::now() + wait;
            while std::time::Instant::now() < due {
                std::hint::spin_loop();
            }
        }
    }
}

/// Show one already-rendered ring frame — the present half of the pipeline,
/// one GPU copy plus the handle relay to Dart. A failed present drops the
/// frame and says so; it never takes playback down.
#[frb(ignore)]
fn present_ring_frame(
    renderer: &mut HeadlessRenderer,
    frame: u64,
    prepared: &lumit_render::PreparedFrame,
    stream: &mut WorkerResponseStream,
) {
    #[cfg(any(
        all(windows, feature = "shared-texture"),
        all(target_os = "macos", feature = "shared-texture-macos")
    ))]
    match renderer.present_prepared(prepared) {
        Ok(shared) => {
            _ = stream.add(WorkerResponse::RenderedSharedTexture(
                BridgeSharedFrameInfo {
                    handle: shared.handle,
                    frame,
                    width: shared.width,
                    height: shared.height,
                    tier: crate::realtime::tier(),
                },
            ));
        }
        Err(err) => eprintln!("Shared-texture present failed, dropping frame: {err}"),
    }

    #[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
    match renderer.present_prepared_dmabuf(prepared) {
        Ok(shared) => {
            _ = stream.add(WorkerResponse::RenderedDMABuf(BridgeSharedFrameInfoLinux {
                fd: shared.fd,
                frame,
                width: shared.width,
                height: shared.height,
                stride: shared.stride,
                offset: shared.offset,
                drm_fourcc: shared.drm_fourcc,
                modifier: shared.modifier,
                tier: crate::realtime::tier(),
            }));
        }
        Err(err) => eprintln!("Shared DMA-BUF present failed, dropping frame: {err}"),
    }

    #[cfg(not(any(
        all(windows, feature = "shared-texture"),
        all(target_os = "linux", feature = "shared-texture-linux"),
        all(target_os = "macos", feature = "shared-texture-macos")
    )))]
    {
        let _ = (renderer, frame, prepared, stream);
        eprintln!("No zero-copy transport in this build; dropping the frame");
    }
}

/// Begin playing, reading the composition's rate and length once up front.
///
/// Playing from the last frame plays from the start, which is what a transport
/// has to do: pressing play at the end otherwise showed itself playing while
/// nothing moved.
#[frb(ignore)]
fn start_playback(req: PlayRequest, state: &mut WorkerState) -> Result<(), BridgeError> {
    let (document, revision) = {
        let document = state.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        (document.store.snapshot(), document.store.revision())
    };
    let comp = document.comp(req.comp.id).ok_or(BridgeError::InvalidComp)?;
    let comp_id = req.comp.id;
    let fps = comp.frame_rate.fps();
    // The same derivation `CompositionReference::duration_frames` uses: the
    // document stores a length in seconds, and the count is that read at the
    // comp's current rate.
    let frames = comp
        .frame_rate
        .frame_at(lumit_core::time::CompTime(comp.duration.0));
    let last = frames.max(1).saturating_sub(1) as u64;

    let from = if req.from >= last { 0 } else { req.from };

    // Ask the disk tier for the first stretch NOW, before the first render
    // turn. The ring fills by rendering back-to-back at the start of a run, so
    // a lead measured from the render head is no lead at all there — a parked
    // span's copies always arrived just after their frames had been composited
    // from scratch, and the start of every pass was paid for twice. Asked
    // here, the IO thread works through the span while the pre-roll runs, and
    // every-frame's bounded grace bridges the first few frames. A fresh run
    // starts at Full (the reset below), so the names are at the plain scale.
    let quality = quality_for(req.scale);
    let bgra = zero_copy_wants_bgra();
    state.renderer.presync_items(&document, comp_id);
    let ask_to = from.saturating_add(DISK_PRE_ASK).min(last);
    for frame in from..=ask_to {
        let name = state
            .names
            .get_or_compute(revision, comp_id, frame, quality.tag(), || {
                state
                    .renderer
                    .frame_key_presynced(&document, comp_id, frame, quality)
            });
        let Some(key) = name else { continue };
        if wants_disk_lead(
            state.renderer.has_frame_texture(key, bgra),
            crate::framecache::contains(key),
            state.disk.contains(key),
            state.disk_wanted.contains_key(&key),
        ) {
            state.disk_wanted.insert(
                key,
                DiskWant {
                    provenance: lumit_render::FrameProvenance {
                        comp: comp_id,
                        frame,
                        scale_q: lumit_render::preview_scale_q(quality),
                        quality,
                    },
                    asked: std::time::Instant::now(),
                },
            );
            _ = state
                .disk
                .tx
                .send(lumit_render::diskio::Cmd::Load { hash: key, bgra });
        }
    }

    state.playback = Some(Playback {
        comp: req.comp,
        pending_audio: Some(req.audio),
        next: from,
        last,
        mode: req.mode,
        scale: req.scale,
        fps: if fps > 0.0 { fps } else { 60.0 },
        from,
        started: std::time::Instant::now(),
        last_presented: None,
        next_present_due: None,
        ring: std::collections::VecDeque::new(),
        costs: crate::playback::CostWindow::default(),
        prefetched_to: None,
        audio_held_for_picture: false,
        on_time_run: 0,
        skipped: 0,
        present_costs: crate::playback::CostWindow::default(),
        pace_reported: std::time::Instant::now(),
    });
    // A fresh run starts optimistic at Full and walks down to whatever this
    // machine can actually hold, rather than inheriting the last run's verdict
    // on a comp that may since have got lighter.
    crate::realtime::reset();
    Ok(())
}

/// Take everything queued, throw away what has been superseded, and serve the
/// rest.
#[frb(ignore)]
fn handle_requests(
    request: WorkerRequest,
    receiver: &Receiver<WorkerRequest>,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) {
    {
        // Latest wins — but *per kind*, which is the whole point.
        //
        // Anything that queued while the previous frame rendered is superseded:
        // a drag emits a request every ~20 ms and a render takes longer, so
        // without this the worker works through a backlog nothing will ever
        // see, each one delaying the only frame the user is waiting for
        // (docs/13 §2, B3: the *first* frame after an interaction is budgeted).
        //
        // What a picture supersedes is another picture. Draining to the single
        // newest request of any kind meant a Scopes trace threw away every
        // frame render queued behind it — and during playback the Scopes panel
        // asks every 120 ms while the Viewer asks every tick, so the picture
        // froze on its first frame while the scopes kept updating. A trace and
        // a frame are different jobs; neither is the other's replacement.
        let (pictures, scope, sample, superseded) =
            drain_to_newest(request, receiver, classify_request);
        // Deliberately not logged. Superseding is the normal, healthy case —
        // it is how a drag stays attached to the pointer — and a line per
        // completed render is console I/O on the worker thread for something
        // that happens sixty times a second. `cache_stats` is where to look for
        // how the Viewer is actually doing.
        let _ = superseded;

        // Pictures first: they are what the user is looking at, and a trace of
        // a frame that is about to be replaced is worth less than the frame.
        //
        // A frame that cannot be rendered is dropped, not fatal: the worker has
        // to survive to serve the next request.
        for request in pictures.into_iter().chain(scope).chain(sample) {
            let outcome = match request {
                WorkerRequest::RenderComp(req) => render_comp(req, state, stream),
                WorkerRequest::SamplePixels(req) => sample_pixels(req, state, stream),
                // Named for what it does rather than "render", so the three
                // variants do not all share a prefix that says nothing.
                WorkerRequest::TraceScope(req) => trace_scope(req, state, stream),
                WorkerRequest::RenderCompWithPreview(req) => {
                    render_comp_with_preview(req, state, stream)
                }
                WorkerRequest::Play(req) => start_playback(req, state),
                WorkerRequest::StopPlayback => {
                    state.playback = None;
                    Ok(())
                }
                WorkerRequest::SetViewerLook {
                    stops,
                    tone_map,
                    transparent_background,
                    region,
                } => {
                    state
                        .renderer
                        .set_display_view(lumit_render::DisplayParams::from_stops(stops, tone_map));
                    state
                        .renderer
                        .set_transparent_background(transparent_background);
                    state.renderer.set_region(region);
                    // The look is folded into every frame's name
                    // (`named_under_view`), so this message renames every
                    // frame without moving the document revision — the one
                    // case the name memo's revision check cannot see. Left
                    // standing, the memo kept serving the old look's names:
                    // the cache bar read all-zero and the idle fill
                    // re-rendered frames it had already banked, for as long
                    // as the grid was up — which is its default state.
                    state.names.clear();
                    // And both readers of those names start over: the bar
                    // sweep rebuilds against the new names rather than
                    // refining a strip of the old ones, and the fill gets to
                    // find its window cold again.
                    state.published_bar = None;
                    state.fill_exhausted = false;
                    Ok(())
                }
            };
            if let Err(err) = outcome {
                eprintln!("Dropping frame: {err}");
            }
        }
    }
}

/// How the drain treats each request kind.
///
/// A [`WorkerRequest::RenderComp`] is always newest-wins, WHATEVER its mode:
/// since playback moved into the worker (K-181) the only RenderComp traffic
/// is "show me the frame under the playhead", and a playhead position the
/// user has already dragged past will never be looked at. Treating every-frame
/// scrubs as keep-all — a leftover from the deleted Dart-side playback
/// pipeline — made a playhead drag render every frame it crossed, in order,
/// long after the user had let go.
///
/// Transport commands are not pictures and must never be dropped: superseding
/// a Stop would leave playback running with nothing left to stop it. A display
/// view (K-314) is not a picture either, and for the same reason: the last one
/// queued is the state the renderer must end up in, so dropping one because a
/// newer request of another kind arrived would leave the Viewer exposing frames
/// after the user had set it back to neutral.
#[frb(ignore)]
fn classify_request(r: &WorkerRequest) -> DrainClass {
    match r {
        WorkerRequest::TraceScope(_) => DrainClass::Scope,
        WorkerRequest::SamplePixels(_) => DrainClass::Sample,
        WorkerRequest::Play(_)
        | WorkerRequest::StopPlayback
        | WorkerRequest::SetViewerLook { .. } => DrainClass::PictureKeepAll,
        WorkerRequest::RenderComp(_) | WorkerRequest::RenderCompWithPreview(_) => {
            DrainClass::PictureNewestWins
        }
    }
}

/// How the drain treats one queued request.
#[frb(ignore)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum DrainClass {
    /// A stale one is worthless: only the newest survives (a scrub — the
    /// playhead position behind the newest will never be looked at).
    PictureNewestWins,
    /// Every one is served, in order (transport commands: Play and Stop; and
    /// the display view, which is a setting rather than a picture).
    PictureKeepAll,
    /// A trace; the newest survives, served after the pictures.
    Scope,
    /// A dropper read; the newest survives, served after the pictures — and in
    /// its own lane, not the trace's. The two are different questions, and a
    /// Scopes panel open while the dropper is armed must not make either one
    /// throw the other away (the same reasoning that gave a trace its own lane
    /// against a frame).
    Sample,
}

/// Take everything queued and keep what its class says to keep.
///
/// Generic over the classifier so the policy can be tested on its own — a
/// `WorkerRequest` needs a live project behind it, and the rule being tested has
/// nothing to do with rendering.
///
/// Returns `(pictures_in_order, scope, sample, superseded_count)`.
#[frb(ignore)]
fn drain_to_newest<T>(
    first: T,
    receiver: &Receiver<T>,
    classify: impl Fn(&T) -> DrainClass,
) -> (Vec<T>, Option<T>, Option<T>, usize) {
    let mut kept: Vec<T> = Vec::new();
    let mut newest_wins: Option<T> = None;
    let mut scope = None;
    let mut sample = None;
    let mut superseded = 0usize;
    let mut newest = Some(first);
    while let Some(item) = newest.take() {
        match classify(&item) {
            DrainClass::Scope => {
                if scope.replace(item).is_some() {
                    superseded += 1;
                }
            }
            DrainClass::Sample => {
                if sample.replace(item).is_some() {
                    superseded += 1;
                }
            }
            DrainClass::PictureKeepAll => kept.push(item),
            DrainClass::PictureNewestWins => {
                if newest_wins.replace(item).is_some() {
                    superseded += 1;
                }
            }
        }
        newest = receiver.try_recv().ok();
    }
    // A surviving newest-wins picture runs after the kept ones: the kept ones
    // were asked for earlier, and order is part of every-frame's contract.
    kept.extend(newest_wins);
    (kept, scope, sample, superseded)
}

/// Translate one measured frame for the frontend (docs/13 §7.1). Ids cross as
/// strings because that is how every other reference does — the frontend
/// matches them against the ids its read model already holds.
#[frb(ignore)]
fn profile_of(p: &lumit_render::FrameProfile) -> crate::api::state::BridgeFrameProfile {
    crate::api::state::BridgeFrameProfile {
        frame: p.frame,
        total_ms: f64::from(p.total_ms),
        layers: p
            .layers
            .iter()
            .map(|l| crate::api::state::BridgeLayerTiming {
                layer: l.layer.to_string(),
                ms: f64::from(l.ms),
                effects: l
                    .effects
                    .iter()
                    .map(|e| crate::api::state::BridgeEffectTiming {
                        effect: e.effect.to_string(),
                        ms: f64::from(e.ms),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Render the one frame the user is waiting for, with the bar running.
///
/// The switches are turned on for this frame and off again straight after, so
/// **nothing else the worker renders is watched or measured** — not playback,
/// not the idle cache fill, not a scope trace. That is the whole rule
/// (docs/07 §2.5: the bar never appears during playback), and keeping it here
/// rather than in each render path is what stops a later caller forgetting it.
/// The one other measured render is [`measure_pending`]'s: the same frame
/// this one asked for, composited again in an idle moment because a tier
/// served it without numbers (K-420).
///
/// The closing report is sent whatever happened inside, including a frame that
/// faulted or one served straight from the cache with no report at all: a bar
/// that was started must always be ended.
#[frb(ignore)]
fn watched<R>(
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
    frame: u64,
    render: impl FnOnce(&mut WorkerState, &mut WorkerResponseStream) -> R,
) -> R {
    state.renderer.watch_frames(true);
    state.renderer.measure_frames(crate::profiling::wanted());
    let out = render(state, stream);
    state.renderer.watch_frames(false);
    state.renderer.measure_frames(false);
    _ = stream.add(WorkerResponse::RenderProgress(
        crate::api::state::BridgeRenderProgress {
            frame,
            stage: lumit_render::RenderStage::Presenting.code(),
            fraction: 1.0,
            done: true,
        },
    ));
    out
}

fn render_comp(
    req: RenderCompRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let document = {
        let document = state.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        document.store.snapshot()
    };

    // The user is looking here now: anchor the idle fill on it, and wake it.
    state.last_shown = Some((req.comp.clone(), req.frame, req.scale));
    state.fill_exhausted = false;
    watched(state, stream, req.frame, |state, stream| {
        publish_frame(
            state,
            req.comp.id,
            req.frame,
            req.scale,
            &document,
            stream,
            req.mode,
            // A committed document: cacheable, and a held frame serves the scrub.
            true,
        );
    });
    Ok(())
}

/// Replace a text layer's document with the one being typed (K-225).
///
/// Only a text layer has a document to replace; anything else is a preview from
/// a layer that changed kind under the tool, and is ignored rather than failing
/// the frame — a provisional picture is never worth taking the worker down for.
#[frb(ignore)]
fn apply_text_preview(
    kind: &mut lumit_core::model::LayerKind,
    document: crate::api::assets::BridgeTextDocument,
) {
    if let lumit_core::model::LayerKind::Text { document: existing } = kind {
        // Through the one conversion, so the typing preview carries the
        // expression and applies the same "an empty box is no expression"
        // rule as a committed write. Rebuilding the document by hand here
        // dropped the expression, so a preview frame of an expression-driven
        // caption fell back to the typed words mid-keystroke.
        *existing = crate::api::assets::text_document_of(document);
    }
}

/// Render a frame under effect values the user is still dragging.
///
/// The effect stack is patched on a *clone* of the snapshot, so a drag never
/// touches the document — no commit, no undo entry, no journal write.
///
/// Note this is a *different* idiom from the v0 bridge's `preview_effect_param`
/// (ABI 12), which keeps a persistent overlay in `Bridge::preview` and replays
/// `Op::SetLayerEffects` over it. Here the whole effect list rides along with the
/// render request instead. Worth converging on one of the two when this path is
/// finished.
fn render_comp_with_preview(
    req: RenderCompRequestWithPreview,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let mut document = {
        let document = state.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        (*document.store.snapshot()).clone()
    };

    let comp = document
        .comp_mut(req.layer.comp_id)
        .ok_or(BridgeError::InvalidComp)?;
    let (comp_width, comp_height) = (comp.width, comp.height);

    let index = comp
        .layers
        .iter()
        .position(|i| i.id == req.layer.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;

    if let Some(effects) = req.effects {
        comp.layers[index].effects = effects;
    }
    if let Some(document) = req.text {
        apply_text_preview(&mut comp.layers[index].kind, document);
    }
    if let Some(paint) = req.paint {
        // Keys cross the seam on the comp clock (K-213), so the layer's own
        // zero comes back off on the way in, exactly as the retime below does.
        let offset = comp.layers[index].start_offset.0;
        comp.layers[index].paint = paint
            .into_iter()
            .map(|s| s.write_at(offset))
            .collect::<Result<Vec<_>, _>>()?;
    }
    if let Some(map) = req.retime {
        // Keys cross the seam on the comp clock (K-213), so the layer's own
        // zero comes back off on the way in. A layer with no Retime is left
        // alone rather than given one: a preview must not invent a state the
        // document cannot be in.
        let offset = comp.layers[index].start_offset.0;
        if let (Ok(animation), Some(retime)) =
            (map.animation_at(offset), comp.layers[index].retime.clone())
        {
            comp.layers[index].retime = Some(lumit_core::anim::Property {
                animation,
                extra: retime.extra,
            });
        }
    }
    if let Some((clip, map)) = req.clip_retime {
        // Clip time, so no layer offset is applied on the way in.
        if let Ok(animation) = map.animation_at(lumit_core::time::Rational::ZERO) {
            if let lumit_core::model::LayerKind::Sequence { clips } = &mut comp.layers[index].kind {
                if let Some(c) = clips.iter_mut().find(|c| c.id == clip) {
                    c.retime = Some(lumit_core::anim::Property {
                        animation,
                        extra: serde_json::Map::new(),
                    });
                }
            }
        }
    }
    if let Some(masks) = req.masks {
        // The preview's masks are the layer's own, so they read on its clock.
        let offset = comp.layers[index].start_offset.0;
        let written: Result<Vec<_>, _> = masks.into_iter().map(|m| m.write(offset)).collect();
        if let Ok(written) = written {
            comp.layers[index].masks = written;
        }
    }
    if let Some(items) = req.contents {
        // Only a shape layer has art; a stale request against another kind
        // renders the layer as it stands rather than failing the frame, which
        // is the same courtesy `apply_text_preview` gives.
        if let lumit_core::model::LayerKind::Shape { contents } = &mut comp.layers[index].kind {
            *contents = items.into_iter().map(|i| i.write_item()).collect();
        }
    }
    if let Some(transform) = &req.transform {
        // The preview's keys arrive on the composition's clock like every other
        // read (K-213); the layer's own offset carries them back.
        let offset = comp.layers[index].start_offset.0;
        transform.write_at(&mut comp.layers[index].transform, offset)?;
    }

    // A drag is not playback, so EveryFrame: the adaptive tier learns from a
    // dozen measured frames and a drag is over before it has finished, which is
    // why the drag has a resolution rule of its own (K-383). Every call that
    // reaches here is a live drag — a release commits and comes back through
    // the ordinary render path at the Viewer's own scale — so the reduction is
    // unconditional here rather than being flagged from Dart, and it covers
    // every drag the frontend has: effects, transform, masks, shapes, text,
    // paint, and the Viewer gizmos.
    //
    // NOT cacheable either — these pixels are of provisional values the
    // document never committed, so they must neither be served back later nor
    // displace honest frames. It IS the case the bar exists for, though: a
    // dragged value on a heavy comp is exactly where the picture goes quiet.
    let document = std::sync::Arc::new(document);
    let scale = crate::realtime::drag_scale(comp_width, comp_height, req.scale);
    watched(state, stream, req.frame, |state, stream| {
        publish_frame(
            state,
            req.comp.id,
            req.frame,
            scale,
            &document,
            stream,
            BridgePlaybackMode::EveryFrame,
            false,
        );
    });
    Ok(())
}

/// Trace `frame` and publish the result.
///
/// Always a CPU read-back even on a zero-copy build: the binning kernel needs
/// the pixels, and on those builds nothing ever brings them back. A failure
/// publishes nothing rather than taking the worker down — a scope that cannot
/// draw is a blank panel, not a lost session.
#[frb(ignore)]
fn trace_scope(
    req: RenderScopeRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let document = {
        let document = state.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        document.store.snapshot()
    };

    // Reuse the picture the Viewer already has, at whatever resolution it was
    // made at. Scopes read the *values* in a frame, so any size answers the
    // question — and compositing the composition a second time to ask it was
    // doubling the cost of every played frame with the panel open.
    // Only a frame that is still what this position *shows* will do (K-330):
    // an edit renames every frame it touches, and the entry the edit orphaned
    // keeps claiming the position it was made for. Asked at the quality each
    // candidate was made at, so a Half-resolution frame is judged by the Half
    // name and not by the Full one.
    // The card first: on a zero-copy build the frame the Viewer shows lives
    // there and nowhere else, so a frame the bar showed green was composited
    // a second time for every trace of it. Reading it back is a copy.
    let quality = quality_for(req.scale);
    let bgra = zero_copy_wants_bgra();
    let on_card = state
        .renderer
        .frame_key(&document, req.comp.id, req.frame, quality)
        .and_then(|key| state.renderer.read_back_frame_texture(key, bgra))
        .map(|(width, height, mut bytes)| {
            if bgra {
                // The Scopes bin R, G and B by name.
                for px in bytes.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
            }
            (width, height, bytes)
        });
    let still_current = |key: u128, quality: lumit_render::Quality| {
        state
            .renderer
            .frame_key_presynced(&document, req.comp.id, req.frame, quality)
            == Some(key)
    };
    let held =
        on_card.or_else(|| crate::framecache::best_frame(req.comp.id, req.frame, still_current));
    let (width, height, rgba) = match held {
        Some(held) => held,
        None => {
            // Nothing held for this frame — the zero-copy Viewer keeps no bytes,
            // so on that path the trace still has to make its own. Cached under
            // the frame's content name, so a second trace of the same frame is
            // free; an unnameable frame (footage still being probed) is traced
            // without banking anything.
            let key = state
                .renderer
                .frame_key(&document, req.comp.id, req.frame, quality);
            let provenance = lumit_render::FrameProvenance {
                comp: req.comp.id,
                frame: req.frame,
                scale_q: lumit_render::preview_scale_q(quality),
                quality,
            };
            let made = match key.and_then(crate::framecache::get) {
                Some(hit) => Some(hit),
                None => {
                    // A flare that fell back to the previous lens during the
                    // render (K-350) made a picture the name taken before it
                    // no longer describes. Banked only when no flare stood
                    // anything in (K-431).
                    let subs_before = state.renderer.flare_substitutions();
                    let made = state
                        .renderer
                        .render_preview(
                            &document,
                            req.comp.id,
                            req.frame,
                            quality_for(req.scale),
                            req.scale,
                        )
                        .ok()
                        .map(|(rgba, width, height)| (width, height, rgba));
                    if let (Some(key), Some((w, h, px))) = (key, made.as_ref()) {
                        if state.renderer.flare_substitutions() == subs_before {
                            crate::framecache::put_rendered(key, provenance, *w, *h, px);
                        }
                    }
                    made
                }
            };
            let Some(made) = made else {
                eprintln!("Scope render failed, dropping the trace");
                return Ok(());
            };
            made
        }
    };

    match state
        .renderer
        .render_scope(&rgba, width, height, req.kind, req.colours)
    {
        Ok(trace) => {
            _ = stream.add(WorkerResponse::Scope(crate::api::state::BridgeScopeTrace {
                kind: req.kind,
                rgba: trace,
            }));
        }
        Err(err) => eprintln!("Scope trace failed: {err}"),
    }
    Ok(())
}

/// Answer one dropper read: find the pixels, cut the window, publish it.
///
/// **A window, not a pixel.** The reply carries a whole square of the picture —
/// [`MAX_WINDOW`] a side — so the frontend can follow the pointer through it
/// without asking again. One read then serves a whole sweep of the pointer and
/// every change of sample size, instead of a request, a render lookup and a
/// stream message per mouse move. It stays a *reading* rather than a picture:
/// 129×129 is 66 KiB, a fraction of a millisecond in the codec, against 8 MiB
/// for a 1080p frame (K-183's reason for deleting the read-back transport).
///
/// The picture comes from the same places a trace's does, in the same order:
/// the frame already banked in RAM — read **in place**, never cloned, since
/// cutting a window out of eight megabytes by copying all eight is the cost
/// this is here to avoid — else a render of it, banked so the next read of the
/// same frame is free. A **layer** read is the one that can reuse neither: it
/// needs that layer alone, which is not what the composite shows, so it renders
/// the composition with the layer soloed and keeps the result in
/// `layer_sample`.
#[frb(ignore)]
fn sample_pixels(
    req: SamplePixelsRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let (document, revision) = {
        let document = state.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        (document.store.snapshot(), document.store.revision())
    };

    let layer_alone = req.layer.is_some();

    // The point arrives as a FRACTION of the picture, not as a pixel of
    // anything, and that is deliberate: the picture actually read may be a
    // reduced-resolution preview, so its pixel grid is not the composition's
    // and neither side can name a pixel in the other's. The reply says which
    // raster it cut from, and every pixel the caller then names is in that one.
    let (u, v) = (req.u.clamp(0.0, 1.0), req.v.clamp(0.0, 1.0));

    let cut = match &req.layer {
        Some(layer) => sample_layer_alone(&document, revision, layer.layer_id, &req, state)
            .and_then(|(w, h, rgba)| cut_patch(&rgba, w, h, u, v, req.window).map(|p| (p, w, h))),
        None => {
            // In place, under the cache lock: a bounded copy of the window's
            // own pixels, not of the frame around them. A frame that came down
            // off the card is BGRA on two of the three platforms, thus the
            // window — and only the window — is put right after the cut.
            // Stale entries are passed over here for the same reason as in
            // `trace_scope` (K-330): a dropper reading the picture frame 12
            // used to show is a wrong number, not a stale one.
            let still_current = |key: u128, quality: lumit_render::Quality| {
                state
                    .renderer
                    .frame_key_presynced(&document, req.comp.id, req.frame, quality)
                    == Some(key)
            };
            let held = crate::framecache::with_best_frame(
                req.comp.id,
                req.frame,
                still_current,
                |bytes, w, h, bgra| {
                    cut_patch(bytes, w, h, u, v, req.window).map(|mut p| {
                        if bgra {
                            for px in p.rgba.chunks_exact_mut(4) {
                                px.swap(0, 2);
                            }
                        }
                        (p, w, h)
                    })
                },
            )
            .flatten();
            match held {
                Some(cut) => Some(cut),
                // Nothing banked for this frame: render it once (banked under
                // the frame's content name, so a re-read of the same frame is
                // free) and cut from that.
                None => {
                    let quality = quality_for(req.scale);
                    let name = state
                        .renderer
                        .frame_key(&document, req.comp.id, req.frame, quality);

                    // A frame that cannot be named yet (its footage is still
                    // being probed, or a flare bake is being made) is rendered
                    // and not banked: an entry under a name the renderer did
                    // not keep is worse than no entry. A flare can also fall
                    // back to the previous lens *during* the render, which
                    // only the render can report — hence the count read
                    // either side of it (K-350, K-431).
                    let provenance = lumit_render::FrameProvenance {
                        comp: req.comp.id,
                        frame: req.frame,
                        scale_q: lumit_render::preview_scale_q(quality),
                        quality,
                    };
                    let made = match name.and_then(crate::framecache::get) {
                        Some(hit) => Some(hit),
                        None => {
                            let subs_before = state.renderer.flare_substitutions();
                            let made = state
                                .renderer
                                .render_preview(
                                    &document,
                                    req.comp.id,
                                    req.frame,
                                    quality,
                                    req.scale,
                                )
                                .ok()
                                .map(|(rgba, width, height)| (width, height, rgba));
                            if let (Some(key), Some((w, h, px))) = (name, made.as_ref()) {
                                if state.renderer.flare_substitutions() == subs_before {
                                    crate::framecache::put_rendered(key, provenance, *w, *h, px);
                                }
                            }
                            made
                        }
                    };
                    made.and_then(|(w, h, rgba)| {
                        cut_patch(&rgba, w, h, u, v, req.window).map(|p| (p, w, h))
                    })
                }
            }
        }
    };

    // Nothing to read: no reply is itself the answer — the magnifier keeps the
    // window it had until a new one arrives, rather than blanking.
    let Some((patch, width, height)) = cut else {
        return Ok(());
    };

    _ = stream.add(WorkerResponse::Sampled(
        crate::api::state::BridgeSampledPixels {
            window: patch.window,
            rgba: patch.rgba,
            width,
            height,
            x: patch.x,
            y: patch.y,
            frame: req.frame,
            layer_alone,
        },
    ));
    Ok(())
}

/// The composition rendered with one layer soloed — that layer alone, in its
/// own place, on nothing.
///
/// Held in `state.layer_sample` against `(comp, frame, layer, revision)`: the
/// dropper asks again on every pointer move, and re-compositing the whole
/// composition per move is not a thing to do while someone is dragging a
/// pointer. An edit moves the document's revision, which retires the entry.
#[frb(ignore)]
fn sample_layer_alone(
    document: &lumit_core::Document,
    revision: u64,
    layer: Uuid,
    req: &SamplePixelsRequest,
    state: &mut WorkerState,
) -> Option<(u32, u32, Vec<u8>)> {
    let stamp = (req.comp.id, req.frame, layer, revision);
    if let Some(held) = &state.layer_sample {
        if held.stamp == stamp {
            return Some((held.width, held.height, held.rgba.clone()));
        }
    }

    // A patched *copy* of the snapshot: soloing for the read must never be
    // something the document remembers, so nothing here goes near `commit`.
    let mut patched = lumit_core::Document::clone(document);
    let comp = patched.comp_mut(req.comp.id)?;
    for l in &mut comp.layers {
        l.switches.solo = l.id == layer;
        // Soloed and still hidden is nothing at all — and a depth pass is very
        // often hidden, which is exactly why this read exists.
        if l.id == layer {
            l.switches.visible = true;
        }
    }

    let (rgba, w, h) = state
        .renderer
        .render_preview(
            &std::sync::Arc::new(patched),
            req.comp.id,
            req.frame,
            quality_for(req.scale),
            req.scale,
        )
        .ok()?;
    state.layer_sample = Some(LayerSample {
        stamp,
        width: w,
        height: h,
        rgba: rgba.clone(),
    });
    Some((w, h, rgba))
}

/// A window cut out of a picture: the pixels, and where its centre landed.
#[frb(ignore)]
pub(crate) struct Patch {
    pub window: u32,
    pub rgba: Vec<u8>,
    pub x: u32,
    pub y: u32,
}

/// Cut a `window × window` square centred on the fraction `(u, v)` of a
/// picture.
///
/// `window` is forced odd and capped at [`MAX_WINDOW`] here rather than
/// trusted: the centre must be a single pixel for the magnifier's centre cell
/// to mean anything, and the payload must stay small enough to be a reading
/// rather than a picture. Pixels off the edge repeat the edge, so the square is
/// always exactly `window × window` and the caller never has a ragged one to
/// draw — and the frontend can index it without a bounds case at the picture's
/// border.
#[frb(ignore)]
pub(crate) fn cut_patch(
    rgba: &[u8],
    width: u32,
    height: u32,
    u: f64,
    v: f64,
    window: u32,
) -> Option<Patch> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }
    let grid = window.clamp(1, MAX_WINDOW) | 1;
    let w = width as i64;
    let h = height as i64;
    let cx = ((u * width as f64) as i64).clamp(0, w - 1);
    let cy = ((v * height as f64) as i64).clamp(0, h - 1);
    let half = i64::from(grid / 2);

    let mut out = Vec::with_capacity((grid * grid * 4) as usize);
    for dy in 0..i64::from(grid) {
        for dx in 0..i64::from(grid) {
            let px = (cx - half + dx).clamp(0, w - 1);
            let py = (cy - half + dy).clamp(0, h - 1);
            let i = ((py * w + px) * 4) as usize;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    Some(Patch {
        window: grid,
        rgba: out,
        x: cx as u32,
        y: cy as u32,
    })
}

/// Render one frame and publish it to Dart — always as a GPU handle (K-183).
///
/// Two implementations, selected at compile time, because the zero-copy entry
/// points only *exist* under their own platform and feature:
///
/// 1. Linux + `shared-texture-linux` → a DMA-BUF handle (K-177).
/// 2. Windows + `shared-texture` → a shared D3D12 texture handle (K-177), and
///    macOS + `shared-texture-macos` → an `IOSurfaceID` (K-195). One body: both
///    report one opaque integer naming a surface, plus its size.
///
/// The engine draws straight into a texture the runner displays and no pixels
/// cross the boundary at all; the read-back transport that copied every pixel
/// off the card and serialised it a byte at a time (~6 ms per 1.4 MB) is
/// deleted. A failed render, or a build with no zero-copy path at all, drops the
/// frame and says so; it never takes the worker down.
#[allow(clippy::too_many_arguments)]
fn publish_frame(
    state: &mut WorkerState,
    comp: Uuid,
    frame: u64,
    scale: f32,
    document: &std::sync::Arc<lumit_core::Document>,
    stream: &mut WorkerResponseStream,
    mode: BridgePlaybackMode,
    cacheable: bool,
) {
    #[cfg(any(
        all(windows, feature = "shared-texture"),
        all(target_os = "linux", feature = "shared-texture-linux"),
        all(target_os = "macos", feature = "shared-texture-macos")
    ))]
    {
        // Everything that reaches here is a scrub, an edit's render or a drag
        // — never playback or the idle fill, which call `prepare_frame`
        // directly. Of those, only a committed document's render may add to
        // the per-effect cache (K-421); a drag's provisional pictures read
        // from it and leave it alone. Off again afterwards, so the flag can
        // never leak into a playback run.
        state.renderer.keep_effect_outputs(cacheable);
        publish_zero_copy(state, comp, frame, scale, document, stream, mode, cacheable);
        state.renderer.keep_effect_outputs(false);
    }

    #[cfg(not(any(
        all(windows, feature = "shared-texture"),
        all(target_os = "linux", feature = "shared-texture-linux"),
        all(target_os = "macos", feature = "shared-texture-macos")
    )))]
    {
        let _ = (state, comp, frame, scale, document, stream, mode, cacheable);
        eprintln!("No zero-copy transport in this build; dropping the frame");
    }
}

#[cfg(all(target_os = "linux", feature = "shared-texture-linux"))]
#[allow(clippy::too_many_arguments)]
fn publish_zero_copy(
    state: &mut WorkerState,
    comp: Uuid,
    frame: u64,
    scale: f32,
    document: &std::sync::Arc<lumit_core::Document>,
    stream: &mut WorkerResponseStream,
    mode: BridgePlaybackMode,
    cacheable: bool,
) {
    // **A still frame is rendered at the scale it was asked for.** The
    // adaptive tier is playback's own instrument (K-186) — it buys a cheaper
    // composite so a run keeps time — and playback applies it itself, where
    // it is read beside the cost it is about to explain (`play_one_frame`).
    // Nothing that reaches here is playback: it is a scrub, a drag preview,
    // or the republish after a lens bake. Applying the tier to those was a
    // leftover from before K-181 moved playback into the worker, and it cost
    // real time (K-372): the tier survives a run, so after any heavy pass
    // every later scrub asked for `scale × tier` while the idle fill went on
    // banking at `scale` — different content names, so a frame the fill had
    // already made was invisible to the scrub that wanted it, and the picture
    // was composited from scratch with the cache bar showing green over it.
    let _ = mode;
    // Through the ladder, not straight to a composite: a frame already held on
    // the card, in memory, or parked on disk costs a copy or an upload rather
    // than a render (see `prepare_frame`).
    let prepared = match prepare_frame(
        state,
        document,
        comp,
        frame,
        still_quality(scale),
        false,
        cacheable,
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            // Dropped, not fatal: the next request renders afresh.
            eprintln!("Shared DMA-BUF render failed, dropping frame: {err}");
            return;
        }
    };
    let shared = match state.renderer.present_prepared_dmabuf(&prepared) {
        Ok(shared) => shared,
        Err(err) => {
            eprintln!("Shared DMA-BUF present failed, dropping frame: {err}");
            return;
        }
    };

    _ = stream.add(WorkerResponse::RenderedDMABuf(BridgeSharedFrameInfoLinux {
        fd: shared.fd,
        frame,
        width: shared.width,
        height: shared.height,
        stride: shared.stride,
        offset: shared.offset,
        drm_fourcc: shared.drm_fourcc,
        modifier: shared.modifier,
        // A still frame is made at Full, whatever playback last settled on,
        // so it must not report a tier it was not rendered at (K-372).
        tier: lumit_eval::schedule::FINEST_TIER,
    }));
}

#[cfg(any(
    all(windows, feature = "shared-texture"),
    all(target_os = "macos", feature = "shared-texture-macos")
))]
#[allow(clippy::too_many_arguments)]
fn publish_zero_copy(
    state: &mut WorkerState,
    comp: Uuid,
    frame: u64,
    scale: f32,
    document: &std::sync::Arc<lumit_core::Document>,
    stream: &mut WorkerResponseStream,
    mode: BridgePlaybackMode,
    cacheable: bool,
) {
    // **A still frame is rendered at the scale it was asked for.** The
    // adaptive tier is playback's own instrument (K-186) — it buys a cheaper
    // composite so a run keeps time — and playback applies it itself, where
    // it is read beside the cost it is about to explain (`play_one_frame`).
    // Nothing that reaches here is playback: it is a scrub, a drag preview,
    // or the republish after a lens bake. Applying the tier to those was a
    // leftover from before K-181 moved playback into the worker, and it cost
    // real time (K-372): the tier survives a run, so after any heavy pass
    // every later scrub asked for `scale × tier` while the idle fill went on
    // banking at `scale` — different content names, so a frame the fill had
    // already made was invisible to the scrub that wanted it, and the picture
    // was composited from scratch with the cache bar showing green over it.
    let _ = mode;
    // Through the ladder, not straight to a composite — see `prepare_frame`.
    let prepared = match prepare_frame(
        state,
        document,
        comp,
        frame,
        still_quality(scale),
        true,
        cacheable,
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            // Dropped, not fatal: the next request renders afresh.
            eprintln!("Shared-texture render failed, dropping frame: {err}");
            return;
        }
    };
    let shared = match state.renderer.present_prepared(&prepared) {
        Ok(shared) => shared,
        Err(err) => {
            eprintln!("Shared-texture present failed, dropping frame: {err}");
            return;
        }
    };

    _ = stream.add(WorkerResponse::RenderedSharedTexture(
        BridgeSharedFrameInfo {
            handle: shared.handle,
            frame,
            width: shared.width,
            height: shared.height,
            // A still frame is made at Full, whatever playback last
            // settled on, so it must not report a tier it was not rendered
            // at (K-372).
            tier: lumit_eval::schedule::FINEST_TIER,
        },
    ));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        drain_to_newest, playback_quality, still_quality, worth_building_for, DrainClass, Playback,
    };
    use crate::api::composition::{BridgePlaybackMode, CompositionReference};
    use std::sync::mpsc::channel;
    use uuid::Uuid;

    /// **A scrub must find what the fill banked** (K-372).
    ///
    /// The adaptive tier survives the run that set it, so before this a heavy
    /// playback pass left every later scrub asking for `scale × tier` while
    /// the idle fill went on banking at `scale`. Different scales are
    /// different content names, so the frame the fill had already made was
    /// invisible to the scrub that wanted it and the picture was composited
    /// from scratch — with the cache bar green over it, because the fill's
    /// copy really was there.
    ///
    /// The tier is passed rather than read precisely so this can be checked
    /// without touching the process-wide controller.
    #[test]
    fn a_still_frame_is_named_the_same_whatever_tier_playback_left_behind() {
        use lumit_eval::schedule::{COARSEST_TIER, FINEST_TIER};

        for scale in [1.0_f32, 0.5, 0.25] {
            let still = still_quality(scale).tag();
            // Whatever playback last settled on, a still frame is named the
            // same — which is what lets the fill and the scrub meet.
            for tier in FINEST_TIER..=COARSEST_TIER {
                assert_eq!(
                    still_quality(scale).tag(),
                    still,
                    "a still frame must not read the tier (scale {scale}, tier {tier})"
                );
            }
            // Playback still gets its coarser frame: that trade is the whole
            // point of the tier, and removing it there would make an Adaptive
            // run drop frames instead of softening.
            assert_eq!(
                playback_quality(scale, BridgePlaybackMode::Adaptive, FINEST_TIER).tag(),
                still,
                "at the finest tier the two must agree"
            );
            assert_ne!(
                playback_quality(scale, BridgePlaybackMode::Adaptive, COARSEST_TIER).tag(),
                still,
                "a coarse tier must genuinely make a playback frame cheaper"
            );
            // Every-frame playback is not paced by the tier either, so it
            // names frames exactly as a scrub does — which is what lets a
            // scrubbed span play back without re-rendering.
            assert_eq!(
                playback_quality(scale, BridgePlaybackMode::EveryFrame, COARSEST_TIER).tag(),
                still,
                "every-frame playback ignores the tier"
            );
        }
    }

    /// **A worker builds no renderer for a project that has already gone**
    /// (K-434).
    ///
    /// Building one is a GPU device and every pipeline the compositor needs,
    /// and it cannot be interrupted once begun — so a process that opens
    /// projects faster than they build piled the devices up and exhausted the
    /// card, at which point healthy projects got no picture at all. The frb
    /// suite is that process: a project per test, most of them drawing, and
    /// each one closed a moment after it opened. Serialising the builds is the
    /// other half; this is what lets the queue drain rather than build every
    /// project that has been and gone.
    #[test]
    fn a_closed_project_is_not_worth_a_renderer() {
        let project =
            crate::api::state::LumitBridgeState::new_project(None).expect("a new project");
        assert!(
            worth_building_for(&project),
            "an open project is exactly what a renderer is for"
        );

        project.close().expect("closing an open project");
        assert!(
            !worth_building_for(&project),
            "a closed project has nothing to draw and no one to ask"
        );
    }

    /// A worker's state around a real renderer, built as `worker_loop` builds
    /// it. `None` where there is no graphics adapter to build one on.
    fn worker_state(project: crate::api::project::ProjectReference) -> Option<super::WorkerState> {
        let Ok(renderer) = super::HeadlessRenderer::new() else {
            eprintln!("no graphics adapter; skipping");
            return None;
        };
        Some(super::WorkerState {
            project,
            renderer,
            preview_engine: super::PreviewEngine::default(),
            playback: None,
            prefetcher: crate::prefetch::Prefetcher::default(),
            last_shown: None,
            disk: lumit_render::diskio::spawn(),
            disk_wanted: std::collections::HashMap::new(),
            names: crate::names::NameCache::default(),
            applied_disk_budget: 0,
            seen_disk_clears: crate::framecache::disk::clears(),
            seen_disk_location: (u64::MAX, None),
            applied_vram_budget: 0,
            seen_vram_clears: crate::framecache::vram::clears(),
            published_vram: (0, 0),
            published_bar: None,
            bar_strip: Vec::new(),
            bar_refined_to: 0,
            bar_dirty: false,
            bar_published_at: std::time::Instant::now() - super::BAR_MIN_INTERVAL,
            fill_exhausted: true,
            backup_exhausted: true,
            last_request: std::time::Instant::now(),
            bakes_seen: 0,
            layer_sample: None,
            pending_measure: None,
        })
    }

    /// A project with one composition holding one solid layer, and the
    /// composition's id.
    fn project_with_solid() -> (crate::api::project::ProjectReference, Uuid) {
        project_with_solid_of(60)
    }

    /// The same, `frames` frames long at 30 fps.
    fn project_with_solid_of(frames: i64) -> (crate::api::project::ProjectReference, Uuid) {
        use lumit_core::model::{Composition, LinearColour, ProjectItem};
        use lumit_core::time::{Duration, FrameRate, Rational};

        let project =
            crate::api::state::LumitBridgeState::new_project(None).expect("a new project");
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 64,
            height: 32,
            frame_rate: FrameRate::new(30, 1).expect("30 fps"),
            duration: Duration(Rational::new(frames, 30).expect("a duration")),
            background: LinearColour([0.0, 0.0, 0.0, 0.0]),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;
        {
            let state = project.state().expect("state");
            let state = state.write().expect("write");
            state
                .store
                .commit(lumit_core::Op::AddItem {
                    index: 0,
                    item: Box::new(ProjectItem::Composition(comp)),
                })
                .expect("comp added");
        }
        CompositionReference::new(project.id, comp_id)
            .add_solid_layer()
            .expect("a solid layer");
        (project, comp_id)
    }

    /// **A held frame is shown at once, and measured afterwards** (K-420).
    ///
    /// The regression: render-time measuring is on by default (K-276 rev 8),
    /// and a measured request stepped over every tier — so a frame the cache
    /// bar showed green was composited again, fenced at every layer, the
    /// moment the playhead landed on it. Now the tier answers, and the idle
    /// turn composites the frame once more for its numbers. Fails without
    /// either half: the first assertion if the hit is refused, the last if
    /// the numbers never come.
    #[test]
    fn a_held_frame_is_served_while_measuring_and_measured_on_the_idle_turn() {
        let (project, comp) = project_with_solid();
        let Some(mut state) = worker_state(project) else {
            return;
        };
        let profiles: std::sync::Arc<std::sync::Mutex<Vec<u64>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = std::sync::Arc::clone(&profiles);
        state
            .renderer
            .set_profile_sink(Some(std::sync::Arc::new(move |p| {
                if let Ok(mut got) = into.lock() {
                    got.push(p.frame);
                }
            })));
        let document = {
            let document = state.project.state().expect("state");
            let document = document.read().expect("read");
            document.store.snapshot()
        };
        let quality = still_quality(1.0);
        let bgra = super::zero_copy_wants_bgra();

        // The fill's render: unmeasured, banked on the card.
        super::prepare_frame(&mut state, &document, comp, 3, quality, bgra, true)
            .expect("the fill renders");
        let key = state
            .renderer
            .frame_key(&document, comp, 3, quality)
            .expect("a solid is nameable");
        assert!(state.renderer.has_frame_texture(key, bgra), "banked");
        assert!(
            profiles.lock().expect("profiles").is_empty(),
            "the fill is never measured"
        );

        // The user lands on it with the column measuring, as `watched` does.
        state.renderer.measure_frames(true);
        let hits = state.renderer.frame_texture_hits();
        super::prepare_frame(&mut state, &document, comp, 3, quality, bgra, true)
            .expect("the scrub is served");
        state.renderer.measure_frames(false);
        assert_eq!(
            state.renderer.frame_texture_hits(),
            hits + 1,
            "a held frame is served, not composited again, while measuring"
        );
        assert!(
            profiles.lock().expect("profiles").is_empty(),
            "a hit made no numbers, and must not pretend to"
        );
        assert_eq!(state.pending_measure, Some((comp, 3, quality)));

        // The idle turn: the numbers arrive, the frame stays held, the slot
        // is cleared so it is not measured again and again.
        super::measure_pending(&mut state);
        assert_eq!(
            profiles.lock().expect("profiles").as_slice(),
            &[3],
            "the numbers for that frame arrive one idle turn later"
        );
        assert!(state.renderer.has_frame_texture(key, bgra), "still held");
        assert_eq!(state.pending_measure, None);
        super::measure_pending(&mut state);
        assert_eq!(profiles.lock().expect("profiles").len(), 1, "measured once");
    }

    /// **The fill does not stop at the card.** A 50-frame work area with room
    /// for 20 frames on the card used to end with 20 held and 30 never
    /// visited, and playback looping the work area re-rendered the far side
    /// every pass. Now the walk wraps and carries on past the window: what
    /// the card pushes out lands in memory, and the whole work area ends up
    /// held in one tier or the other. Fails without the change: the loop
    /// below ends with thirty frames in neither.
    #[test]
    fn the_fill_keeps_going_into_memory_once_the_card_is_full() {
        let (project, comp) = project_with_solid_of(50);
        // A still solid names every frame the same (content keying), so the
        // solid turns: fifty frames, fifty names.
        {
            let layer = {
                let state = project.state().expect("state");
                let state = state.read().expect("read");
                state.store.snapshot().comp(comp).expect("comp").layers[0].id
            };
            let state = project.state().expect("state");
            let state = state.write().expect("write");
            state
                .store
                .commit(lumit_core::Op::SetTransformProperty {
                    comp,
                    layer,
                    prop: lumit_core::model::TransformProp::Rotation,
                    animation: lumit_core::anim::Animation::Expression("time * 90".into()),
                })
                .expect("animated");
        }
        let Some(mut state) = worker_state(project) else {
            return;
        };
        // Room for twenty 64x32 frames, and a little slack.
        let frame_bytes = 64 * 32 * 4;
        state
            .renderer
            .set_frame_texture_budget(frame_bytes * 20 + frame_bytes / 2);
        let mut stream = crate::frb_generated::StreamSink::deserialize("0".into());
        // The user is looking at frame 7; that is the fill's anchor.
        let document = {
            let document = state.project.state().expect("state");
            let document = document.read().expect("read");
            document.store.snapshot()
        };
        let quality = still_quality(1.0);
        let bgra = super::zero_copy_wants_bgra();
        super::prepare_frame(&mut state, &document, comp, 7, quality, bgra, true)
            .expect("the shown frame");
        state.last_shown = Some((CompositionReference::new(state.project.id, comp), 7, 1.0));
        state.fill_exhausted = false;
        // Idle turns until the fill has nothing left; each turn also collects
        // what the card handed back, as the worker loop does.
        for _ in 0..2_000 {
            if state.fill_exhausted {
                break;
            }
            super::idle_fill(&mut state, &mut stream);
            super::drain_demotions(&mut state);
        }
        assert!(state.fill_exhausted, "the fill terminates");
        // Read-backs still in flight land over the next few turns.
        for _ in 0..200 {
            super::drain_demotions(&mut state);
        }
        let mut on_card = 0;
        let mut missing = Vec::new();
        for frame in 0..50u64 {
            let key = state
                .renderer
                .frame_key(&document, comp, frame, quality)
                .expect("a solid is nameable");
            if state.renderer.has_frame_texture(key, bgra) {
                on_card += 1;
            } else if !crate::framecache::contains(key) {
                missing.push(frame);
            }
        }
        assert!(
            missing.is_empty(),
            "held on the card or in memory: missing {missing:?}"
        );
        assert!(
            on_card <= 20,
            "the card's window is still the card's budget, got {on_card}"
        );

        // **A full card climbs nothing from disk either.** Move one memory-held
        // frame down to disk and out of memory, then run the fill again: the
        // frame must stay on disk. Before the guard covered disk as well as
        // memory, the fill uploaded it, the card pushed another frame down,
        // memory pushed a third onto disk, and the next turn found that one
        // and started again - an idle loop with no fixed point.
        let keys: Vec<_> = (0..50u64)
            .map(|f| {
                state
                    .renderer
                    .frame_key(&document, comp, f, quality)
                    .expect("nameable")
            })
            .collect();
        let parked = keys
            .into_iter()
            .find(|k| {
                !state.renderer.has_frame_texture(*k, bgra) && crate::framecache::contains(*k)
            })
            .expect("a frame held in memory only");
        let (w, h, bytes) = crate::framecache::get(parked).expect("its bytes");
        // The disk tier parks nothing until it has a folder.
        let dir = tempfile::tempdir().expect("a folder for the disk tier");
        state
            .disk
            .tx
            .send(lumit_render::diskio::Cmd::SetRoot(Some(
                dir.path().to_path_buf(),
            )))
            .expect("the disk thread");
        assert!(state
            .disk
            .park(parked, w, h, bgra, std::sync::Arc::new(bytes), 1, 1000));
        for _ in 0..500 {
            if state.disk.contains(parked) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.disk.contains(parked), "the park landed");
        crate::framecache::clear();
        let card_before = state.renderer.frame_texture_stats().0;
        state.fill_exhausted = false;
        for _ in 0..200 {
            if state.fill_exhausted {
                break;
            }
            super::idle_fill(&mut state, &mut stream);
            super::collect_disk_loads(&mut state);
            super::drain_demotions(&mut state);
        }
        assert!(state.fill_exhausted, "the second pass terminates");
        assert!(
            !state.renderer.has_frame_texture(parked, bgra),
            "a disk-held frame is not climbed onto a full card"
        );
        assert_eq!(
            state.renderer.frame_texture_stats().0,
            card_before,
            "the card is untouched"
        );
    }

    fn playback(mode: BridgePlaybackMode, last: u64) -> Playback {
        Playback {
            comp: CompositionReference::new(Uuid::nil(), Uuid::nil()),
            next: 0,
            last,
            mode,
            scale: 1.0,
            fps: 60.0,
            from: 0,
            started: std::time::Instant::now(),
            last_presented: None,
            next_present_due: None,
            ring: std::collections::VecDeque::new(),
            costs: crate::playback::CostWindow::default(),
            prefetched_to: None,
            audio_held_for_picture: false,
            on_time_run: 0,
            pending_audio: None,
            skipped: 0,
            present_costs: crate::playback::CostWindow::default(),
            pace_reported: std::time::Instant::now(),
        }
    }

    /// **The pre-roll.** The sound waits for the picture to bank a frame or two
    /// — otherwise it starts while the first composite is still running and, in
    /// adaptive mode, the picture skips to catch the clock up, so every press of
    /// play begins with a jump. The wait is bounded: a comp too heavy to bank
    /// three frames inside the budget starts anyway rather than sitting silent.
    #[test]
    fn the_sound_waits_for_the_first_frames_but_not_for_long() {
        let play = playback(BridgePlaybackMode::Adaptive, 100);
        assert!(!play.pre_roll_done(0), "nothing banked yet");
        assert!(!play.pre_roll_done(2), "still short of the pre-roll");
        assert!(play.pre_roll_done(3), "three frames is a pre-roll");

        // Budget spent: the sound starts on whatever there is.
        let mut slow = playback(BridgePlaybackMode::Adaptive, 100);
        slow.started = std::time::Instant::now() - std::time::Duration::from_millis(200);
        assert!(
            slow.pre_roll_done(0),
            "a heavy comp must not play in silence waiting for a ring"
        );
    }

    /// **The pacing regression, on the present side.** Renders are free to run
    /// ahead into the ring — that is the scheduler's point — so the PRESENT is
    /// what paces playback now. Without [`Playback::present_choice`]'s clock
    /// gate a comp cheaper than realtime would play as fast as the renderer
    /// manages, which is the "plays at several hundred fps" bug the old
    /// per-render wait existed for. Fails without the gate.
    #[test]
    fn adaptive_playback_presents_frames_only_when_the_clock_reaches_them() {
        let mut p = playback(BridgePlaybackMode::Adaptive, 100);
        let queued = [0u64, 1, 2, 3];

        // Frame 0 is due the instant playback starts; nothing beyond it is.
        assert_eq!(
            p.present_choice(&queued),
            Some(0),
            "frame 0 is due at the very start, and only frame 0"
        );

        // Half a second in, the clock has reached frame 30: the ring's newest
        // due entry is presented and everything older is dropped with it —
        // showing frame 1 half a second late is worse than not showing it.
        p.started = std::time::Instant::now() - std::time::Duration::from_millis(500);
        let queued = [28u64, 29, 30, 40];
        let chosen = p.present_choice(&queued).expect("plenty is due by now");
        assert!(
            (1..=2).contains(&chosen),
            "the newest frame the clock has reached, not the oldest queued: {chosen}"
        );

        // And a ring full of the future presents nothing at all.
        assert_eq!(p.present_choice(&[500, 501]), None, "the future can wait");
        // The wait until it is due is bounded by when frame 500 falls due.
        let wait = p.wait_until_present(&[500, 501]).expect("not due yet");
        assert!(wait.as_secs_f64() <= 500.0 / 60.0);
    }

    /// Every-frame never skips, whatever it costs — that is the mode's whole
    /// definition (K-171); when it cannot keep the comp's rate it plays slow
    /// and the sound pauses rather than drifting.
    #[test]
    fn every_frame_playback_never_skips() {
        let mut p = playback(BridgePlaybackMode::EveryFrame, 3);
        for expected in 0..=3 {
            assert_eq!(p.advance(), Some(expected), "never skips one");
        }
        assert_eq!(p.advance(), None, "past the last frame, playback is over");
    }

    /// **The cached-playback regression, on the present side.** Every-frame is
    /// allowed to fall behind — a comp too heavy to render in realtime plays
    /// slow rather than dropping frames — but it must never run *ahead*. Once a
    /// span is cached, renders cost almost nothing and the RING fills instantly;
    /// without the present gate the mode replayed cached spans many times
    /// faster than realtime: "it zooms through those parts". Fails without the
    /// per-present pacing.
    ///
    /// The gate is the present GRID (`next_present_due`), not a stopwatch from
    /// the last actual present — the stopwatch added every scrap of loop
    /// lateness to every frame, and a 60 fps comp could never actually play at
    /// 60 (the drift itself is pinned in `crate::playback`'s tests).
    #[test]
    fn every_frame_playback_never_presents_faster_than_realtime() {
        let mut p = playback(BridgePlaybackMode::EveryFrame, 100);
        let queued = [0u64, 1, 2];

        // The first frame of a run is due immediately — nothing has been shown
        // yet, so there is no grid to be early against. And it is the FRONT:
        // every-frame shows every frame, in order, never the newest.
        assert_eq!(p.present_choice(&queued), Some(0));

        // The next present is not due for ten milliseconds, however full of
        // cached frames the ring already is.
        p.next_present_due = Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
        assert_eq!(
            p.present_choice(&queued),
            None,
            "a full ring is not a licence to run ahead of the comp's rate"
        );
        let wait = p
            .wait_until_present(&queued)
            .expect("not due yet means there is a wait to sit out");
        assert!(
            wait.as_secs_f64() > 0.005 && wait.as_secs_f64() <= 0.010 + 1e-6,
            "waits out the rest of the schedule, no more: {wait:?}"
        );

        // A present that is already overdue happens now. Late is allowed;
        // making it later is not.
        p.next_present_due = Some(std::time::Instant::now() - std::time::Duration::from_millis(50));
        assert_eq!(
            p.present_choice(&queued),
            Some(0),
            "already behind, so the front goes out immediately — it never \
             tries to catch up and never adds to the delay"
        );
        assert_eq!(
            p.wait_until_present(&queued),
            None,
            "an overdue present has no wait left"
        );
    }

    /// The scheduler's slack, end to end at the decision level: cheap frames
    /// keep the ring's capacity at the impl note's floor of 8, a run of
    /// expensive ones raises it, and the raise ages out with the costs that
    /// caused it — the lookahead follows the comp the playhead is in now.
    #[test]
    fn the_ring_capacity_adapts_to_measured_render_cost() {
        let mut p = playback(BridgePlaybackMode::Adaptive, 1000);
        assert_eq!(p.capacity(), 8, "a fresh run starts at the floor");
        for _ in 0..32 {
            p.costs.push(0.1); // 6 budgets at 60 fps: a struggling comp.
        }
        assert_eq!(p.capacity(), 12, "2 × 0.1 s × 60 fps");
        for _ in 0..32 {
            p.costs.push(0.004); // The playhead moved somewhere cheap.
        }
        assert_eq!(p.capacity(), 8, "the expensive stretch ages out");
    }

    /// Adaptive skips frames the clock has already gone past, rather than
    /// falling further behind. Driven by moving the start time into the past,
    /// which is what a slow render does to the wall clock.
    #[test]
    fn adaptive_playback_skips_frames_the_clock_has_passed() {
        let mut p = playback(BridgePlaybackMode::Adaptive, 100);
        p.started = std::time::Instant::now() - std::time::Duration::from_millis(500);

        let frame = p.advance().expect("still inside the composition");
        assert!(
            frame >= 29,
            "half a second at 60 fps is about frame 30, not frame 0: got {frame}"
        );
    }

    /// **The always-Full regression.** The tier only ever saw what the worker
    /// could time — its own render and hand-off — and the rest of a frame's
    /// journey (the decode, the paint, everything the frontend does per frame)
    /// happens after the worker has let go. So on a machine where the worker
    /// spent 9 ms of a 16.7 ms budget the controller read "plenty of headroom"
    /// and stayed at Full, while playback visibly skipped frames to keep time.
    ///
    /// A skip is the symptom of the whole round trip being too slow, whoever
    /// spent the time, so it is what the cost is derived from. Fails without
    /// `observed_cost` — the reported cost would be the 9 ms busy time, which
    /// sits comfortably under the 15 ms drop threshold and moves nothing.
    #[test]
    fn skipped_frames_are_reported_as_over_budget_however_little_the_worker_spent() {
        let mut p = playback(BridgePlaybackMode::Adaptive, 1000);
        let budget = 1.0 / 60.0;

        // Keeping up: the worker's own measurement stands, so a cheap frame
        // reads cheap and the tier is free to climb back.
        p.skipped = 0;
        assert_eq!(p.observed_cost(0.009), 0.009);
        assert!(
            p.observed_cost(0.009) < 0.9 * budget,
            "a frame that kept up must not read as over budget"
        );

        // Behind by one frame: the worker still only spent 9 ms, but the round
        // trip demonstrably took more than its budget.
        p.skipped = 1;
        assert!(
            p.observed_cost(0.009) > 0.9 * budget,
            "one skipped frame means the last one cost about two budgets, \
             whatever the worker's own stopwatch says"
        );

        // And the further behind it falls, the worse the reported cost, so the
        // tier keeps coming down instead of settling one step in.
        p.skipped = 3;
        assert!(p.observed_cost(0.009) > p.observed_cost(0.009) / 2.0);
        assert_eq!(p.observed_cost(0.009), 4.0 * budget);
    }

    /// The requests these tests queue: an adaptive picture (newest wins), an
    /// every-frame picture (all kept, in order), and a scope trace. Standing in
    /// for `WorkerRequest`, which needs a live project.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum Req {
        Adaptive(u32),
        Sample(u32),
        // Kept-in-order requests — standing in for the transport commands
        // (Play, Stop), the only keep-all class since scrubs became
        // newest-wins in every mode.
        EveryFrame(u32),
        Scope(u32),
    }

    fn classify(r: &Req) -> DrainClass {
        match r {
            Req::Adaptive(_) => DrainClass::PictureNewestWins,
            Req::EveryFrame(_) => DrainClass::PictureKeepAll,
            Req::Scope(_) => DrainClass::Scope,
            Req::Sample(_) => DrainClass::Sample,
        }
    }

    /// **The playhead-drag regression.** A scrub render is newest-wins in
    /// EVERY transport mode: since playback moved into the worker (K-181),
    /// a RenderComp only ever means "show the frame under the playhead", and
    /// classifying every-frame scrubs as keep-all made a drag render every
    /// frame it crossed, in order, long after the pointer had let go.
    #[test]
    fn a_scrub_supersedes_whatever_the_transport_mode() {
        let comp = CompositionReference::new(Uuid::nil(), Uuid::nil());
        let scrub = |frame: u64, mode: BridgePlaybackMode| {
            super::WorkerRequest::RenderComp(super::RenderCompRequest {
                comp: comp.clone(),
                frame,
                mode,
                scale: 1.0,
            })
        };
        assert!(matches!(
            super::classify_request(&scrub(5, BridgePlaybackMode::EveryFrame)),
            DrainClass::PictureNewestWins
        ));
        assert!(matches!(
            super::classify_request(&scrub(5, BridgePlaybackMode::Adaptive)),
            DrainClass::PictureNewestWins
        ));
        // The transport commands stay keep-all: superseding a Stop would leave
        // playback running with nothing left to stop it.
        assert!(matches!(
            super::classify_request(&super::WorkerRequest::StopPlayback),
            DrainClass::PictureKeepAll
        ));
    }

    /// The bug this policy exists to fix: during playback the Viewer asks for a
    /// frame every tick and the Scopes panel asks for a trace every 120 ms.
    /// Draining to the single newest request of *any* kind meant one trace threw
    /// away every frame queued behind it, so the picture froze on its first
    /// frame while the scopes carried on updating.
    #[test]
    fn a_scope_trace_does_not_supersede_a_frame() {
        let (tx, rx) = channel();
        for frame in 1..=3 {
            tx.send(Req::Adaptive(frame)).unwrap();
        }
        // The trace arrives last, which is what used to win outright.
        tx.send(Req::Scope(9)).unwrap();
        drop(tx);

        let (pictures, scope, _, superseded) = drain_to_newest(Req::Adaptive(0), &rx, classify);
        assert_eq!(
            pictures,
            vec![Req::Adaptive(3)],
            "the newest frame survives a trace queued behind it"
        );
        assert_eq!(scope, Some(Req::Scope(9)), "and the trace is served too");
        assert_eq!(superseded, 3, "the three older frames were dropped");
    }

    /// The behaviour the policy is *for*: a backlog of adaptive pictures
    /// collapses to the newest, because the ones behind it are frames nobody
    /// will ever see.
    #[test]
    fn pictures_still_collapse_to_the_newest() {
        let (tx, rx) = channel();
        for frame in 1..=5 {
            tx.send(Req::Adaptive(frame)).unwrap();
        }
        drop(tx);

        let (pictures, scope, _, superseded) = drain_to_newest(Req::Adaptive(0), &rx, classify);
        assert_eq!(pictures, vec![Req::Adaptive(5)]);
        assert_eq!(scope, None, "nothing asked for a trace");
        assert_eq!(superseded, 5);
    }

    /// And traces collapse among themselves for the same reason.
    #[test]
    fn traces_collapse_to_the_newest_too() {
        let (tx, rx) = channel();
        tx.send(Req::Scope(2)).unwrap();
        tx.send(Req::Scope(3)).unwrap();
        drop(tx);

        let (pictures, scope, _, superseded) = drain_to_newest(Req::Scope(1), &rx, classify);
        assert!(pictures.is_empty());
        assert_eq!(scope, Some(Req::Scope(3)));
        assert_eq!(superseded, 2);
    }

    /// A single request with nothing behind it is served as it is.
    #[test]
    fn a_lone_request_is_not_counted_as_superseded() {
        let (tx, rx) = channel::<Req>();
        drop(tx);

        let (pictures, scope, _, superseded) = drain_to_newest(Req::Adaptive(7), &rx, classify);
        assert_eq!(pictures, vec![Req::Adaptive(7)]);
        assert_eq!(scope, None);
        assert_eq!(superseded, 0);
    }

    /// The keep-all class's contract: nothing dropped, order preserved — what
    /// keeps a Play or a Stop from vanishing under a backlog of pictures.
    #[test]
    fn every_frame_requests_all_survive_in_order() {
        let (tx, rx) = channel();
        for frame in 2..=4 {
            tx.send(Req::EveryFrame(frame)).unwrap();
        }
        // An adaptive scrub and a trace land in the middle of the backlog.
        tx.send(Req::Adaptive(9)).unwrap();
        tx.send(Req::Scope(1)).unwrap();
        drop(tx);

        let (pictures, scope, _, superseded) = drain_to_newest(Req::EveryFrame(1), &rx, classify);
        assert_eq!(
            pictures,
            vec![
                Req::EveryFrame(1),
                Req::EveryFrame(2),
                Req::EveryFrame(3),
                Req::EveryFrame(4),
                Req::Adaptive(9),
            ],
            "every-frame requests all survive, in order, before the adaptive one"
        );
        assert_eq!(scope, Some(Req::Scope(1)));
        assert_eq!(superseded, 0, "nothing every-frame was thrown away");
    }

    /// A dropper read has its own lane. The Scopes panel and an armed dropper
    /// are often open together — the panel asks every 120 ms and the magnifier
    /// asks on every pointer move — and neither question is the other's
    /// replacement, so neither may supersede the other or a frame.
    #[test]
    fn a_dropper_read_and_a_trace_do_not_supersede_each_other() {
        let (tx, rx) = channel();
        tx.send(Req::Scope(1)).unwrap();
        tx.send(Req::Sample(7)).unwrap();
        tx.send(Req::Sample(8)).unwrap();
        drop(tx);

        let (pictures, scope, sample, superseded) =
            drain_to_newest(Req::Adaptive(4), &rx, classify);
        assert_eq!(pictures, vec![Req::Adaptive(4)], "the frame survives both");
        assert_eq!(
            scope,
            Some(Req::Scope(1)),
            "and the trace survives the reads"
        );
        assert_eq!(
            sample,
            Some(Req::Sample(8)),
            "reads collapse among themselves — only where the pointer is now matters"
        );
        assert_eq!(superseded, 1, "one older read, and nothing else");
    }

    /// The window is always exactly `window × window`, odd, and centred on the
    /// pixel the fraction names — including hard against an edge, where the
    /// picture runs out and the edge pixel repeats. A ragged square would leave
    /// the magnifier drawing whatever was in memory next, and the frontend
    /// indexes this square without a border case.
    #[test]
    fn a_window_is_square_odd_and_clamped_to_the_picture() {
        // 2×2: red, green / blue, white.
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let patch = super::cut_patch(&rgba, 2, 2, 0.0, 0.0, 3).expect("a picture to read");
        assert_eq!(patch.window, 3);
        assert_eq!(patch.rgba.len(), 3 * 3 * 4);
        assert_eq!((patch.x, patch.y), (0, 0), "the top-left pixel");
        // The centre cell is the pixel asked for; the row above repeats it,
        // because there is no row above.
        assert_eq!(
            &patch.rgba[16..20],
            &[255, 0, 0, 255],
            "centre is the red pixel"
        );
        assert_eq!(
            &patch.rgba[0..4],
            &[255, 0, 0, 255],
            "off the top-left, the edge repeats"
        );
        assert_eq!(
            &patch.rgba[20..24],
            &[0, 255, 0, 255],
            "and the neighbour is the green one"
        );

        // An even window is forced odd, and one past the cap is capped, so the
        // centre cell always means one pixel and the payload stays a reading.
        assert_eq!(
            super::cut_patch(&rgba, 2, 2, 0.5, 0.5, 4)
                .expect("cut")
                .window,
            5
        );
        assert_eq!(
            super::cut_patch(&rgba, 2, 2, 0.5, 0.5, 9999)
                .expect("cut")
                .window,
            super::MAX_WINDOW
        );
        // And the cap really is a cap on the payload: the biggest reply the
        // dropper can ask for stays two orders of magnitude below a frame
        // (8 MiB at 1080p, K-183), which is what keeps this a reading.
        let biggest = super::cut_patch(&rgba, 2, 2, 0.5, 0.5, u32::MAX).expect("cut");
        assert_eq!(biggest.window, super::MAX_WINDOW);
        assert!(biggest.rgba.len() < 100_000, "{}", biggest.rgba.len());

        // A picture with fewer bytes than it claims is refused rather than read
        // past the end of.
        assert!(super::cut_patch(&[0, 0, 0, 255], 2, 2, 0.0, 0.0, 1).is_none());
        assert!(super::cut_patch(&rgba, 0, 0, 0.0, 0.0, 1).is_none());
    }

    /// The Type tool's live preview (K-225): the picture keeps up with what is
    /// being typed, and the document is not touched until the edit ends.
    #[test]
    fn a_text_preview_replaces_only_a_text_layer() {
        use crate::api::assets::{BridgeColourRgba, BridgeTextDocument};
        use lumit_core::model::{LayerKind, LinearColour, TextDocument};

        let typed = BridgeTextDocument {
            text: "Hello".into(),
            expression: None,
            size: 48.0,
            fill: BridgeColourRgba {
                r: 1.0,
                g: 0.5,
                b: 0.0,
                a: 1.0,
            },
        };

        let mut text = LayerKind::Text {
            document: TextDocument {
                text: "Text".into(),
                expression: None,
                size: 72.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                extra: serde_json::Map::new(),
            },
        };
        super::apply_text_preview(&mut text, typed.clone());
        let LayerKind::Text { document } = &text else {
            panic!("still a text layer");
        };
        assert_eq!(document.text, "Hello");
        assert_eq!(document.size, 48.0);
        assert_eq!(document.fill.0[1], 0.5);

        // A layer that is not text takes the preview without changing: a stale
        // request must never fail a frame.
        let mut other = LayerKind::Adjustment;
        super::apply_text_preview(&mut other, typed);
        assert!(matches!(other, LayerKind::Adjustment));
    }

    /// A dragged stroke previews through the same door the typed word does
    /// (K-239): the whole paint list rides along with the render request and
    /// lands on a clone, so the picture moves while the document does not.
    ///
    /// What this pins is the *conversion*. The preview carries bridge strokes
    /// and the renderer wants engine ones, so the values have to survive the
    /// crossing — including the clamping `write` does, which is the reason the
    /// preview and the commit cannot each convert in their own way.
    #[test]
    fn a_paint_preview_carries_the_strokes_across() {
        use crate::api::assets::BridgeColourRgba;
        use crate::api::layer::{BridgePaintMode, BridgeStroke, BridgeStrokePoint};

        let stroke = BridgeStroke {
            id: uuid::Uuid::from_u128(3),
            name: "Brush 1".into(),
            points: vec![
                BridgeStrokePoint { x: 4.0, y: 5.0 },
                BridgeStrokePoint { x: 40.0, y: 50.0 },
            ],
            colour: BridgeColourRgba {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            width: 12.0,
            hardness: 0.8,
            shape: crate::api::layer::BridgeBrushShape::Round,
            // Mid-drag values are provisional, so an out-of-range one must be
            // clamped rather than rendered — the same rule the commit follows.
            opacity: 140.0,
            start: crate::api::effect::BridgeScalar::Static(0.0),
            end: crate::api::effect::BridgeScalar::Static(100.0),
            mode: BridgePaintMode::Paint,
            clone_offset_x: 0.0,
            clone_offset_y: 0.0,
        };

        let written = stroke
            .write_at(lumit_core::time::Rational::ZERO)
            .expect("a valid stroke");
        assert_eq!(written.name, "Brush 1");
        assert_eq!(written.points.len(), 2);
        assert_eq!(written.points[1], (40.0, 50.0));
        assert_eq!(written.width, 12.0);
        assert!(
            written.opacity <= 100.0,
            "a provisional opacity is clamped, not rendered as it arrived"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod bar_strip_tests {
    use super::{
        audio_chase, mark_banked, on_time_limit, refine_bar_strip, sample_bar_strip,
        wants_disk_lead, AudioChase, BarFingerprint, AUDIO_REALTIME_FRAMES,
    };

    /// **The stripe greens while playback lays frames down.** The sweep walks
    /// forward from the playhead, so the frames playback just banked — behind
    /// it — were the last it reached, and the stripe sat unchanged until a
    /// pause. A banked frame is painted straight into the strip instead; the
    /// paint only lands when the strip being shown is of that comp at that
    /// scale, and only sets the dirty flag when it actually changed a pixel
    /// (the flag is what nudges the frontend to redraw).
    #[test]
    fn a_banked_frame_paints_its_own_strip_slot() {
        let comp = uuid::Uuid::now_v7();
        let fingerprint = BarFingerprint {
            comp,
            frames: 4,
            scale_q: 1000,
            revision: 0,
            vram_version: 0,
            ram_entries: 0,
            disk_entries: 0,
        };
        let mut strip = vec![0u8, 0, 0, 0];
        let mut dirty = false;

        mark_banked(Some(fingerprint), &mut strip, &mut dirty, comp, 2, 1000);
        assert_eq!(strip, vec![0, 0, 2, 0], "the banked frame reads held");
        assert!(dirty, "a changed pixel asks for a redraw");

        // The same frame again changes nothing, so it asks for nothing.
        dirty = false;
        mark_banked(Some(fingerprint), &mut strip, &mut dirty, comp, 2, 1000);
        assert!(!dirty, "an unchanged pixel is not a redraw");

        // Another comp, another scale, a frame past the strip, no strip at
        // all: each is left alone rather than painting the wrong stripe.
        mark_banked(
            Some(fingerprint),
            &mut strip,
            &mut dirty,
            uuid::Uuid::now_v7(),
            1,
            1000,
        );
        mark_banked(Some(fingerprint), &mut strip, &mut dirty, comp, 1, 500);
        mark_banked(Some(fingerprint), &mut strip, &mut dirty, comp, 99, 1000);
        mark_banked(None, &mut strip, &mut dirty, comp, 1, 1000);
        assert_eq!(strip, vec![0, 0, 2, 0]);
        assert!(!dirty);
    }

    /// The audio chase's lateness allowance is a quarter of the frame period —
    /// floored at a few milliseconds, because at high comp rates a quarter
    /// period shrinks inside ordinary scheduler jitter and the sound stopped
    /// over pictures that were holding the rate to the eye.
    #[test]
    fn the_on_time_allowance_never_shrinks_inside_scheduler_jitter() {
        let at = |fps: f64| on_time_limit(std::time::Duration::from_secs_f64(1.0 / fps));
        // 24 fps: the proportional allowance stands (41.7 + 10.4 ms).
        assert!((at(24.0).as_secs_f64() - (1.25 / 24.0)).abs() < 1e-9);
        // 120 fps: a quarter period would be ~2 ms; the floor holds instead
        // (8.3 + 5 ms), so one scheduler tick of jitter is not "late".
        let limit = at(120.0).as_secs_f64();
        assert!(
            (limit - (1.0 / 120.0 + 0.005)).abs() < 1e-9,
            "floored allowance, got {limit}"
        );
    }

    /// **The sound stops the moment the picture stops keeping time, and comes
    /// back to the picture once the picture has held the rate.**
    ///
    /// Every-frame playback shows every frame however long each takes, thus the
    /// picture can fall out of time with the sound. Lumit stops the sound rather
    /// than let it run over a picture that is not keeping up (K-171).
    ///
    /// Three rules were tried and the first two were wrong, each in its own way:
    ///
    /// * *Never start again.* The clock stops reporting when the sound stops,
    ///   thus the test that stopped it could not run a second time.
    /// * *Wait for the picture to reach the sound.* The sound stops in front of
    ///   the picture by however long the slow frame took, thus after a long
    ///   stall the picture may never reach it, and does not reach it at all if
    ///   the composition ends first.
    /// * *Start when frames are banked.* Frames are usually banked at the moment
    ///   a picture goes out, even in a run far slower than the composition's
    ///   rate — so the sound started again on the very next picture and, to the
    ///   ear, never stopped at all.
    ///
    /// What is measured now is the thing that matters: whether the pictures are
    /// arriving at the composition's rate.
    #[test]
    fn the_sound_stops_off_the_rate_and_returns_on_it() {
        let long = AUDIO_REALTIME_FRAMES;
        // Running, and the pictures are on time: nothing happens.
        assert_eq!(audio_chase(false, true, long), AudioChase::Leave);
        // Running, and ONE picture is late: the sound stops at once. This is the
        // case the "how far in front" rule left running for half a second.
        assert_eq!(audio_chase(false, false, 0), AudioChase::Hold);

        // Stopped, and the pictures are still late: it stays stopped.
        assert_eq!(audio_chase(true, false, 0), AudioChase::Leave);
        // Stopped, and a few on-time pictures have gone by — but not enough.
        // This is the case the "frames are banked" rule started on, which made
        // the sound stutter rather than stop.
        assert_eq!(audio_chase(true, true, 1), AudioChase::Leave);
        assert_eq!(audio_chase(true, true, long - 1), AudioChase::Leave);
        // Stopped, and the rate has held: the sound comes back.
        assert_eq!(audio_chase(true, true, long), AudioChase::Start);
        assert_eq!(audio_chase(true, true, long + 40), AudioChase::Start);

        // A run of on-time pictures does not, on its own, stop a running sound
        // or restart one that a late picture has just stopped: the two answers
        // are not each other's negation.
        assert_eq!(audio_chase(false, true, 1), AudioChase::Leave);
        assert_eq!(audio_chase(true, false, long), AudioChase::Leave);
    }

    /// Playback asks the disk tier for a frame in advance, and only when the
    /// read is of use — the last rung, reached only when the ones above it
    /// cannot answer. The rung above is memory, and it is climbed in advance
    /// too: `line_up_frame` uploads a held frame to the card before the frame is
    /// due, thus by the time this predicate is asked, "in memory" has already
    /// been dealt with and only a genuine file read is left.
    ///
    /// **Why this matters.** A read off disk arrives one or two turns of the
    /// worker loop after it is asked for. A frame asked for at the moment it
    /// must be shown thus always arrives too late, and the frame is composited
    /// again — which made a span parked on disk worth nothing to playback. The
    /// loop asks for the coming frames instead, at the same time as it posts
    /// their source decodes.
    ///
    /// The rule is tested here; that playback applies it over the whole
    /// look-ahead window is `play_one_frame`'s to do, and the tiers below it are
    /// proven in `lumit_render::diskio::tests`.
    #[test]
    fn a_coming_frame_is_read_off_disk_only_when_the_read_helps() {
        // On disk, and nowhere above it: the one case that gains a read.
        assert!(wants_disk_lead(false, false, true, false));
        // On the card already: playback shows it without any of this.
        assert!(!wants_disk_lead(true, false, true, false));
        // In memory: one upload away, which is cheaper than a file.
        assert!(!wants_disk_lead(false, true, true, false));
        // Not on disk at all: there is nothing to read.
        assert!(!wants_disk_lead(false, false, false, false));
        // Asked for already: a second read of the same frame is pure IO.
        assert!(!wants_disk_lead(false, false, true, true));
    }

    /// A composition short enough to name every frame of is named exactly, and
    /// reports itself finished — there is nothing for the refinement pass to do.
    #[test]
    fn a_short_composition_is_exact_on_the_first_pass() {
        let held = [false, true, true, false, true];
        let mut asked = Vec::new();
        let sampled = sample_bar_strip(5, 1, &mut |frame| {
            asked.push(frame);
            u8::from(held[frame as usize]) * 2
        });
        assert_eq!(sampled.tiers, vec![0, 2, 2, 0, 2]);
        assert_eq!(sampled.refined_to, 5, "a stride of one leaves nothing over");
        assert_eq!(asked, vec![0, 1, 2, 3, 4], "every frame named once");
    }

    /// A long one is sampled: one frame in four is named and stands for the four.
    /// The whole stripe therefore has an answer immediately — the alternative is a
    /// bar that fills in from one end, which reads as the *cache* filling in from
    /// one end.
    #[test]
    fn a_long_composition_is_sampled_then_refined_to_the_truth() {
        // Frame 4 is the only one held, and it is not a sample point (samples are
        // 0, 4, 8, … with stride 4 — so it IS one here; use 5 instead).
        let held = |frame: u64| frame == 5;
        let tier = move |frame: u64| u8::from(held(frame)) * 2;

        let mut sampled = sample_bar_strip(12, 4, &mut { tier });
        assert_eq!(
            sampled.tiers,
            vec![0; 12],
            "frame 5 is not a sample point, so the coarse pass misses it"
        );
        assert_eq!(sampled.refined_to, 0, "and the strip needs refining");

        // Two turns of four frames each: the truth appears where it belongs, and
        // only there.
        let refined = refine_bar_strip(&mut sampled.tiers, 0, 0, 4, &mut { tier });
        assert_eq!(refined, 4);
        assert_eq!(sampled.tiers, vec![0; 12], "the first four hold nothing");
        let refined = refine_bar_strip(&mut sampled.tiers, 0, refined, 4, &mut { tier });
        assert_eq!(refined, 8);
        assert_eq!(
            sampled.tiers,
            vec![0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0],
            "frame 5 alone, not the run of four it sits in"
        );

        // And the sweep finishes rather than running past the end.
        let refined = refine_bar_strip(&mut sampled.tiers, 0, refined, 100, &mut { tier });
        assert_eq!(refined, 12);
    }

    /// A held sample paints the frames it stands for, so a warm span reads warm
    /// straight away — the coarse pass's whole purpose.
    #[test]
    fn a_held_sample_stands_for_the_frames_it_skipped() {
        let sampled = sample_bar_strip(8, 4, &mut |frame| if frame == 0 { 2 } else { 0 });
        assert_eq!(sampled.tiers, vec![2, 2, 2, 2, 0, 0, 0, 0]);
    }

    /// **The refinement starts where the user is looking.** It sweeps from the
    /// anchor and wraps, so on a long composition the region under the playhead
    /// firms up in the first turn rather than after a sweep of everything before
    /// it.
    #[test]
    fn the_refinement_sweep_starts_at_the_anchor_and_wraps() {
        let mut asked = Vec::new();
        let mut tiers = vec![0u8; 10];
        let refined = refine_bar_strip(&mut tiers, 8, 0, 4, &mut |frame| {
            asked.push(frame);
            0
        });
        assert_eq!(
            asked,
            vec![8, 9, 0, 1],
            "from the anchor, wrapping past the end"
        );
        assert_eq!(refined, 4);

        // Picking up where it left off, still relative to the anchor.
        asked.clear();
        refine_bar_strip(&mut tiers, 8, refined, 3, &mut |frame| {
            asked.push(frame);
            0
        });
        assert_eq!(asked, vec![2, 3, 4]);
    }

    /// Refinement overwrites a coarse guess with the truth, including downwards:
    /// a frame the coarse pass painted green because its sample was held reads as
    /// nothing once it is named itself.
    #[test]
    fn refinement_corrects_the_coarse_guess_in_both_directions() {
        let mut tiers = vec![2u8, 2, 2, 2];
        refine_bar_strip(&mut tiers, 0, 0, 4, &mut |frame| {
            if frame == 1 {
                4
            } else {
                0
            }
        });
        assert_eq!(tiers, vec![0, 4, 0, 0]);
    }

    /// Degenerate spans do nothing rather than panicking — an empty composition
    /// and a zero-length turn both reach here from ordinary interface states.
    #[test]
    fn empty_strips_and_empty_turns_are_calm() {
        let mut none: Vec<u8> = Vec::new();
        assert_eq!(refine_bar_strip(&mut none, 0, 0, 8, &mut |_| 2), 0);
        let mut some = vec![0u8; 4];
        assert_eq!(refine_bar_strip(&mut some, 0, 0, 0, &mut |_| 2), 0);
        assert_eq!(some, vec![0; 4]);
        assert!(sample_bar_strip(0, 4, &mut |_| 2).tiers.is_empty());
        // A stride of zero would divide by nothing; it is floored to one.
        assert_eq!(sample_bar_strip(2, 0, &mut |_| 2).tiers, vec![2, 2]);
    }
}
