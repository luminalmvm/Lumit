//! Application state behind the shell: the document store, project path,
//! journal, dirty tracking, autosave. Slice 3 of docs/impl/phase-0-kickoff.md.

use lumit_core::model::{Composition, Document, FootageItem, LinearColour, MediaRef, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_core::DocumentStore;
use lumit_project::JournalFile;
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

pub const AUTOSAVE_INTERVAL_SECS: u64 = 300;
pub const AUTOSAVE_KEEP: usize = 5;

// `impl AppState` and the module's tests are split across the sibling files
// below for readability; behaviour is unchanged.
mod compositions;
mod layers;
mod playback;
mod previewing;
mod project;

/// Latest-wins background frame decoding for the Viewer (slice 5).
/// In plain terms: the UI sends "show frame N of item X" requests down a
/// channel; a worker thread owns the decoders and answers with pixels; stale
/// requests are simply skipped (the epoch/latest-wins idea from
/// docs/impl/playback-scheduler.md, in miniature).
#[cfg(feature = "media")]
pub mod preview;

/// Probe/index results for footage items, filled by background threads.
#[cfg(feature = "media")]
pub mod media;

/// While the user is actively scrubbing or dragging, footage decodes at most
/// this wide so a frame comes back fast (the specified resolution reloads the
/// moment they stop). Chosen to keep even 4K sources instant to draft.
const DRAFT_MAX_WIDTH: u32 = 640;

/// Infallible constructor for small literal rationals.
/// One decode-width policy for requests AND cache keys — if these ever
/// disagreed, a cached frame could present at the wrong resolution. `draft`
/// caps the width for instant feedback and never exceeds the specified tier.
fn decode_target_width(
    natural_w: u32,
    draft: bool,
    auto_res: bool,
    display_scale: f32,
    divisor: u32,
) -> Option<u32> {
    let specified = if auto_res {
        let scale = display_scale.clamp(0.05, 1.0);
        let w = (natural_w as f32 * scale).round() as u32;
        (w < natural_w).then_some(w.max(16))
    } else {
        (divisor > 1).then(|| natural_w / divisor)
    };
    if draft {
        // Never coarser than needed: cap the specified width, never raise it.
        let w = specified.unwrap_or(natural_w).min(DRAFT_MAX_WIDTH);
        return (w < natural_w).then_some(w.max(16));
    }
    specified
}

/// Frame visit order for the idle background cache fill: the playhead first,
/// then a forward-biased walk — roughly three frames ahead of the playhead for
/// every one behind — because playback and scrubbing usually head forwards, so
/// the frames most likely to be viewed next should cache first (Mack). Every
/// work-area frame appears exactly once.
fn fill_walk_order(playhead: usize, start: usize, end: usize) -> Vec<usize> {
    let mut order = Vec::new();
    if end <= start || playhead < start || playhead >= end {
        return order;
    }
    let span = end - start;
    order.push(playhead);
    let (mut ahead, mut behind) = (1usize, 1usize);
    let mut k = 0usize;
    while order.len() < span && k < span * 2 + 8 {
        // One behind for every three ahead; when a side is exhausted the other
        // takes over so every frame is still visited.
        let want_behind = k % 4 == 3;
        let forward = playhead + ahead;
        if !want_behind && forward < end {
            order.push(forward);
            ahead += 1;
        } else if let Some(f) = playhead.checked_sub(behind).filter(|f| *f >= start) {
            order.push(f);
            behind += 1;
        } else if forward < end {
            order.push(forward);
            ahead += 1;
        }
        k += 1;
    }
    order
}

/// Frames to warm ahead of the playhead during playback: the bounded forward
/// window `[playhead + 1, playhead + lookahead]`, clamped to the work-area end
/// (`end` exclusive). Playback presentation chases the audio clock, so warming
/// a little ahead of it keeps the work-area loop smooth once frames are cached
/// (docs/impl/playback-scheduler.md §5). Empty once the playhead reaches the end.
fn playback_lookahead(playhead: usize, end: usize, lookahead: usize) -> Vec<usize> {
    let first = playhead.saturating_add(1);
    let stop = first.saturating_add(lookahead).min(end);
    (first..stop).collect()
}

/// Pan-behind: the position that keeps a layer visually fixed when its origin
/// (anchor) moves from `anchor` to `new_anchor`. Position places the anchor in
/// comp space, so shifting the anchor by Δ in layer space must shift position
/// by the layer's scale·rotation applied to Δ (docs/01-GLOSSARY.md anchor).
pub fn pan_behind_position(
    anchor: (f64, f64),
    new_anchor: (f64, f64),
    position: (f64, f64),
    scale_pct: (f64, f64),
    rotation_deg: f64,
) -> (f64, f64) {
    let vx = (new_anchor.0 - anchor.0) * scale_pct.0 / 100.0;
    let vy = (new_anchor.1 - anchor.1) * scale_pct.1 / 100.0;
    let (sin, cos) = rotation_deg.to_radians().sin_cos();
    (
        position.0 + vx * cos - vy * sin,
        position.1 + vx * sin + vy * cos,
    )
}

/// Merge pasted keyframes into an existing list, OVERWRITING any existing key
/// whose time is within `tol` of a pasted key (note 2.2), then keeping the list
/// sorted and unique by time — the pasted key wins the collision (it is inserted
/// after the survivors are filtered). Pure; the paste commit and its test share
/// it.
pub(crate) fn merge_paste_keys(
    existing: &[lumit_core::anim::Keyframe],
    pasted: &[lumit_core::anim::Keyframe],
    tol: f64,
) -> Vec<lumit_core::anim::Keyframe> {
    let mut out: Vec<lumit_core::anim::Keyframe> = existing
        .iter()
        .filter(|k| {
            !pasted
                .iter()
                .any(|p| (p.time.to_f64() - k.time.to_f64()).abs() < tol)
        })
        .copied()
        .collect();
    out.extend_from_slice(pasted);
    out.sort_by_key(|k| k.time);
    out.dedup_by(|a, b| a.time == b.time);
    out
}

/// A change-detection fingerprint of a comp's mixed audio: the ordered set of
/// contributing sources with their comp-timeline placement, plus the mix length
/// (the comp duration). Any edit that changes what the comp sounds like — mute,
/// move, trim, delete, add a source — changes this. A baked mix is kept in step
/// with the document by comparing this each frame, so a mix baked once never
/// outlives the state it was baked from (the GEN-4 audio fixes). Pure, so the
/// gating is a plain deterministic test.
#[cfg(feature = "media")]
pub(crate) fn audio_jobs_signature(jobs: &[crate::export::AudioJob], duration_s: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    jobs.len().hash(&mut h);
    duration_s.to_bits().hash(&mut h);
    for j in jobs {
        j.path.hash(&mut h);
        j.in_s.to_bits().hash(&mut h);
        j.out_s.to_bits().hash(&mut h);
        j.offset_s.to_bits().hash(&mut h);
    }
    h.finish()
}

/// What keeping the loaded comp-audio mix in step with the document needs this
/// frame, derived purely from the current jobs and what is loaded / in flight.
#[cfg(feature = "media")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioSync {
    /// The loaded mix already matches the comp: nothing to do.
    UpToDate,
    /// The comp is silent now (every audio layer muted or gone): unload it.
    Silence,
    /// The comp's audio changed: (re)bake to this signature.
    Rebake(u64),
}

/// Decide how to reconcile the loaded comp mix with the current document. When
/// the comp has no audible audio a loaded mix is unloaded ([`AudioSync::Silence`]);
/// otherwise the mix is re-baked whenever its signature no longer matches, unless
/// exactly that bake is already in flight. Pure — the four GEN-4 behaviours
/// (mute, move, span, delete) are asserted against it without a device.
#[cfg(feature = "media")]
pub(crate) fn comp_audio_sync(
    loaded_comp: Option<Uuid>,
    loaded_sig: Option<u64>,
    preparing: Option<(Uuid, u64)>,
    comp_id: Uuid,
    jobs: &[crate::export::AudioJob],
    duration_s: f64,
) -> AudioSync {
    if jobs.is_empty() {
        return if loaded_comp == Some(comp_id) {
            AudioSync::Silence
        } else {
            AudioSync::UpToDate
        };
    }
    let sig = audio_jobs_signature(jobs, duration_s);
    if loaded_comp == Some(comp_id) && loaded_sig == Some(sig) {
        return AudioSync::UpToDate;
    }
    if preparing == Some((comp_id, sig)) {
        return AudioSync::UpToDate;
    }
    AudioSync::Rebake(sig)
}

/// The Y partner of a linked pair's X channel (Anchor/Position/Scale), or None —
/// so copy carries both axes of a linked keyframe.
fn linked_axis_partner(
    p: lumit_core::model::TransformProp,
) -> Option<lumit_core::model::TransformProp> {
    use lumit_core::model::TransformProp::{
        AnchorX, AnchorY, PositionX, PositionY, ScaleX, ScaleY,
    };
    match p {
        AnchorX => Some(AnchorY),
        PositionX => Some(PositionY),
        ScaleX => Some(ScaleY),
        _ => None,
    }
}

/// A transform whose origin (anchor) is the centre of a `nat_w`×`nat_h`
/// object, placed at the centre of a `comp_w`×`comp_h` composition — the AE
/// default so a new layer appears centred and pivots about its middle.
fn centred_transform(
    nat_w: f64,
    nat_h: f64,
    comp_w: u32,
    comp_h: u32,
) -> lumit_core::model::TransformGroup {
    use lumit_core::anim::Property;
    lumit_core::model::TransformGroup {
        anchor_x: Property::fixed(nat_w * 0.5),
        anchor_y: Property::fixed(nat_h * 0.5),
        position_x: Property::fixed(f64::from(comp_w) * 0.5),
        position_y: Property::fixed(f64::from(comp_h) * 0.5),
        ..Default::default()
    }
}

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap_or(Rational::ZERO)
}

/// The composition settings dialogue (AE: Composition Settings): used both
/// for creating a comp (editing = None) and editing one later. Opened with
/// footage-matched defaults when a drop starts the project's first comp.
/// The parametric shape the shape tool draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ShapeKind {
    #[default]
    Rectangle,
    Ellipse,
    Star,
}

impl ShapeKind {
    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Star => "Star",
        }
    }
}

/// What a pointer drag/click does in the Viewer (the toolbar's mouse mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ToolMode {
    /// Click selects; drag pans (object selection arrives with the object
    /// tools — for now Select pans like Hand so the view stays navigable).
    #[default]
    Select,
    /// Drag pans the view (the hand).
    Hand,
    /// Drag rubber-bands a new mask of the current [`ShapeKind`].
    Shape,
    /// Click places mask vertices (pen).
    Pen,
}

/// What an armed eyedropper samples from the Viewer, and how it is committed
/// back to its target parameter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EyedropperMode {
    /// Sample a colour and write it to a Colour parameter's RGB channels
    /// (converted to the scene-linear values the parameter holds).
    Colour,
    /// Sample a depth proxy (the luma of the picked pixel) and write it to a
    /// Float parameter — the depth-of-field Focus pick.
    Depth,
}

/// An armed eyedropper: which effect parameter the next Viewer click writes,
/// and what it samples. Set by the inspector's eyedropper button, consumed by
/// the Viewer overlay ([`crate::shell::eyedropper`]) — a click samples and
/// commits an undoable `SetLayerEffects`, then disarms; Escape or a click
/// outside the image cancels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EyedropperTarget {
    /// The layer whose effect stack carries the parameter.
    pub layer: Uuid,
    /// Index of the effect within that layer's stack.
    pub effect: usize,
    /// Index of the parameter within that effect.
    pub param: usize,
    /// What to sample and how to commit it.
    pub mode: EyedropperMode,
}

pub struct CompDialog {
    /// Some = editing an existing comp; None = creating a new one.
    pub editing: Option<Uuid>,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_s: f64,
    /// Keep width:height fixed: editing one dimension rescales the other.
    pub lock_ratio: bool,
    /// The locked aspect (width / height), captured when the lock engages.
    pub aspect: f64,
    /// Item to add as the first layer once the comp exists (drag-drop with
    /// no comp yet).
    pub pending_item: Option<Uuid>,
    /// Comp-wide motion-blur shutter (K-120), editable when an existing comp is
    /// open; a fresh comp starts with the default (off).
    pub motion_blur: lumit_core::model::MotionBlur,
}

/// The lane guide-line mode — what the faint vertical lines mark.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimelineGrid {
    /// Detected beats and markers (suits montage and music-led work).
    Beats,
    /// The time grid: seconds, subdividing to frames as the zoom allows.
    /// The default.
    Time,
    /// No guide lines.
    Off,
}

/// Cache-bar memo key: (document snapshot ptr, cache epoch, quality tag,
/// comp id, disk-set size) — the bar is stale iff any of these moved.
#[cfg(feature = "media")]
type CacheBarKey = (usize, u64, u32, Uuid, usize);

/// A frame's cache-bar tier (docs/06 §5.6): green plays now, blue promotes.
#[cfg(feature = "media")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    None,
    /// In RAM at current quality — plays in real time now (green).
    Ram,
    /// On disk only — promotable, not yet playable (blue).
    Disk,
}

/// The disk tier's IO side (docs/06 §5.4): one background thread owns the
/// [`lumit_cache::disk::DiskCache`] so the UI thread never touches the
/// filesystem. Writes are fire-and-forget (write-behind); loads come back
/// through a channel and are folded into the RAM tier each frame. The shared
/// `known` set mirrors which hashes exist on disk, for the cache bar's blue
/// tier and the fill scheduler's promote-before-render choice.
#[cfg(feature = "media")]
pub mod diskio;

/// One display-ready comp frame in Kura's RAM tier (sRGB bytes as shown and
/// as exported — the same pixels, K-031).
#[cfg(feature = "media")]
pub struct CachedCompFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(feature = "media")]
impl lumit_cache::ByteSized for CachedCompFrame {
    fn byte_size(&self) -> usize {
        self.rgba.len() + 16
    }
}

/// See [`AppState::stamper`].
#[cfg(feature = "media")]
pub struct PreviewStamper<'a> {
    doc: &'a Document,
    media: &'a media::MediaRegistry,
    auto_res: bool,
    display_scale: f32,
    divisor: u32,
}

#[cfg(feature = "media")]
impl lumit_eval::SourceStamper for PreviewStamper<'_> {
    fn stamp(&self, item: Uuid, lt: f64) -> Option<(String, u64)> {
        let Some(ProjectItem::Footage(f)) = self.doc.item(item) else {
            return None;
        };
        let media::MediaStatus::Ready { probe, frames, .. } = self.media.map.get(&item)? else {
            return None;
        };
        let video = probe.video.as_ref()?;
        let source_frame =
            ((lt * video.fps()).round().max(0.0) as usize).min(frames.saturating_sub(1));
        // Key at the specified resolution: draft frames are never cached, so
        // the content-hash key always represents the settled resolution.
        let target = decode_target_width(
            video.width,
            false,
            self.auto_res,
            self.display_scale,
            self.divisor,
        );
        Some((
            format!("{}#w{}", f.media.absolute_path, target.unwrap_or(0)),
            source_frame as u64,
        ))
    }
}

/// A recovery offer: the saved document plus the journal ops beyond it.
pub struct PendingRecovery {
    pub doc: Document,
    pub path: PathBuf,
    pub ops: Vec<Op>,
}

/// Beat-analysis result handed back from the worker: (comp, bpm, onsets).
#[cfg(feature = "media")]
type BeatMsg = (Uuid, f64, Vec<(f64, f32)>);

/// A marquee selection in the graph editor: which keyframes of which channel
/// are selected. In plain terms: the box you drag over a curve remembers its
/// keys here, and every entry is pinned to both its index *and* the time the
/// key had when it was selected — if any other edit inserts, removes or
/// re-orders keys, the pins no longer line up and the whole selection reads
/// as stale (and clears) instead of ever editing the wrong keyframes.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphSelection {
    /// The layer whose curve the selection was made on.
    pub layer: Uuid,
    /// The transform property (channel) the indices refer to. Ignored when
    /// `retime` is set — the Retime Time channel isn't a transform property.
    pub prop: lumit_core::model::TransformProp,
    /// True when the selection was made on the footage layer's Retime Time
    /// channel (K-078) rather than a transform property, so a selection on one
    /// never leaks onto the other.
    pub retime: bool,
    /// (keyframe index, its time when selected), ascending by index.
    pub keys: Vec<(usize, Rational)>,
}

impl GraphSelection {
    /// The selected indices, if every pin still lines up with `keys`; `None`
    /// means the selection is stale (the keyframe list changed underneath).
    pub fn indices_for(&self, keys: &[lumit_core::anim::Keyframe]) -> Option<Vec<usize>> {
        self.keys
            .iter()
            .map(|&(i, t)| keys.get(i).filter(|k| k.time == t).map(|_| i))
            .collect()
    }
}

/// Which property row is highlighted in the layer/Effect Controls area (note
/// 2.8.1). Clicking anywhere on a property row selects it; the row's background
/// lifts, and if the row is an effect parameter its effect's title bar lifts too
/// (note 2.8.2). One row at a time in v1 — multi-property selection (note 2.6)
/// will grow this into a set. Shared by the Timeline and Effect Controls panels
/// so a row highlights in both (note 2.8.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PropSel {
    pub layer: Uuid,
    pub row: PropRow,
}

/// The identity of a property row within a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropRow {
    /// A transform channel (position, anchor, scale, rotation, opacity).
    Transform(lumit_core::model::TransformProp),
    /// One effect parameter: the effect's index in the stack and the param index.
    Effect { effect: usize, param: usize },
    /// The footage Retime channel — the "Time"/"Velocity" speed row (K-072/K-075).
    /// One per layer (it wears two lenses but is a single channel), so no fields.
    Retime,
}

/// A single keyframe picked out in the timeline lane/layer view (note 2.1). The
/// lane is a pure time axis, so a key is named by its property row and its own
/// (layer-local) time — never a value or a list index. Identifying keys by time
/// is what lets a marquee span several property rows at once (note 2.6) and lets
/// the linked Anchor/Position/Scale rows, whose lane shows the union of both
/// axes' keys, treat both axes' keys at one time as a single draggable point.
/// Selection is UI state only; every edit still commits through the document
/// ops, so preview equals export.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneKeySel {
    pub layer: Uuid,
    pub row: PropRow,
    pub time: Rational,
}

/// An in-flight lane keyframe drag (note 2.1): the grabbed key's identity and
/// the provisional (layer-local) time it is being dragged to — already frame
/// snapped when the magnet is on. The whole lane selection rides the same delta
/// `to − grabbed.time`; the release commits one Batch so the slide is a single
/// undo step.
#[derive(Clone, Copy, Debug)]
pub struct LaneKeyDrag {
    pub grabbed: LaneKeySel,
    pub to: f64,
}

impl LaneKeyDrag {
    /// The time shift (seconds) the drag currently applies to every selected
    /// key: the grabbed key's landing time minus its resting time.
    pub fn delta(&self) -> f64 {
        self.to - self.grabbed.time.to_f64()
    }
}

/// One drawn keyframe glyph on a lane this frame: its selection identity and its
/// screen position, gathered across every property row so the timeline's
/// marquee can hit-test keys from different rows in one pass (note 2.6). Rebuilt
/// each frame; never persisted.
#[derive(Clone, Copy, Debug)]
pub struct LaneGlyph {
    pub sel: LaneKeySel,
    pub pos: egui::Pos2,
}

/// A keyframe on the lane clipboard (note 2.2): the key itself (carrying its
/// bezier handles in `interp_in`/`interp_out`) plus the layer and property row
/// it came from and its time relative to the copy anchor (the earliest selected
/// key). Paste replays these onto their own property at the playhead, preserving
/// each key's offset and its handles and overwriting any key whose time
/// coincides. Clipboard is UI state, never the document.
#[derive(Clone, Copy, Debug)]
pub struct ClipboardKey {
    pub layer: Uuid,
    pub row: PropRow,
    /// Time of this key minus the earliest copied key's time (seconds).
    pub offset: f64,
    pub key: lumit_core::anim::Keyframe,
}

pub struct AppState {
    pub store: DocumentStore,
    pub path: Option<PathBuf>,
    journal: Option<JournalFile>,
    pub dirty: bool,
    pub selected_comp: Option<Uuid>,
    /// Item highlighted in the Project panel (any kind, not just comps). The
    /// primary of a multi-selection: the info header follows it and it anchors
    /// Ctrl-additions to [`Self::selected_items`].
    pub selected_item: Option<Uuid>,
    /// The Project panel's multi-selection (A3): Ctrl/Shift-click builds this set
    /// so several items drag into a comp at once. Empty means the selection is
    /// just [`Self::selected_item`] (the ordinary single-select case).
    pub selected_items: Vec<Uuid>,
    /// Open composition-settings dialogue, if any.
    pub comp_dialog: Option<CompDialog>,
    pub pending_recovery: Option<PendingRecovery>,
    pub error: Option<String>,
    #[cfg(feature = "media")]
    pub media: media::MediaRegistry,
    #[cfg(feature = "media")]
    pub preview_engine: preview::PreviewEngine,
    #[cfg(feature = "media")]
    audio_engine: Option<lumit_audio::AudioEngine>,
    #[cfg(feature = "media")]
    audio_cache: std::collections::HashMap<Uuid, std::sync::Arc<lumit_media::AudioBuffer>>,
    #[cfg(feature = "media")]
    audio_loaded: Option<Uuid>,
    #[cfg(feature = "media")]
    audio_rx: std::sync::mpsc::Receiver<(Uuid, Result<lumit_media::AudioBuffer, String>)>,
    #[cfg(feature = "media")]
    audio_tx: std::sync::mpsc::Sender<(Uuid, Result<lumit_media::AudioBuffer, String>)>,
    /// The comp whose mixed audio is loaded in the engine (drives its clock).
    #[cfg(feature = "media")]
    audio_loaded_comp: Option<Uuid>,
    /// Signature of the loaded comp mix (its contributing layers and their
    /// placement). When an edit — mute, move, trim, delete — changes what the
    /// comp sounds like, this stops matching the document and the mix is
    /// re-baked, so playback always follows the current comp (GEN-4 fixes).
    #[cfg(feature = "media")]
    audio_loaded_sig: Option<u64>,
    /// A comp-audio bake in flight: (comp, target signature). Stops the same
    /// bake being re-spawned every frame while it decodes.
    #[cfg(feature = "media")]
    audio_preparing: Option<(Uuid, u64)>,
    /// Background-mixed comp audio arriving from the prepare thread, tagged with
    /// the signature it was baked from so a superseded mix can be dropped.
    #[cfg(feature = "media")]
    comp_audio_rx: std::sync::mpsc::Receiver<(Uuid, u64, lumit_media::AudioBuffer)>,
    #[cfg(feature = "media")]
    comp_audio_tx: std::sync::mpsc::Sender<(Uuid, u64, lumit_media::AudioBuffer)>,
    /// Detected beats (comp id, bpm, (time_s, confidence)…) from the analysis
    /// thread.
    #[cfg(feature = "media")]
    beats_rx: std::sync::mpsc::Receiver<BeatMsg>,
    #[cfg(feature = "media")]
    beats_tx: std::sync::mpsc::Sender<BeatMsg>,
    /// (comp id, estimated BPM) from the last beat detection, shown by the ruler.
    #[cfg(feature = "media")]
    pub detected_bpm: Option<(Uuid, f64)>,
    /// (comp id, (min,max) peaks) for the timeline waveform, computed when the
    /// comp's audio is mixed. Drawn under the ruler.
    #[cfg(feature = "media")]
    pub comp_waveform: Option<(Uuid, Vec<(f32, f32)>)>,
    /// In-flight property drag (layer, property, provisional value): commits
    /// once on release so a drag is ONE undo step, not hundreds.
    pub prop_edit: Option<(Uuid, lumit_core::model::TransformProp, f64)>,
    /// In-flight effect-parameter drag (layer, effect index, param index,
    /// provisional value): the live preview re-runs the effect stack with the
    /// patched value each frame, committing once on release — the effect twin
    /// of `prop_edit`.
    pub fx_edit: Option<(Uuid, usize, usize, f64)>,
    /// In-flight *linked* scale drag (layer, x%, y%): the live preview needs
    /// both axes, since one drag moves both (else only x scales until release).
    pub scale_preview: Option<(Uuid, f64, f64)>,
    /// In-flight Retime "Time" value drag (layer, provisional retime store):
    /// unlike a transform or effect drag, changing the retime changes which
    /// *source* frame is on screen, so the live preview must re-decode with this
    /// store (the decode job builder overrides the layer's retime with it)
    /// rather than re-composite the already-decoded frame. Commits once on
    /// release, like every other value drag.
    pub retime_edit: Option<(Uuid, lumit_core::retime::Retime)>,
    /// In-flight bar-edge trim: (layer, trimming_out_edge, provisional seconds).
    pub trim_edit: Option<(Uuid, bool, f64)>,
    /// In-flight whole-layer move (drag the bar body): (layer, provisional new
    /// in-point in comp seconds, unsnapped). On release the whole span shifts —
    /// in/out/start_offset together — so the content moves with the bar.
    pub move_edit: Option<(Uuid, f64)>,
    /// Layer whose properties the graph editor shows (clicked in the Timeline).
    pub selected_layer: Option<Uuid>,
    /// Selected clip within a Sequence layer (clicked sub-bar), for per-clip
    /// speed editing.
    pub selected_clip: Option<Uuid>,
    /// Mask vertex mid-drag in the Viewer: (mask index, vertex index,
    /// layer-space position). Committed as one SetLayerMasks op on release.
    pub mask_drag: Option<(usize, usize, (f64, f64))>,
    /// The active pointer tool (toolbar): what a Viewer drag/click does.
    pub tool: ToolMode,
    /// An armed eyedropper (colour or depth pick from the Viewer), or None.
    /// While Some, the Viewer shows a magnifier and the next click over the
    /// image samples and commits to this parameter (see [`EyedropperTarget`]).
    pub eyedropper: Option<EyedropperTarget>,
    /// The eyedropper's averaging region side length in image pixels: 1 (a
    /// single pixel), 2, 3, … Shift+scroll over the Viewer grows it so a wider
    /// sample beats grain; the committed value is the average over the region.
    /// Reset to 1 each time the eyedropper is armed.
    pub eyedropper_region: u32,
    /// False on the frame the eyedropper is armed, true afterwards: keeps the
    /// same click that armed it (over another panel) from immediately reading
    /// as a click outside the Viewer and cancelling — the docked panels can
    /// draw in either order within a frame.
    pub eyedropper_primed: bool,
    /// The shape the shape tool draws (its last-picked kind).
    pub shape_kind: ShapeKind,
    /// Shape-tool rubber-band start in layer space; Some while dragging.
    pub shape_drag: Option<(f64, f64)>,
    /// Origin (anchor) mid-drag in the Viewer: the new anchor in layer space.
    /// Committed as one Batch (anchor + pan-behind position) on release.
    pub origin_drag: Option<(f64, f64)>,
    /// The pen's in-progress path (layer space); closes into a mask when the
    /// first vertex is clicked again.
    pub pen_path: Vec<lumit_core::mask::Vertex>,
    /// Property shown in the graph editor.
    pub graph_prop: Option<lumit_core::model::TransformProp>,
    /// In-flight keyframe drag: (key index, provisional layer-time, value).
    pub graph_edit: Option<(usize, f64, f64)>,
    /// In-flight marquee (rubber-band) drag on the graph's background:
    /// (press anchor, current corner) in screen points. `Some` only while the
    /// mouse button is down; on release it becomes a `graph_selection`.
    pub graph_marquee: Option<(egui::Pos2, egui::Pos2)>,
    /// Keyframes selected in the graph editor — by the marquee, or the last
    /// dragged key. Pinned to one channel; see `GraphSelection`.
    pub graph_selection: Option<GraphSelection>,
    /// The highlighted property row in the layer/Effect Controls area (note
    /// 2.8.1; see [`PropSel`]). The *anchor* of the property-row selection — the
    /// most recently clicked row, which a shift-click ranges to and which the
    /// graph follows. `None` when nothing is selected. Kept in step with
    /// `selected_props`: whenever that set is non-empty this is its anchor.
    pub selected_prop: Option<PropSel>,
    /// The full set of highlighted property rows (note 2.6): plain click picks
    /// one, Ctrl-click toggles one, Shift-click ranges from the anchor over the
    /// rows drawn between. Every row in the set lifts its background; the single
    /// `selected_prop` remains the anchor. UI state only.
    pub selected_props: Vec<PropSel>,
    /// Keyframes picked out on the timeline lanes (note 2.1): by clicking a key,
    /// by the lane marquee, or by the last lane drag. Spans property rows
    /// (note 2.6). Entries pin a key by time; a stale one (its key edited away)
    /// simply matches nothing. UI state only.
    pub lane_selection: Vec<LaneKeySel>,
    /// In-flight lane keyframe time drag (note 2.1), driving the live preview of
    /// every selected key's slide; committed as one Batch on release.
    pub lane_key_drag: Option<LaneKeyDrag>,
    /// In-flight lane marquee (rubber-band) on empty timeline space: (press
    /// anchor, current corner) in screen points, mirroring the graph editor's.
    /// `Some` only while the button is down; on release it selects the keys it
    /// covered.
    pub lane_marquee: Option<(egui::Pos2, egui::Pos2)>,
    /// Whether the live lane marquee adds to the existing selection (Shift held
    /// when the drag began) rather than replacing it (note 2.6c).
    pub lane_marquee_add: bool,
    /// A released lane marquee band waiting to be hit-tested: set when the drag
    /// ends, resolved after the row loop has refilled `lane_glyphs`. Transient.
    pub lane_marquee_commit: Option<(egui::Pos2, egui::Pos2)>,
    /// Every keyframe glyph drawn on the lanes this frame, gathered for the
    /// marquee's cross-row hit-test (see [`LaneGlyph`]). Cleared and refilled
    /// each timeline frame.
    pub lane_glyphs: Vec<LaneGlyph>,
    /// The lane keyframe clipboard (note 2.2): the copied keys with their bezier
    /// handles, offset from the copy anchor. Ctrl+V replays them at the
    /// playhead, overwriting any key whose time coincides.
    pub keyframe_clipboard: Vec<ClipboardKey>,
    /// Set on the frame a lane keyframe drag is released: the time shift
    /// (seconds) to apply to the whole lane selection. `timeline_panel` reads it
    /// after the row loop, builds one Batch, and clears it. Transient.
    pub lane_drag_commit: Option<f64>,
    /// Transform rows drawn this frame as a *linked* Anchor/Position/Scale pair
    /// (layer, the x channel): so a lane drag on such a row moves both axes' keys
    /// at that time, while a standalone (unlinked) axis row moves only its own.
    /// Rebuilt each frame. Transient.
    pub lane_linked: Vec<(Uuid, lumit_core::model::TransformProp)>,
    /// The property rows drawn this frame in visual (top-to-bottom) order, for
    /// Shift range-select of property names (note 2.6b). Rebuilt each timeline
    /// frame. Transient.
    pub prop_row_order: Vec<PropSel>,
    /// A Shift-click on a property name, resolved after the row loop against
    /// `prop_row_order`: the range from the anchor (`selected_prop`) to this row
    /// becomes `selected_props`. Transient.
    pub prop_range_target: Option<PropSel>,
    /// In-flight speed-graph drag: (key index, provisional speed in
    /// value-units/second). Separate from `graph_edit` because the speed lens
    /// edits a keyframe's tangent (K-070), not its value or time.
    pub graph_speed_edit: Option<(usize, f64)>,
    /// In-flight value-lens tangent-handle drag: (key index, out side?,
    /// provisional slope in value-units/second, provisional influence in (0, 1]).
    /// `out` chooses the forward or backward handle; the curve previews live and
    /// the release writes the bezier side(s) back (unified unless Alt-dragged).
    pub graph_tangent_edit: Option<(usize, bool, f64, f64)>,
    /// The in-flight tangent drag's mirroring mode: (was the key unified when
    /// the drag started, has Alt been held at any point since). Mirroring =
    /// XOR of the two (see `tangent_mirrors`): Alt toggles it once and latches,
    /// so a break survives releasing Alt, and Alt on a broken key re-unifies.
    pub graph_tangent_mode: Option<(bool, bool)>,
    /// A pending interpolation change for the graphed transform channel's keys
    /// (selection, or all keys when nothing is selected): set by F9 and the
    /// bottom-bar Linear/Bezier buttons, consumed by `graph_plot`.
    pub graph_set_interp: Option<lumit_core::anim::SideInterp>,
    /// Graph editor lens: false = value graph, true = speed graph
    /// (docs/01-GLOSSARY.md §3: two views of the same data, never separate).
    pub graph_speed_view: bool,
    /// Manual value-lens y-range `(min, max)` when the user has scrolled or
    /// zoomed the graph vertically (K-079). `None` = auto-fit to the curve (the
    /// default); the bottom-bar Fit toggle clears it back to `None`.
    pub graph_view_y: Option<(f64, f64)>,
    /// Whether the value graph keeps re-fitting its y-range to the curve every
    /// frame (the bottom-bar Fit toggle, on by default). A vertical wheel,
    /// Ctrl-wheel zoom or scrollbar drag switches it off and takes over via
    /// `graph_view_y`; switching it back on clears the manual range.
    pub graph_auto_fit: bool,
    /// The plot height (px) the current manual `graph_view_y` was framed at.
    /// When the timeline panel is resized the manual range grows or shrinks
    /// about its centre by the height ratio, so the value scale (units per
    /// pixel) holds — more height shows more curve, never a stretch. `None`
    /// while auto-fitting, or until `graph_plot` stamps the live height.
    pub graph_view_h: Option<f32>,
    /// The auto-fit y-range `graph_plot` computed last frame, so a first
    /// vertical scroll can seed a manual range from what's on screen (K-079).
    pub graph_last_fit: Option<(f64, f64)>,
    /// Graph the selected footage layer's Retime channel (K-075) rather than a
    /// transform property: value lens = source position as frame timecode,
    /// derivative lens = speed %.
    pub graph_retime: bool,
    /// Vegas-editor preference (K-075): the Speed/Retime channel opens to the
    /// speed-% (derivative) lens by default; off, to the frame-timecode lens.
    /// Session state for now — a persisted Settings home is a later refinement.
    pub vegas_default_lens: bool,
    /// What the faint vertical guide lines through the lanes mark: detected
    /// beats (default), the time grid (seconds, subdividing with zoom), or
    /// nothing. Session state, like the other timeline preferences.
    pub timeline_grid: TimelineGrid,
    /// In-flight speed-keyframe drag on the Retime channel's % lens (K-075, 2b):
    /// (keyframe index, provisional speed per cent). The retime rebuilds from the
    /// edited keyframe on release; downstream boundaries recompute (K-070).
    pub graph_retime_edit: Option<(usize, f64)>,
    /// Comp shown in the Viewer (takes precedence over preview_item).
    pub preview_comp: Option<Uuid>,
    /// Wall-clock comp playback v0 (the frame scheduler replaces this):
    /// (started, frame at start).
    pub comp_playback: Option<(Instant, usize)>,
    /// Footage item currently shown in the Viewer, and the scrub position.
    pub preview_item: Option<Uuid>,
    pub preview_frame: usize,
    /// Preview resolution divisor: 1 = Full, 2 = Half, 3 = Third, 4 = Quarter.
    /// Ignored while `preview_auto_res` is on.
    pub preview_divisor: u32,
    /// Auto resolution (K-030 family): decode at the size actually displayed,
    /// capped at 100% — zooming past 1:1 never upsamples the decode.
    pub preview_auto_res: bool,
    /// True while the user is actively scrubbing the playhead: the preview
    /// decodes a coarse draft for instant feedback, then reloads at the
    /// specified resolution once scrubbing stops (Mack's "force realtime").
    pub preview_draft: bool,
    /// View zoom (1.0 = fit) and pan, in screen pixels. View controls only —
    /// never part of any render (07-UI-SPEC: Viewer).
    pub view_zoom: f32,
    pub view_pan: egui::Vec2,
    /// Screen pixels per native image pixel at last paint (Auto res input).
    pub last_display_scale: f32,
    /// Draggable width of the timeline's left (layer-controls) column, px.
    pub timeline_name_w: f32,
    /// Accumulated raw pointer width while the layer/lane divider is being
    /// dragged (drag-catch-up note 1): the shown `timeline_name_w` is this
    /// value clamped, so once the drag pins at a limit the divider only starts
    /// moving back when the cursor returns to its actual position. `None` when
    /// no divider drag is in flight.
    pub timeline_divider_raw: Option<f32>,
    /// Snapping toggle for the timeline (magnet, on by default): when on, a
    /// dragged keyframe snaps its time to the nearest whole frame. Shared by the
    /// lane and graph views. Session state, like the other timeline preferences.
    pub magnet_snap: bool,
    /// Lane-area horizontal view (07-UI-SPEC §4): zoom (1.0 = the whole comp fits
    /// the track width; larger zooms in) and the comp time at the left edge.
    /// Alt-wheel zooms, Shift-wheel scrolls; vertical scroll is the ScrollArea's.
    pub timeline_zoom: f64,
    pub timeline_view_start: f64,
    /// Timeline right area shows the graph editor (curves) instead of the
    /// layer bars — a mode of the Timeline, not a separate panel (K-070).
    pub timeline_graph_mode: bool,
    /// Kura's RAM tier for final comp frames (K-016): display-ready sRGB
    /// bytes keyed by content hash. Hash mismatch is the only invalidation.
    #[cfg(feature = "media")]
    pub comp_frame_cache: lumit_cache::ByteLru<u128, CachedCompFrame>,
    /// Bumped on every cache insert (cache-bar memo + repaint driver).
    #[cfg(feature = "media")]
    pub cache_epoch: u64,
    /// A warm frame the shell should present instead of waiting on a render.
    #[cfg(feature = "media")]
    pub cached_present: Option<u128>,
    /// The (comp, frame) currently rendering for the background cache fill.
    #[cfg(feature = "media")]
    pub fill_in_flight: Option<(Uuid, usize)>,
    /// The disk tier's IO worker (docs/06 §5.4), started lazily once the
    /// project has a path (unsaved projects have no sidecar to cache into).
    pub disk_io: Option<diskio::DiskIo>,
    /// The sidecar root the worker currently points at (memo, so the root is
    /// re-sent only when the project path actually changes).
    disk_root: Option<std::path::PathBuf>,
    /// Keys with a disk load in flight — suppresses duplicate load requests
    /// until the frame lands in RAM (drained each frame).
    disk_load_pending: std::collections::HashSet<u128>,
    /// Cache-bar memo: recomputed only when the memo key changes.
    #[cfg(feature = "media")]
    cache_bar_memo: Option<(CacheBarKey, std::sync::Arc<Vec<CacheTier>>)>,
    last_autosave: Instant,
    comp_counter: usize,
    /// Comps open as Timeline tabs, in tab order (07-UI-SPEC §4: one Timeline
    /// panel, one tab per open comp). `selected_comp` names the active tab and
    /// is always one of these when it is set. Session state, not saved in the
    /// document.
    pub open_comps: Vec<Uuid>,
    /// Set by the Timeline comp strip's context menu ("Pop out timeline"): a
    /// solo Timeline has no dock tab to host the pop-out button (K-086), and
    /// the strip renders deep inside the panel, so the request travels to the
    /// shell through here. Consumed each frame after the dock draws.
    pub pop_out_timeline: bool,
    /// Set after importing footage (UI-13): the shell brings the Project tab to
    /// the front so the user sees where the new item landed. Consumed each frame
    /// after the dock draws, like `pop_out_timeline`.
    pub focus_project_tab: bool,
    /// The layer whose name is being edited inline in the Timeline outline, and
    /// its live edit buffer. Double-clicking a layer name opens the editor;
    /// Enter or focus-loss commits a `RenameLayer`, Escape cancels. Session
    /// state — nothing is written to the document until commit.
    pub renaming_layer: Option<(Uuid, String)>,
    /// In-flight reorder drag in the Timeline outline: the layer being dragged
    /// and the current pointer y (screen px). Cleared on release, when a
    /// `ReorderLayer` is committed if the drop lands on a new slot.
    pub layer_reorder: Option<(Uuid, f32)>,
}

impl Default for AppState {
    fn default() -> Self {
        let doc = Document::new();
        let journal = JournalFile::for_document(doc.id);
        #[cfg(feature = "media")]
        let (audio_tx, audio_rx) = std::sync::mpsc::channel();
        #[cfg(feature = "media")]
        let (comp_audio_tx, comp_audio_rx) = std::sync::mpsc::channel();
        #[cfg(feature = "media")]
        let (beats_tx, beats_rx) = std::sync::mpsc::channel();
        Self {
            store: DocumentStore::new(doc),
            path: None,
            journal,
            dirty: false,
            selected_comp: None,
            pending_recovery: None,
            error: None,
            #[cfg(feature = "media")]
            media: media::MediaRegistry::default(),
            #[cfg(feature = "media")]
            preview_engine: preview::PreviewEngine::default(),
            #[cfg(feature = "media")]
            audio_engine: None,
            #[cfg(feature = "media")]
            audio_cache: std::collections::HashMap::new(),
            #[cfg(feature = "media")]
            audio_loaded: None,
            #[cfg(feature = "media")]
            audio_loaded_comp: None,
            #[cfg(feature = "media")]
            audio_loaded_sig: None,
            #[cfg(feature = "media")]
            audio_preparing: None,
            #[cfg(feature = "media")]
            comp_audio_rx,
            #[cfg(feature = "media")]
            comp_audio_tx,
            #[cfg(feature = "media")]
            beats_rx,
            #[cfg(feature = "media")]
            beats_tx,
            #[cfg(feature = "media")]
            comp_waveform: None,
            #[cfg(feature = "media")]
            detected_bpm: None,
            #[cfg(feature = "media")]
            audio_rx,
            #[cfg(feature = "media")]
            audio_tx,
            prop_edit: None,
            fx_edit: None,
            scale_preview: None,
            retime_edit: None,
            trim_edit: None,
            move_edit: None,
            selected_layer: None,
            selected_clip: None,
            graph_prop: None,
            graph_edit: None,
            graph_marquee: None,
            graph_selection: None,
            selected_prop: None,
            selected_props: Vec::new(),
            lane_selection: Vec::new(),
            lane_key_drag: None,
            lane_marquee: None,
            lane_marquee_add: false,
            lane_marquee_commit: None,
            lane_glyphs: Vec::new(),
            keyframe_clipboard: Vec::new(),
            lane_drag_commit: None,
            lane_linked: Vec::new(),
            prop_row_order: Vec::new(),
            prop_range_target: None,
            graph_speed_edit: None,
            graph_tangent_edit: None,
            graph_tangent_mode: None,
            graph_set_interp: None,
            graph_speed_view: false,
            graph_view_y: None,
            graph_auto_fit: true,
            graph_view_h: None,
            graph_last_fit: None,
            graph_retime: false,
            vegas_default_lens: false,
            timeline_grid: TimelineGrid::Time,
            graph_retime_edit: None,
            preview_comp: None,
            comp_playback: None,
            preview_item: None,
            preview_frame: 0,
            preview_divisor: 1,
            preview_auto_res: false,
            preview_draft: false,
            view_zoom: 1.0,
            view_pan: egui::Vec2::ZERO,
            last_display_scale: 1.0,
            timeline_name_w: 300.0,
            timeline_divider_raw: None,
            magnet_snap: true,
            timeline_zoom: 1.0,
            timeline_view_start: 0.0,
            timeline_graph_mode: false,
            #[cfg(feature = "media")]
            comp_frame_cache: lumit_cache::ByteLru::new(512 * 1024 * 1024),
            #[cfg(feature = "media")]
            cache_epoch: 0,
            #[cfg(feature = "media")]
            cached_present: None,
            #[cfg(feature = "media")]
            fill_in_flight: None,
            disk_io: None,
            disk_root: None,
            disk_load_pending: std::collections::HashSet::new(),
            #[cfg(feature = "media")]
            cache_bar_memo: None,
            last_autosave: Instant::now(),
            comp_counter: 0,
            open_comps: Vec::new(),
            pop_out_timeline: false,
            focus_project_tab: false,
            renaming_layer: None,
            layer_reorder: None,
            selected_item: None,
            selected_items: Vec::new(),
            mask_drag: None,
            tool: ToolMode::default(),
            eyedropper: None,
            eyedropper_region: 1,
            eyedropper_primed: false,
            shape_kind: ShapeKind::default(),
            shape_drag: None,
            origin_drag: None,
            pen_path: Vec::new(),
            comp_dialog: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod preview_tests;
