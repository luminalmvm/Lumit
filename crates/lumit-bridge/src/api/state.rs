use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, LazyLock, Mutex, RwLock},
};

use flutter_rust_bridge::frb;
use lumit_core::{store::DocumentChange, Document, DocumentStore};
use lumit_project::JournalFile;
use uuid::Uuid;

use crate::{
    api::{
        composition::CompositionReference, layer::LayerReference, project::ProjectReference,
        project_item::ItemReference, worker_thread::WorkerRequest, BridgeError,
    },
    frb_generated::StreamSink,
    media::MediaCache,
};
#[frb(ignore_all)]
pub struct LumitBridgeState {
    pub store: DocumentStore,
    pub path: Option<PathBuf>,
    /// The store revision the last save wrote (or the revision the project
    /// opened at). `is_dirty` is "the store has moved past this" — an undo
    /// after a save counts as dirty, the same answer AE gives.
    pub saved_revision: u64,
    pub(crate) media: MediaCache,
    /// Where committed ops are journalled for crash recovery.
    ///
    /// Shared with the change observer rather than owned outright, and that is
    /// not a style choice: the observer fires from inside `commit`, while the
    /// caller still holds this project's write lock. An observer that reached
    /// back through `PROJECTS` for the journal would take that same lock and
    /// deadlock on the first edit. Sharing an `Arc` means it needs no lookup —
    /// and a `Mutex` rather than a bare handle so recovery can re-arm it when
    /// the document changes identity.
    pub journal: SharedJournal,
    pub sender: Option<Sender<WorkerRequest>>,
    /// The project's OCIO config as the *seam* holds it (K-490): the parse and
    /// the baked output-space tables the colour reads answer from.
    ///
    /// Derived state, never stored — the document holds a path and nothing else
    /// — and rebuilt from the file by content hash, so this is a cache with no
    /// invalidation step to get wrong. The render worker keeps its own for the
    /// same file, deliberately: the renderer lives on another thread behind a
    /// request channel, and a summary read that had to wait for a frame to
    /// finish would be a panel blocked on the Viewer.
    ///
    /// A `Mutex` inside the state rather than a field the state's own write
    /// lock guards, because syncing it is what a *read* does.
    pub(crate) colour: Mutex<lumit_render::colour::ColourState>,
}

/// The journal handle the observer writes through. `None` before one is armed,
/// or after a save has made it redundant.
pub type SharedJournal = Arc<Mutex<Option<JournalFile>>>;

/// Arm a journal for `document`, if this platform gives us somewhere to put one.
#[frb(ignore)]
pub(crate) fn journal_for(document: &Document) -> SharedJournal {
    Arc::new(Mutex::new(JournalFile::for_document(document.id)))
}

#[frb(non_opaque)]
#[derive(Debug)]
pub struct ScopedChange {
    pub project: ProjectReference,
    pub item: Option<ItemReference>,
    pub layer: Option<LayerReference>,
    /// The project item list changed: an item was added, removed, renamed,
    /// refiled or relinked. The Project panel rebuilds on this and ignores
    /// everything else, so tweaking a layer value no longer re-probes every
    /// footage file on disk.
    pub items: bool,
}

#[frb(non_opaque)]
#[derive(Clone)]
pub struct BridgeSharedFrameInfoLinux {
    pub fd: i32,
    /// Which frame of the composition this is. The frontend does not track this
    /// itself: a picture that says which frame it is needs no bookkeeping to
    /// place — the frontend paints it and moves the playhead there.
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub offset: u32,
    /// The DRM fourcc (`DRM_FORMAT_ABGR8888`, memory order R,G,B,A).
    pub drm_fourcc: u32,
    /// The DRM modifier (`DRM_FORMAT_MOD_LINEAR` = 0 on the linear-tiling path).
    pub modifier: u64,
    /// The preview tier this frame was made at: 1 Full, 2 Half, 3 Third,
    /// 4 Quarter.
    ///
    /// Carried on the frame in place of being asked for. Two Viewer widgets
    /// showed the tier, and each one asked the engine for it in its `build()` —
    /// two calls across the boundary for each frame of playback, for a number
    /// that only changes when a frame is made. The frame that changes it now
    /// brings it.
    pub tier: u32,
}

/// The Windows zero-copy Viewer frame (K-177): an NT handle to a shared D3D12
/// texture the Flutter runner imports directly, so no pixels cross the FFI
/// boundary. The handle is stable for the session and changes only when the
/// comp's dimensions do. The format is always RGBA8, so it is not carried.
#[frb(non_opaque)]
#[derive(Clone)]
pub struct BridgeSharedFrameInfo {
    /// The NT `HANDLE` value. `u64` because a Windows handle is 64-bit; it
    /// reaches Dart as a `BigInt`.
    pub handle: u64,
    /// Which frame of the composition this is. The frontend does not track this
    /// itself: a picture that says which frame it is needs no bookkeeping to
    /// place — the frontend paints it and moves the playhead there.
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    /// The preview tier this frame was made at: 1 Full, 2 Half, 3 Third,
    /// 4 Quarter.
    ///
    /// Carried on the frame in place of being asked for. Two Viewer widgets
    /// showed the tier, and each one asked the engine for it in its `build()` —
    /// two calls across the boundary for each frame of playback, for a number
    /// that only changes when a frame is made. The frame that changes it now
    /// brings it.
    pub tier: u32,
}

/// A small still picture as plain pixels — the thumbnail payload
/// (`FootageReference::thumbnail`). **Not a Viewer transport**: the read-back
/// frame path was deleted in K-183, so the only pixel payloads that cross the
/// bridge are these thumbnails, the scope traces and the dropper's windows,
/// each small by construction.
#[frb(non_opaque)]
#[derive(Clone)]
pub struct BridgeRenderedFrame {
    /// Which frame of the source this is (0 for a thumbnail's poster frame).
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    /// Tightly packed, straight (non-premultiplied) RGBA8: `width * height * 4`.
    pub rgba: Vec<u8>,
}

/// One scope trace: a fixed 256x256 RGBA picture the Scopes panel draws.
///
/// The one place pixels still cross the boundary, and small enough not to
/// matter — 256 KiB against a 1080p frame's 8 MiB. Viewer frames themselves
/// only ever cross as GPU handles (K-183): flutter_rust_bridge's SSE codec
/// serialises a `Vec<u8>` one byte at a time, measured at 8.8 ms for a 1080p
/// frame, which is why the read-back frame transport was deleted.
#[frb(non_opaque)]
#[derive(Clone)]
pub struct BridgeScopeTrace {
    /// The trace this picture *is*, echoed back from the request: 0 waveform,
    /// 1 parade, 2 vectorscope, 3 histogram. Two panels may want traces at
    /// once — the Scopes panel and the Levels row's histogram (K-413) — and
    /// they share one response stream, so each has to be able to tell whether
    /// the picture that just arrived is the one it asked for.
    pub kind: u32,
    pub rgba: Vec<u8>,
}

/// The pixels under the dropper: a square window of the picture, centred on the
/// point the pointer was over when it was asked for (docs/07 §6.1).
///
/// **A window rather than the nine pixels the magnifier shows**, so the pointer
/// can move without asking again: the frontend cuts the magnifier's grid out of
/// what it already has, and re-reads only when the pointer approaches the edge
/// of it. That turns a sweep across the picture from a request per mouse move
/// into a handful.
///
/// Small by construction — 129×129 is 66 KiB, against a 1080p frame's 8 MiB —
/// so it crosses the boundary as plain pixels without breaking the K-183 rule
/// that *frames* only ever cross as GPU handles. It is the answer to a question
/// about a few pixels, not a picture to display.
#[frb(non_opaque)]
#[derive(Clone)]
pub struct BridgeSampledPixels {
    /// The window's side length in pixels: `window × window`, always odd, so
    /// there is a single centre pixel.
    pub window: u32,
    /// Tightly packed display-ready sRGB RGBA8, `window * window * 4`,
    /// row-major from the top-left of the window. Edge pixels repeat where the
    /// window runs off the picture, so it is always exactly this size and can
    /// be indexed without a border case.
    pub rgba: Vec<u8>,
    /// The raster the window was taken from, and where in it the centre pixel
    /// sits — which is what says where in the picture the window lies.
    ///
    /// **This raster, not the composition's.** The picture read may be a
    /// reduced-resolution preview, so these are the only coordinates in which
    /// the window can be indexed; a caller holding composition pixels must map
    /// through `width`/`height` rather than assume they line up.
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    /// Which frame this is of, so a window that arrives after the playhead has
    /// moved on can be recognised as stale rather than drawn.
    pub frame: u64,
    /// True when the window is of one layer rendered alone rather than of the
    /// composite — a depth pass being read for a focal point, say.
    pub layer_alone: bool,
}

/// Where the Viewer cuts a layer's effect stack short — the "at effect" chip's
/// point (K-524, superseding K-486's thumbnail seam).
///
/// **In plain terms.** Picking an effect and turning the chip on shows the
/// composition rendered with that layer's stack stopping after the picked
/// effect and nothing past it. The point is named by the layer it is on (which
/// carries its composition) and the effect instance it stops after, which is
/// everything the engine needs to shorten the stack.
///
/// It rides the ordinary render request, so the picture comes back down the one
/// frame transport at the Viewer's own quality — this names a *way of looking*,
/// not a second viewport.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgePrefixPoint {
    /// The layer whose stack is cut, and — through its `comp_id` — the
    /// composition the cut belongs to. A point naming another composition's
    /// layer cuts nothing, so a stale chip is harmless rather than wrong.
    pub layer: crate::api::layer::LayerReference,
    /// The effect instance the stack stops **after**. An effect the layer no
    /// longer carries cuts nothing, for the same reason.
    pub effect: Uuid,
}

/// How far the frame the user is waiting for has got (docs/13 §7.1).
///
/// Sent only for a frame somebody is *waiting on* — a scrub, a value drag, a
/// playhead move — and never during playback, where a frame due in 16 ms has
/// neither the need for a bar nor the time to describe itself. A frame served
/// from the cache reports nothing at all, because there was nothing to wait
/// for: it simply arrives.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeRenderProgress {
    /// Which frame this is about, so a report that arrives after the playhead
    /// has moved on can be recognised as stale rather than drawn.
    pub frame: u64,
    /// The stage's wire code — 0 planning, 1 decoding, 2 building, 3
    /// compositing, 4 presenting ([`lumit_render::RenderStage::code`]).
    pub stage: u32,
    /// How much of the whole frame is done, 0..=1. An estimate built from
    /// fixed stage weights, which is what a progress bar needs and all it can
    /// honestly claim.
    pub fraction: f64,
    /// True on the last report of a frame — the render is finished (or was
    /// abandoned) and the bar should go. Sent by the worker rather than the
    /// engine, so a frame that failed still ends its own bar.
    pub done: bool,
}

/// One effect's measured cost within its layer, in milliseconds.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeEffectTiming {
    /// The effect *instance* id, as a string — the row in the layer's stack.
    pub effect: String,
    pub ms: f64,
}

/// One layer's measured cost for the frame just made.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLayerTiming {
    pub layer: String,
    /// The layer's own picture: its source (a Precomp's whole comp included)
    /// and its effect stack. The final composite is one pass over the whole
    /// stack rather than a per-layer act, so it lands in `total_ms` and on no
    /// row — see `lumit_render::profile`.
    pub ms: f64,
    pub effects: Vec<BridgeEffectTiming>,
}

/// What one measured frame cost, per layer and per effect — the Timeline's
/// render-time column and the Effect controls panel's readouts (docs/13 §7.1).
///
/// Published only while the frontend has asked to be measuring
/// (`set_render_profiling`), because measuring is not free: it fences the
/// graphics card at each node so a millisecond means the work rather than the
/// paperwork.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFrameProfile {
    pub frame: u64,
    /// The whole frame, wall-clock, including the stages no layer owns.
    pub total_ms: f64,
    /// The composition's top-level layers, bottom-most first.
    pub layers: Vec<BridgeLayerTiming>,
}

/// What the render worker publishes for one frame. Which frame variant a build
/// can actually produce is decided at compile time by the zero-copy features —
/// see `worker_thread::publish_frame` — but both are always declared, so the
/// generated Dart is identical on every platform and the Viewer holds one
/// `switch` over the lot.
#[frb(non_opaque)]
#[derive(Clone)]
pub enum WorkerResponse {
    /// Linux, `shared-texture-linux`.
    RenderedDMABuf(BridgeSharedFrameInfoLinux),
    /// Windows, `shared-texture`.
    RenderedSharedTexture(BridgeSharedFrameInfo),
    /// A scope trace, which rides the same stream as the frames so the panel
    /// needs no second channel.
    Scope(BridgeScopeTrace),
    /// The pixels under the dropper — the answer to one
    /// `CompositionReference::sample_pixels`, riding the same stream for the
    /// same reason a trace does.
    Sampled(BridgeSampledPixels),
    /// Playback finished on its own — it ran off the end of the composition.
    ///
    /// Sent so the transport can show itself stopped without the frontend having
    /// to work out where the end was. Stopping *because the user asked* needs no
    /// message: the frontend already knows, having asked.
    PlaybackEnded,
    /// The idle fill banked another frame (docs/06 §5.5), so the Timeline's
    /// cache bar has something new to draw. Carries nothing: the bar re-asks
    /// `cached_frames` itself. Without this the fill worked invisibly — the
    /// bar only redrew when a frame arrived, and a fill shows no frame.
    CacheFilled,
    /// How far the frame being waited for has got — the Viewer's preview
    /// progress bar (docs/07 §2.5).
    RenderProgress(BridgeRenderProgress),
    /// What the frame just made cost, layer by layer and effect by effect —
    /// the render-time indicators (docs/13 §7.1).
    FrameProfile(BridgeFrameProfile),
}

pub(crate) type CallbackStream = StreamSink<ScopedChange>;

pub type WorkerResponseStream = StreamSink<WorkerResponse>;

// The open projects, and the change stream each one publishes to.
//
// Two registries rather than one struct, because the change observer needs the
// stream *while a project's own lock is held* — it fires from inside `commit`.
// Keeping them apart is what lets it reach the stream without touching the lock
// the committing thread already has.
//
// **The lock order is a rule, not a coincidence**, and frb calls run on a worker
// pool so two threads really can be in here at once:
//
//   1. `PROJECTS` or `STREAMS` — the registries. Take what you need, clone the
//      `Arc` out, and *drop the guard*. Never hold one across step 2, and never
//      hold both at once.
//   2. One project's `RwLock`. Held across a commit.
//   3. Inside the observer, from within step 2: `STREAMS` for the sink, and the
//      project's journal `Mutex`. Both are leaves — nothing taken here may ever
//      reach back for a project lock.
//
// Anything that takes these in another order can deadlock against an ordinary
// edit. `new_project` and `open_project` disagreed about steps 1 and 2 until
// this was written down.
pub static PROJECTS: LazyLock<RwLock<BTreeMap<Uuid, Arc<RwLock<LumitBridgeState>>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

pub static STREAMS: LazyLock<RwLock<BTreeMap<Uuid, Arc<CallbackStream>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Forget every change-stream sink but `keep`'s — the `STREAMS` half of the
/// wholesale close [`LumitBridgeState::open_project`] performs, the shape
/// `ProjectReference::close` has for one project. Without it, opening projects
/// all day leaves one live sink per project the process ever had.
///
/// Its own function so a test can run it without `open_project`'s registry
/// clear, which no test may do: the registries are process-wide and the suite
/// runs in parallel. Takes one registry, holds no other lock.
pub(crate) fn forget_streams_except(keep: Uuid) -> Result<(), BridgeError> {
    let mut streams = STREAMS.write().map_err(|_| BridgeError::WriteFailed)?;
    streams.retain(|held, _| *held == keep);
    Ok(())
}

/// The scope of one op: which composition it touches, which layer within it,
/// and whether it changed the project item list.
///
/// Matching the enum rather than sniffing a JSON blob for `comp`/`layer` string
/// fields is the whole point: every project-level op used to fall through with
/// nothing set, so the Project panel had no way to tell "an item was added" from
/// "someone nudged an opacity keyframe" and rebuilt on both.
///
/// Structural layer ops (add / remove / reorder) report the comp but not the
/// layer: what changed is the comp's layer list, not one layer's contents.
pub(crate) fn op_scope(op: &lumit_core::Op) -> (Option<Uuid>, Option<Uuid>, bool) {
    use lumit_core::Op;
    match op {
        // The project item list itself.
        Op::AddItem { .. }
        | Op::RemoveItem { .. }
        | Op::RenameItem { .. }
        // A colour tag tints the item's row icon and feeds the panel's filter
        // chips, so the panel has to hear about it (K-451).
        | Op::SetItemLabel { .. }
        | Op::SetMediaRef { .. }
        // A proxy is a second media reference on a footage item (K-501), and
        // all three of these change what the item's row says about itself —
        // whether it has a stand-in, whether it is being used, and the
        // project-wide switch that governs every row at once. All of them also
        // rename every frame that reads the item, so the panel and the cache
        // bar both have to hear.
        | Op::SetItemProxy { .. }
        | Op::SetItemUseProxy { .. }
        | Op::SetUseProxies { .. }
        | Op::SetFolderChildren { .. }
        | Op::SetAutoFolder { .. }
        // Where this project parks its frames. No panel draws it — Settings
        // reads it directly — but it is a document change like any other, so it
        // belongs in the item scope rather than in a silent default.
        | Op::SetCacheLocation { .. }
        // How hard the renderer works at the edges (K-274). No panel draws it
        // either — Settings reads it directly — but it is a document change,
        // and one that renames every frame of every comp, so it must be
        // reported rather than fall through silently.
        | Op::SetAntiAliasing { .. }
        // Which OCIO config the project's colour names come from, and what one
        // footage item arrives as (K-490). Both rename frames — the config
        // every frame of every comp, the assignment every frame reading that
        // item — and the item's row will name its space, so both report.
        | Op::SetColourConfig { .. }
        | Op::SetFootageColourSpace { .. }
        // A solid def is a project item, and its name shows in the panel.
        | Op::SetSolidDef { .. } => (None, None, true),

        // Comp settings carry the comp's name, so the panel row changes too.
        Op::SetCompSettings { comp, .. } => (Some(*comp), None, true),

        // The comp, but no one layer.
        Op::AddLayer { comp, .. }
        | Op::RemoveLayer { comp, .. }
        | Op::ReorderLayer { comp, .. }
        | Op::SetCompMotionBlur { comp, .. }
        | Op::SetCompBackground { comp, .. }
        | Op::SetWorkArea { comp, .. }
        | Op::SetCompMarkers { comp, .. }
        // A layer that becomes an adjustment starts acting on everything
        // beneath it, and one that stops leaves those layers alone again — so
        // the comp is the honest scope, not the one row whose kind changed.
        | Op::SetLayerKind { comp, .. } => (Some(*comp), None, false),

        // One layer's own contents.
        Op::SetLayerSpan { comp, layer, .. }
        | Op::SetLayerMarkers { comp, layer, .. }
        | Op::RenameLayer { comp, layer, .. }
        | Op::SetLayerMasks { comp, layer, .. }
        | Op::SetLayerPaint { comp, layer, .. }
        | Op::SetShapeContents { comp, layer, .. }
        | Op::SetLayerEffects { comp, layer, .. }
        | Op::SetLayerGraph { comp, layer, .. }
        | Op::SetLayerFx { comp, layer, .. }
        | Op::SetLayerThreeD { comp, layer, .. }
        | Op::SetSequenceClips { comp, layer, .. }
        | Op::SetLayerAudible { comp, layer, .. }
        | Op::SetLayerVisible { comp, layer, .. }
        | Op::SetLayerSolo { comp, layer, .. }
        | Op::SetLayerMotionBlur { comp, layer, .. }
        | Op::SetLayerAcceptsLights { comp, layer, .. }
        | Op::SetLayerShy { comp, layer, .. }
        | Op::SetLayerGuide { comp, layer, .. }
        | Op::SetLayerLocked { comp, layer, .. }
        | Op::SetLayerLabel { comp, layer, .. }
        | Op::SetLayerCollapse { comp, layer, .. }
        | Op::SetTextDocument { comp, layer, .. }
        | Op::SetLayerBlend { comp, layer, .. }
        | Op::SetLayerMatte { comp, layer, .. }
        | Op::SetLayerParent { comp, layer, .. }
        | Op::SetTransformProperty { comp, layer, .. }
        | Op::SetCameraZoom { comp, layer, .. }
        | Op::SetCameraSolveLink { comp, layer, .. }
        | Op::SetLayerVolume { comp, layer, .. }
        | Op::SetLayerInterpolation { comp, layer, .. }
        | Op::SetRetimeProperty { comp, layer, .. } => (Some(*comp), Some(*layer), false),

        // A batch is as broad as its members: the item flag is the union, and
        // the reference scope widens to "no one subtree" rather than picking a
        // member's comp and leaving the others unrefreshed.
        Op::Batch { ops } => (None, None, ops.iter().any(|o| op_scope(o).2)),
    }
}

impl LumitBridgeState {
    #[frb(sync)]
    pub fn new_project(
        on_change_stream: Option<CallbackStream>,
    ) -> Result<ProjectReference, BridgeError> {
        let id = Uuid::now_v7();

        // The stream first, and its guard dropped before the registry is
        // touched — see the lock-order note above. Registering it before the
        // project exists is safe: the observer cannot fire until the store has
        // one, and the store is built below.
        if let Some(stream) = on_change_stream {
            let mut s = STREAMS.write().map_err(|_| BridgeError::WriteFailed)?;
            s.insert(id, Arc::new(stream));
        }

        let document = Document::new();
        let journal = journal_for(&document);
        let store = DocumentStore::new(document);
        let state = LumitBridgeState {
            saved_revision: store.revision(),
            store,
            path: None,
            media: MediaCache::default(),
            journal: Arc::clone(&journal),
            sender: None,
            colour: Mutex::new(lumit_render::colour::ColourState::default()),
        };

        state.store.set_callback(Arc::new(move |c| {
            Self::handle_change_callback(c, id, &journal)
        }));

        PROJECTS
            .write()
            .map_err(|_| BridgeError::WriteFailed)?
            .insert(id, Arc::new(RwLock::new(state)));

        Ok(ProjectReference::new(id))
    }

    #[frb(sync)]
    pub fn get_current_project() -> Result<Option<ProjectReference>, BridgeError> {
        let p = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;

        Ok(p.keys().next().map(|id| ProjectReference::new(*id)))
    }

    /// Turn a committed op into the narrowest scope Dart can rebuild from.
    ///
    /// This runs inside `DocumentStore`'s observer, which returns nothing, so
    /// there is no caller to hand an error to. It therefore cannot fail: the
    /// scope comes from matching the `Op` enum, so a new variant is a compile
    /// error here rather than a silently unscoped change at runtime.
    fn handle_change_callback(
        document_change: DocumentChange,
        project_id: Uuid,
        journal: &SharedJournal,
    ) {
        // Journal first, then tell the interface. A crash between the two loses
        // the redraw, which the next one fixes; a crash the other way round
        // loses the edit, which nothing does.
        if let Ok(journal) = journal.lock() {
            if let Some(journal) = journal.as_ref() {
                // A journal that cannot be written is not worth taking the
                // editor down for — the work is still in the document, and the
                // next save writes it properly.
                let _ = journal.append(&document_change.op);
            }
        }

        let (comp, layer, items) = op_scope(&document_change.op);

        // **Nothing is invalidated here, and that is the point (K-178).** This
        // used to drop every held frame of every composition on every committed
        // op, because frames were filed by position: the edit did not change any
        // frame's *name*, so the only safe answer was to throw them all away.
        // The cost was paid on edits that cannot change a pixel — a rename, a
        // work-area nudge, a solo toggle, sound added to a layer — and the cache
        // bar went blank with each one.
        //
        // Frames are now filed under a hash of what is in them (docs/06 §5.2),
        // so an edit renames exactly the frames it changed and every other frame
        // stays addressable. An undo asks for the names it asked for before and
        // finds them still held. There is no invalidation step left to get right.

        let change = ScopedChange {
            project: ProjectReference::new(project_id),
            item: comp
                .map(|c| ItemReference::Composition(CompositionReference::new(project_id, c))),
            layer: comp
                .zip(layer)
                .map(|(c, l)| LayerReference::new(project_id, c, l)),
            items,
        };

        let Ok(streams) = STREAMS.read() else {
            eprintln!("Stream registry poisoned; dropping change for {project_id}");
            return;
        };

        if let Some(stream) = streams.get(&project_id) {
            _ = stream.add(change);
        }
    }

    /// Deliberately **not** `#[frb(sync)]`, unlike its `new_project` sibling:
    /// reading a `.lum` parses a whole document and stats every media file it
    /// names, and on the UI isolate that froze the window for as long as it
    /// took. Async puts it on a worker thread, which is what lets Dart hold the
    /// previous document on screen behind a progress bar until this returns.
    pub fn open_project(
        path: &str,
        on_change_stream: Option<CallbackStream>,
    ) -> Result<Option<ProjectReference>, BridgeError> {
        let path = PathBuf::from(path);
        let Ok((doc, _manifest)) = lumit_project::open(&path) else {
            // Not an error to report: a `.lum` that will not open is the file
            // picker's problem, and Dart shows its own notice for None.
            return Ok(None);
        };

        // Relative media paths resolve against the project's own directory. A
        // path with no parent (a bare filename) resolves against the working
        // directory, which `Path::new("")` gives us — nothing to relink from,
        // rather than a panic.
        let project_dir = path.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
        let (project, _missing) = adopt(doc, Some(path), &project_dir, on_change_stream)?;
        Ok(Some(project))
    }
}

/// Make `doc` the open project: relink its media, hand every project already
/// open its farewell, and register this one in its place.
///
/// **The one road into the registry for a document that came from outside**,
/// shared by [`LumitBridgeState::open_project`] and by the After Effects import
/// (`api::import`). Import is an open — a whole new document replacing whatever
/// was loaded — and the half of an open that is easy to leave out is not the
/// insert but the *departure*: every displaced project's media cache, its
/// change sink, and above all its render worker, which holds a whole GPU device
/// and never stops on its own.
///
/// `saved_at` is where the document lives on disk, or `None` for one that has
/// no file yet — an import, whose project is unsaved until somebody chooses a
/// name for it. `media_root` is what relative media paths resolve against.
///
/// Answers the adopted project and **the names of the footage items whose file
/// was not found**, which is not an error and never stops an adoption: those
/// items are offline, which is a thing a project is allowed to be (docs/11
/// §2.5). Opening a `.lum` ignores the list — the Project panel already draws a
/// relink slate on each of them — while the importer turns it into report rows,
/// because an import is exactly the moment somebody wants to be told.
pub(crate) fn adopt(
    mut doc: Document,
    saved_at: Option<PathBuf>,
    media_root: &Path,
    on_change_stream: Option<CallbackStream>,
) -> Result<(ProjectReference, Vec<String>), BridgeError> {
    let id = Uuid::now_v7();

    let (_relinked, missing) = lumit_project::resolve_all_media(&mut doc, media_root, &[]);

    // Every footage file this project holds, handed to the probe worker
    // before a panel has had a chance to ask about any of them. Opening a
    // project is exactly when a Project panel full of rows is about to ask
    // each of its items what it is, and reading them in the background is
    // the difference between a list that fills and one that appears.
    // Queued after `resolve_all_media`, so the paths are the resolved ones,
    // and before the registry lock is taken (docs/14 §3).
    let warm: Vec<PathBuf> = doc
        .items
        .iter()
        .filter_map(|item| match item {
            lumit_core::model::ProjectItem::Footage(f) if !f.media.absolute_path.is_empty() => {
                Some(PathBuf::from(&f.media.absolute_path))
            }
            _ => None,
        })
        .collect();

    let journal = journal_for(&doc);
    let store = DocumentStore::new(doc);
    let state = LumitBridgeState {
        saved_revision: store.revision(),
        store,
        path: saved_at,
        media: MediaCache::default(),
        journal: Arc::clone(&journal),
        sender: None,
        colour: Mutex::new(lumit_render::colour::ColourState::default()),
    };
    state.store.set_callback(Arc::new(move |c| {
        LumitBridgeState::handle_change_callback(c, id, &journal)
    }));

    if let Some(stream) = on_change_stream {
        let mut s = STREAMS.write().map_err(|_| BridgeError::WriteFailed)?;
        s.insert(id, Arc::new(stream));
    }

    {
        let mut p = PROJECTS.write().map_err(|_| BridgeError::WriteFailed)?;

        for entry in p.values() {
            // A project whose lock is poisoned is being discarded anyway,
            // so a failed cache clear is not worth refusing the open over.
            if let Ok(mut e) = entry.write() {
                e.media.clear();
            }
        }
        // The waveform summaries are keyed by file path and shared between
        // projects, so they are not any one project's to clear — but the
        // project being closed is the reason they were built (K-280). The
        // probe answers are shared the same way and go for the same reason,
        // and clearing them also cancels whatever the probe worker still
        // had queued for the project that is closing.
        crate::peaks::clear();
        crate::probe::clear();
        // And any beat detection queued for the project that is going: its
        // audio is about to stop being anybody's audio.
        crate::beats::clear();

        // Clear any other project that is currently open
        // Will also prevent any existing references from working
        p.clear();

        p.insert(id, Arc::new(RwLock::new(state)));
    }

    // The displaced projects' change sinks go too — a forgotten project
    // with a live sink is a leak, and one registry at a time means after
    // the `PROJECTS` guard above has been dropped, not inside it.
    forget_streams_except(id)?;

    // After the clear, so this project's requests are not the ones
    // cancelled, and outside the registry lock.
    for file in &warm {
        crate::probe::request(file);
    }

    Ok((ProjectReference::new(id), missing))
}
