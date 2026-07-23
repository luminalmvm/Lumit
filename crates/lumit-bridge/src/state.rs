//! The shared engine state and the pure state transitions.
//!
//! # In plain terms
//!
//! Everything the Flutter client can *do* to the document is a function here.
//! Each one takes the engine state (or nothing) and returns the reply JSON as a
//! `String`; it holds no lock and touches no global, so the tests drive their
//! own [`Bridge`] in full isolation and the exported C functions in [`crate::ffi`]
//! route the one shared instance through them. Mutations go through the real
//! [`lumit_core::ops::Op`] path and [`DocumentStore`], so undo/redo works exactly
//! as it does in the egui frontend.

use crate::err_json;
use crate::media::MediaCache;
use crate::snapshot::snapshot_value;
use lumit_core::anim::Animation;
use lumit_core::markers::Marker;
use lumit_core::model::{
    Composition, Document, Folder, FootageItem, LinearColour, MediaRef, ProjectItem, TransformProp,
};
use lumit_core::ops::{AutoFolderKind, Op, SpanEdit};
use lumit_core::store::DocumentStore;
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_project::JournalFile;
use serde_json::json;
use std::path::PathBuf;
#[cfg(feature = "render")]
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// The engine-side state one Flutter client drives: the document store (which
/// owns the document and its undo/redo history, exactly as `lumit-ui`'s
/// `AppState` does), the path the project was loaded from or last saved to, and
/// the cached media-probe results ([`MediaCache`], populated on import/open when
/// the `media` feature is on).
pub(crate) struct Bridge {
    pub store: DocumentStore,
    pub path: Option<PathBuf>,
    pub media: MediaCache,
    /// The crash journal for the current document (K-176 recovery). Every
    /// [`commit`] appends its op here, exactly as `lumit-ui`'s `AppState::commit`
    /// does, so `restore_journal` recovers *this* frontend's unsaved work rather
    /// than a journal another session wrote. Established at each document-install
    /// point ([`set_journal_for_current_doc`]) — `Bridge::new()` leaves it `None`
    /// so a raw test bridge never touches the on-disk cache; the real app calls
    /// `new_project`/`open_project` at start-up, which arms it. Cleared on a
    /// successful save (the journal covers work *between* saves) and when a new
    /// document replaces the old one.
    pub journal: Option<JournalFile>,
    /// The active drag-preview overlay, or `None` outside any drag. Overwritten
    /// in place each tick within one drag ("latest wins", docs/14 §5) — a plain
    /// `Option` behind the existing `BRIDGE` mutex, not a second lock and not a
    /// channel, so staging a tick costs exactly one mutex acquisition, the same
    /// as every other bridge call. Lives here, entirely outside `DocumentStore`/
    /// `Document` — export takes its own fresh `with_bridge(|b| b.store.snapshot())`
    /// and never sees this, so a preview can never leak into an exported file
    /// (docs/14 §3: "preview resolution affects preview only").
    pub preview: Option<TransformPreview>,
}

/// One in-flight, uncommitted transform preview: which comp/layer it targets,
/// and the property→value edits staged this drag. A `Vec` (not two named
/// fields) covers the linked-Scale-axes case (2 entries) and any future
/// multi-property preview without a shape change.
#[derive(Clone)]
pub(crate) struct TransformPreview {
    pub comp: Uuid,
    pub layer: Uuid,
    pub edits: Vec<(TransformProp, Animation)>,
}

impl Bridge {
    pub fn new() -> Self {
        Self {
            store: DocumentStore::new(Document::new()),
            path: None,
            media: MediaCache::default(),
            journal: None,
            preview: None,
        }
    }
}

/// Point the bridge's journal at the current document's sidecar (keyed by the
/// document id, `lumit_project::journal_path`). Called at every document-install
/// point so the journal always tracks the live document. A machine with no cache
/// directory yields `None` (journaling silently disabled) — never an error.
pub(crate) fn set_journal_for_current_doc(bridge: &mut Bridge) {
    let id = bridge.store.snapshot().id;
    bridge.journal = JournalFile::for_document(id);
}

/// The single process-wide client state (single-client assumption). The lock is
/// only ever held for the duration of one pure state transition below — never
/// across anything that could re-enter the bridge — so it cannot deadlock.
static BRIDGE: OnceLock<Mutex<Bridge>> = OnceLock::new();

/// Run `f` against the shared bridge state. A previous panic that poisoned the
/// lock is recovered from (the caught panic already produced an error reply;
/// the state itself is a valid `Document`), so one bad call never wedges the
/// bridge for the rest of the session.
pub(crate) fn with_bridge<R>(f: impl FnOnce(&mut Bridge) -> R) -> R {
    let mutex = BRIDGE.get_or_init(|| Mutex::new(Bridge::new()));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    f(&mut guard)
}

/// `{"ok":true,"version":"…","abi":7}` — stateless.
pub(crate) fn version() -> String {
    json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "abi": crate::ABI_VERSION,
    })
    .to_string()
}

/// The current document as the panels read it (snapshot v2 — see [`crate::snapshot`]).
pub(crate) fn snapshot(bridge: &Bridge) -> String {
    snapshot_value(bridge).to_string()
}

pub(crate) fn new_project(bridge: &mut Bridge) -> String {
    // Replacing the store wholesale bypasses `commit()`; drop any stale
    // preview so it can never reference ids from the discarded document.
    bridge.preview = None;
    // Clear the outgoing document's journal before switching documents (the old
    // work is being discarded), exactly as `AppState::new_project` does.
    if let Some(journal) = &bridge.journal {
        let _ = journal.clear();
    }
    bridge.store = DocumentStore::new(Document::new());
    bridge.path = None;
    bridge.media.clear();
    set_journal_for_current_doc(bridge);
    snapshot(bridge)
}

pub(crate) fn open_project(bridge: &mut Bridge, path: &str) -> String {
    let path = PathBuf::from(path);
    match lumit_project::open(&path) {
        Ok((doc, _manifest)) => {
            bridge.preview = None;
            bridge.store = DocumentStore::new(doc);
            bridge.path = Some(path);
            bridge.media.clear();
            set_journal_for_current_doc(bridge);
            refresh_media(bridge);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("open project: {e}")),
    }
}

/// Save to `path`; an empty `path` means "save to the loaded path" and errors
/// when there is none yet (bridge v0 has no file dialog to ask for one). The
/// written document is rebased against its folder exactly as `lumit-ui`'s Save
/// does, so no machine-specific path reaches the file (K-173).
pub(crate) fn save_project(bridge: &mut Bridge, path: &str) -> String {
    let target = if path.is_empty() {
        match &bridge.path {
            Some(p) => p.clone(),
            None => {
                return err_json(
                    "save project: no path yet — this project has never been saved, so a path is required",
                );
            }
        }
    } else {
        PathBuf::from(path)
    };
    let dir = target.parent().unwrap_or_else(|| std::path::Path::new(""));
    let doc = lumit_project::rebase_for_save(&bridge.store.snapshot(), dir);
    match lumit_project::save(&doc, &target) {
        Ok(()) => {
            // The journal covers work *between* saves; a successful save makes
            // it redundant, so clear it (matching `AppState::save`).
            if let Some(journal) = &bridge.journal {
                let _ = journal.clear();
            }
            bridge.path = Some(target);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("save project: {e}")),
    }
}

/// Create a composition and file it in the "Compositions" auto-folder, as one
/// undo step — the same op path `lumit-ui`'s `confirm_comp_dialog` commits, so
/// undo removes it cleanly. An empty `name` becomes "Comp N" (N counting the
/// compositions already present).
pub(crate) fn new_composition(bridge: &mut Bridge, name: &str) -> String {
    let doc = bridge.store.snapshot();
    let name = if name.trim().is_empty() {
        let existing = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Composition(_)))
            .count();
        format!("Comp {}", existing + 1)
    } else {
        name.to_owned()
    };

    let (frame_rate, duration) = match (FrameRate::new(60, 1), Rational::new(30, 1)) {
        (Ok(fr), Ok(dur)) => (fr, Duration(dur)),
        _ => return err_json("new composition: could not build the default frame rate"),
    };

    // Ensure the Compositions auto-folder exists (tracked by id, like the egui
    // frontend: renaming or nesting it keeps it the Compositions folder).
    let mut ops: Vec<Op> = Vec::new();
    let folder_id = match doc
        .auto_folders
        .compositions
        .filter(|id| doc.folder(*id).is_some())
    {
        Some(id) => id,
        None => {
            let id = Uuid::now_v7();
            ops.push(Op::AddItem {
                index: doc.items.len(),
                item: Box::new(ProjectItem::Folder(Folder {
                    id,
                    name: "Compositions".into(),
                    children: Vec::new(),
                    extra: serde_json::Map::new(),
                })),
            });
            ops.push(Op::SetAutoFolder {
                kind: AutoFolderKind::Compositions,
                folder: Some(id),
            });
            id
        }
    };

    let comp = Composition {
        id: Uuid::now_v7(),
        name,
        width: 1920,
        height: 1080,
        frame_rate,
        duration,
        background: LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;

    // The comp's index accounts for any AddItem ops queued ahead of it.
    let added = ops
        .iter()
        .filter(|o| matches!(o, Op::AddItem { .. }))
        .count();
    ops.push(Op::AddItem {
        index: doc.items.len() + added,
        item: Box::new(ProjectItem::Composition(comp)),
    });

    // File it into the folder. The folder may have been created earlier in this
    // same batch (so it is absent from `doc`), in which case its children start
    // empty — matching `lumit-ui`'s `file_into_folder_op`.
    let mut children = doc
        .folder(folder_id)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.push(comp_id);
    ops.push(Op::SetFolderChildren {
        folder: folder_id,
        children,
    });

    let op = Op::Batch { ops };
    match bridge.store.commit(op.clone()) {
        Ok(_) => {
            journal_append(bridge, &op);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("new composition: {e}")),
    }
}

/// Add a footage item for `path` as one undo step — the same op `lumit-ui`'s
/// `import_paths` commits for each picked file. Footage has no auto-folder (only
/// solids and comps do), so this mirrors the egui frontend exactly: a single
/// [`Op::AddItem`] appended to the flat item list, and undo removes it.
///
/// # In plain terms
///
/// This records that a media file belongs to the project. With the `media`
/// feature on it then *probes* the file synchronously (reads its resolution,
/// frame rate and frame count, and builds/loads the frame index) so the Project
/// panel can show the details straight away; the result is cached in the
/// [`MediaCache`] and carried in the snapshot. A file that is not on disk probes
/// to "missing" — that is a normal state (relink), never an error reply.
pub(crate) fn import_footage(bridge: &mut Bridge, path: &str) -> String {
    if path.trim().is_empty() {
        return err_json("import footage: no path given");
    }
    let file = PathBuf::from(path);
    // The item's name is the file's own name, exactly as `import_paths` derives
    // it (falling back to "footage" for a path with no final component).
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "footage".into());
    let item = FootageItem {
        id: Uuid::now_v7(),
        name: name.clone(),
        media: MediaRef {
            // Import stores the bare name as the relative path; Save rebases it
            // against the project folder (K-173), just as the egui frontend does.
            relative_path: name,
            absolute_path: file.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    };
    let index = bridge.store.snapshot().items.len();
    let op = Op::AddItem {
        index,
        item: Box::new(ProjectItem::Footage(item)),
    };
    match bridge.store.commit(op.clone()) {
        Ok(_) => {
            journal_append(bridge, &op);
            refresh_media(bridge);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("import footage: {e}")),
    }
}

pub(crate) fn undo(bridge: &mut Bridge) -> String {
    // Bypasses `commit()`, so drop a stale preview here too — otherwise a
    // lost-mouseup followed by Ctrl+Z could leave an overlay referencing ids
    // from a document state undo just moved past.
    bridge.preview = None;
    match bridge.store.undo() {
        Ok(_) => snapshot(bridge),
        Err(e) => err_json(format!("undo: {e}")),
    }
}

pub(crate) fn redo(bridge: &mut Bridge) -> String {
    bridge.preview = None;
    match bridge.store.redo() {
        Ok(_) => snapshot(bridge),
        Err(e) => err_json(format!("redo: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Snapshot-v2 ops: switches, spans, transforms, markers. Each maps onto the
// real, unit-tested lumit-core op so undo/redo is one clean step, and each
// returns the full refreshed snapshot on success (the panels re-read wholesale).
// ---------------------------------------------------------------------------

/// Flip one of a layer's switches. `switch_name` is the model's own field name:
/// `visible`, `audible`, `locked`, `solo`, `motion_blur`, `fx`, `three_d`, or
/// `collapse`. Each routes through the matching `SetLayer*` op.
pub(crate) fn set_layer_switch(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    switch_name: &str,
    value: bool,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("set layer switch: {e}")),
    };
    let op = match switch_name {
        "visible" => Op::SetLayerVisible {
            comp,
            layer,
            visible: value,
        },
        "audible" => Op::SetLayerAudible {
            comp,
            layer,
            audible: value,
        },
        "locked" => Op::SetLayerLocked {
            comp,
            layer,
            locked: value,
        },
        "solo" => Op::SetLayerSolo {
            comp,
            layer,
            solo: value,
        },
        "motion_blur" => Op::SetLayerMotionBlur {
            comp,
            layer,
            motion_blur: value,
        },
        "fx" => Op::SetLayerFx {
            comp,
            layer,
            fx: value,
        },
        "three_d" => Op::SetLayerThreeD {
            comp,
            layer,
            three_d: value,
        },
        "collapse" => Op::SetLayerCollapse {
            comp,
            layer,
            collapse: value,
        },
        other => return err_json(format!("set layer switch: unknown switch '{other}'")),
    };
    commit(bridge, op, "set layer switch")
}

/// Edit a layer's span relative to the playhead `frame`. `edit` is one of
/// `move_in`, `move_out`, `trim_in`, `trim_out`, mapped onto lumit-core's
/// tested [`SpanEdit`]/[`lumit_core::ops::edit_layer_span`]. The new
/// `(in, out, start_offset)` is committed as one [`Op::SetLayerSpan`].
pub(crate) fn edit_layer_span(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    edit: &str,
    frame: i64,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("edit layer span: {e}")),
    };
    let span_edit = match edit {
        "move_in" => SpanEdit::MoveIn,
        "move_out" => SpanEdit::MoveOut,
        "trim_in" => SpanEdit::TrimIn,
        "trim_out" => SpanEdit::TrimOut,
        other => return err_json(format!("edit layer span: unknown edit '{other}'")),
    };
    let doc = bridge.store.snapshot();
    let Some(c) = doc.comp(comp) else {
        return err_json("edit layer span: unknown composition");
    };
    let Some(l) = c.layers.iter().find(|l| l.id == layer) else {
        return err_json("edit layer span: unknown layer");
    };
    let playhead = match c.frame_rate.time_of_frame(frame) {
        Ok(t) => t,
        Err(e) => return err_json(format!("edit layer span: {e}")),
    };
    let Some((in_point, out_point, start_offset)) = lumit_core::ops::edit_layer_span(
        l.in_point,
        l.out_point,
        l.start_offset,
        playhead,
        span_edit,
    ) else {
        return err_json("edit layer span: the edit would collapse the layer");
    };
    commit(
        bridge,
        Op::SetLayerSpan {
            comp,
            layer,
            in_point,
            out_point,
            start_offset,
        },
        "edit layer span",
    )
}

/// Set one transform property to a static `value`. `property` is one of the
/// snake_case names mirroring [`TransformProp`]: `anchor_x`, `anchor_y`,
/// `position_x`, `position_y`, `position_z`, `scale_x`, `scale_y`, `rotation`,
/// `rotation_x`, `rotation_y`, `opacity`. Committed as [`Op::SetTransformProperty`]
/// with a [`Animation::Static`], replacing whatever animation was there (the
/// coarse-grained op the graph editor later refines).
pub(crate) fn set_transform(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    value: f64,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("set transform: {e}")),
    };
    let Some(prop) = parse_transform_prop(property) else {
        return err_json(format!("set transform: unknown property '{property}'"));
    };
    commit(
        bridge,
        Op::SetTransformProperty {
            comp,
            layer,
            prop,
            animation: Animation::Static(value),
        },
        "set transform",
    )
}

/// Stage (or update) an in-memory transform preview for `layer_id`'s
/// `property` — no document mutation, no undo entry, no journal write, no
/// snapshot re-serialisation (the whole point: a drag can call this every
/// tick for a cost of one mutex acquisition and a small in-memory write). A
/// call for the SAME (comp, layer) already staged this drag updates just that
/// property's entry (so the linked-Scale-axes case — two `preview_transform`
/// calls per tick, one per axis — accumulates both without clobbering each
/// other); a call for a DIFFERENT (comp, layer) replaces the whole overlay (a
/// new drag started). Returns the tiny stateless `{"ok":true}` ack
/// ([`crate::ok_json`]) — callers must NOT treat this as a document snapshot.
pub(crate) fn preview_transform(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    property: &str,
    value: f64,
) -> String {
    let (comp, layer) = match parse_comp_layer(comp_id, layer_id) {
        Ok(pair) => pair,
        Err(e) => return err_json(format!("preview transform: {e}")),
    };
    let Some(prop) = parse_transform_prop(property) else {
        return err_json(format!("preview transform: unknown property '{property}'"));
    };
    let animation = Animation::Static(value);
    match &mut bridge.preview {
        Some(p) if p.comp == comp && p.layer == layer => {
            if let Some(slot) = p.edits.iter_mut().find(|(pr, _)| *pr == prop) {
                slot.1 = animation;
            } else {
                p.edits.push((prop, animation));
            }
        }
        _ => {
            bridge.preview = Some(TransformPreview {
                comp,
                layer,
                edits: vec![(prop, animation)],
            });
        }
    }
    crate::ok_json()
}

/// Drop the active preview without committing (Escape / drag-cancel). The
/// next render call falls back to the untouched, pre-drag document. Returns
/// the tiny stateless ack.
pub(crate) fn cancel_transform_preview(bridge: &mut Bridge) -> String {
    bridge.preview = None;
    crate::ok_json()
}

/// The document to render THIS frame: the store's real committed snapshot
/// with the active preview's edits applied on top, entirely in memory — never
/// published to `DocumentStore`'s `ArcSwap`, never journalled, never given an
/// undo entry. `None` preview is the existing cheap path (an `Arc` clone, no
/// new `Document` allocation) — unchanged from what `render_comp_frame_gen`
/// already does. When a preview IS active this does one `Document::clone`
/// (the same cost `DocumentStore::commit` already pays per edit) and reuses
/// `lumit_core::ops::apply` — the exact code [`commit`] calls — so a preview
/// renders pixel-identical to what the eventual real commit produces; the
/// returned inverse is discarded (nothing to undo). A preview whose comp/layer
/// vanished mid-drag (e.g. deleted by an out-of-band edit) is tolerated: the
/// `Err` is swallowed and the doc renders as if no preview were staged, never
/// a panic. Only [`crate::render`] (the `render` feature) has a use for this —
/// without it, nothing renders a preview frame at all.
#[cfg(feature = "render")]
pub(crate) fn snapshot_with_preview(bridge: &Bridge) -> Arc<Document> {
    let Some(preview) = &bridge.preview else {
        return bridge.store.snapshot();
    };
    let mut doc = Document::clone(&bridge.store.snapshot());
    for (prop, animation) in &preview.edits {
        let _ = lumit_core::ops::apply(
            &mut doc,
            &Op::SetTransformProperty {
                comp: preview.comp,
                layer: preview.layer,
                prop: *prop,
                animation: animation.clone(),
            },
        );
    }
    Arc::new(doc)
}

/// Drop a plain user marker on the composition timeline at `frame`. Committed as
/// [`Op::SetCompMarkers`] (the whole list, trivially invertible), so undo removes
/// exactly the one added.
pub(crate) fn add_marker(bridge: &mut Bridge, comp_id: &str, frame: i64) -> String {
    let comp = match Uuid::parse_str(comp_id) {
        Ok(id) => id,
        Err(_) => return err_json("add marker: composition id is not a valid UUID"),
    };
    let doc = bridge.store.snapshot();
    let Some(c) = doc.comp(comp) else {
        return err_json("add marker: unknown composition");
    };
    let time = match c.frame_rate.time_of_frame(frame) {
        Ok(t) => t,
        Err(e) => return err_json(format!("add marker: {e}")),
    };
    let mut markers = c.markers.clone();
    markers.push(Marker::user(Uuid::now_v7(), time.0));
    commit(bridge, Op::SetCompMarkers { comp, markers }, "add marker")
}

/// The on-disk path a footage item points at this session (the absolute path
/// when known, else the stored relative one). `None` when `item_id` is not a
/// footage item. Used by the frame-decode path in [`crate::ffi`] and the probe
/// path — both `media`-feature only.
#[cfg(feature = "media")]
pub(crate) fn footage_path(bridge: &Bridge, item_id: &str) -> Option<PathBuf> {
    let id = Uuid::parse_str(item_id).ok()?;
    match bridge.store.snapshot().item(id)? {
        ProjectItem::Footage(f) => Some(footage_pathbuf(f)),
        _ => None,
    }
}

#[cfg(feature = "media")]
fn footage_pathbuf(f: &FootageItem) -> PathBuf {
    if f.media.absolute_path.is_empty() {
        PathBuf::from(&f.media.relative_path)
    } else {
        PathBuf::from(&f.media.absolute_path)
    }
}

/// Commit `op`, returning the refreshed snapshot on success or a calm error
/// reply prefixed with `ctx` on failure. Shared with the v0.3 edit ops in
/// [`crate::edits`], so every mutation refreshes the snapshot the same way.
pub(crate) fn commit(bridge: &mut Bridge, op: Op, ctx: &str) -> String {
    // Any real commit obsoletes whatever was being drag-previewed (the drag
    // that staged it either just released — into this very commit — or was
    // superseded by an unrelated edit).
    bridge.preview = None;
    match bridge.store.commit(op.clone()) {
        Ok(_) => {
            journal_append(bridge, &op);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("{ctx}: {e}")),
    }
}

/// Append `op` to the crash journal after a successful commit, mirroring
/// `AppState::commit`. A journal write failure is not fatal (the edit already
/// landed); it simply means recovery may miss this op — the same tolerance the
/// egui path takes. Shared by [`commit`] and the few ops that commit directly
/// (`new_composition`, `import_footage`, the retime setter), so every bridge
/// commit is journalled.
pub(crate) fn journal_append(bridge: &Bridge, op: &Op) {
    if let Some(journal) = &bridge.journal {
        let _ = journal.append(op);
    }
}

/// Parse a composition id and a layer id together, with a shared calm message.
pub(crate) fn parse_comp_layer(comp_id: &str, layer_id: &str) -> Result<(Uuid, Uuid), String> {
    let comp =
        Uuid::parse_str(comp_id).map_err(|_| "composition id is not a valid UUID".to_owned())?;
    let layer = Uuid::parse_str(layer_id).map_err(|_| "layer id is not a valid UUID".to_owned())?;
    Ok((comp, layer))
}

/// Map a snake_case transform-property name to its [`TransformProp`]. The names
/// mirror [`TransformProp`] exactly (`anchor_x`…`opacity`) and are the same set
/// [`set_transform`] accepts; shared with [`crate::edits`] so the read-back,
/// the setter and the keyframe ops all speak one vocabulary.
pub(crate) fn parse_transform_prop(property: &str) -> Option<TransformProp> {
    Some(match property {
        "anchor_x" => TransformProp::AnchorX,
        "anchor_y" => TransformProp::AnchorY,
        "position_x" => TransformProp::PositionX,
        "position_y" => TransformProp::PositionY,
        "position_z" => TransformProp::PositionZ,
        "scale_x" => TransformProp::ScaleX,
        "scale_y" => TransformProp::ScaleY,
        "rotation" => TransformProp::Rotation,
        "rotation_x" => TransformProp::RotationX,
        "rotation_y" => TransformProp::RotationY,
        "opacity" => TransformProp::Opacity,
        _ => return None,
    })
}

/// Probe every footage item not already in the cache (synchronous, `media`
/// feature only). Without the feature this is a no-op and every footage item
/// reports status "unprobed". Shared with [`crate::items`]'s relink, which
/// clears the cache and re-probes the freshly-linked files.
pub(crate) fn refresh_media(bridge: &mut Bridge) {
    #[cfg(feature = "media")]
    {
        let doc = bridge.store.snapshot();
        for item in &doc.items {
            if let ProjectItem::Footage(f) = item {
                if bridge.media.get(&f.id).is_none() {
                    let status = crate::media::probe_path(&footage_pathbuf(f));
                    bridge.media.insert(f.id, status);
                }
            }
        }
    }
    // Referenced only under the feature; keep the parameter honest either way.
    let _ = bridge;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    /// Build a document with one comp holding one footage layer, returning the
    /// bridge, comp id and layer id. Drives the real ops, so the layer is a
    /// genuine `Op::AddLayer` the snapshot then serialises.
    fn comp_with_layer() -> (Bridge, Uuid, Uuid) {
        let mut b = Bridge::new();
        // A comp (through the real batch), then read its id back from the tree.
        new_composition(&mut b, "Scene");
        let doc = b.store.snapshot();
        let comp_id = doc
            .items
            .iter()
            .find_map(|i| match i {
                ProjectItem::Composition(c) => Some(c.id),
                _ => None,
            })
            .expect("a composition exists");
        // Add a footage layer straight through the store.
        let footage = FootageItem {
            id: Uuid::now_v7(),
            name: "clip.mp4".into(),
            media: MediaRef {
                relative_path: "clip.mp4".into(),
                absolute_path: String::new(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
        };
        let footage_id = footage.id;
        b.store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(footage)),
            })
            .unwrap();
        let layer = lumit_core::model::Layer {
            id: Uuid::now_v7(),
            name: "clip.mp4".into(),
            kind: lumit_core::model::LayerKind::Footage {
                item: footage_id,
                retime: None,
            },
            in_point: lumit_core::time::CompTime(Rational::new(0, 1).unwrap()),
            out_point: lumit_core::time::CompTime(Rational::new(5, 1).unwrap()),
            start_offset: lumit_core::time::CompTime(Rational::new(0, 1).unwrap()),
            transform: lumit_core::model::TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: lumit_core::model::Switches::default(),
            extra: serde_json::Map::new(),
        };
        let layer_id = layer.id;
        b.store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(layer),
            })
            .unwrap();
        (b, comp_id, layer_id)
    }

    #[test]
    fn version_reports_the_abi() {
        let v = parse(&version());
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["abi"], json!(crate::ABI_VERSION));
        assert_eq!(v["abi"], json!(11));
    }

    #[test]
    fn new_project_shows_an_empty_document() {
        let mut b = Bridge::new();
        let snap = parse(&new_project(&mut b));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(snap["items"], json!([]));
        assert_eq!(snap["can_undo"], json!(false));
        assert_eq!(snap["path"], Value::Null);
    }

    #[test]
    fn new_composition_lists_it_nested_and_enables_undo() {
        let mut b = Bridge::new();
        let snap = parse(&new_composition(&mut b, "Intro"));
        assert_eq!(snap["can_undo"], json!(true));
        let items = snap["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["kind"], json!("folder"));
        let children = items[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["kind"], json!("composition"));
        assert_eq!(children[0]["name"], json!("Intro"));
    }

    #[test]
    fn import_footage_lists_the_item_and_undoes() {
        let mut b = Bridge::new();
        let snap = parse(&import_footage(&mut b, "/media/clips/shot.mp4"));
        let items = snap["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["kind"], json!("footage"));
        assert_eq!(items[0]["name"], json!("shot.mp4"));
        // Footage always carries a status; without a real file it is missing
        // (media feature) or unprobed (no feature) — never absent.
        assert!(items[0].get("status").is_some(), "footage carries a status");
        let after_undo = parse(&undo(&mut b));
        assert_eq!(after_undo["items"], json!([]));
    }

    #[test]
    fn import_footage_with_an_empty_path_is_a_calm_error() {
        let mut b = Bridge::new();
        let reply = parse(&import_footage(&mut b, "   "));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("no path"));
        assert_eq!(parse(&snapshot(&b))["items"], json!([]));
    }

    #[test]
    fn set_layer_switch_flips_and_undoes() {
        let (mut b, comp, layer) = comp_with_layer();
        // Solo defaults false; set it true through the op.
        let snap = parse(&set_layer_switch(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "solo",
            true,
        ));
        assert_eq!(snap["ok"], json!(true));
        // Find the layer's switches in the snapshot and confirm solo flipped.
        let solo_of = |snap: &Value| -> bool {
            let comp_item = find_comp(snap);
            comp_item["comp"]["layers"][0]["switches"]["solo"] == json!(true)
        };
        assert!(solo_of(&snap), "solo is now set");
        // Undo restores it.
        let after = parse(&undo(&mut b));
        assert!(!solo_of(&after), "undo clears the switch");
    }

    #[test]
    fn set_layer_switch_rejects_an_unknown_switch() {
        let (mut b, comp, layer) = comp_with_layer();
        let reply = parse(&set_layer_switch(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "nebula",
            true,
        ));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("unknown switch"));
    }

    #[test]
    fn edit_layer_span_moves_the_in_point_to_the_playhead() {
        let (mut b, comp, layer) = comp_with_layer();
        // The comp runs at 60 fps; move the in point to frame 120 (comp-time 2s).
        let snap = parse(&edit_layer_span(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "move_in",
            120,
        ));
        assert_eq!(snap["ok"], json!(true));
        let comp_item = find_comp(&snap);
        // in_frame is derived from the comp's fps: 2s × 60 = 120.
        assert_eq!(comp_item["comp"]["layers"][0]["in_frame"], json!(120));
        // The move keeps the 5s duration, so out lands at frame 120 + 300 = 420.
        assert_eq!(comp_item["comp"]["layers"][0]["out_frame"], json!(420));
    }

    #[test]
    fn edit_layer_span_rejects_a_degenerate_trim() {
        let (mut b, comp, layer) = comp_with_layer();
        // Trimming the in point past the out point (out is 5s = frame 300) is
        // rejected as degenerate, not applied.
        let reply = parse(&edit_layer_span(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "trim_in",
            600,
        ));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("collapse"));
    }

    #[test]
    fn set_transform_round_trips_a_value() {
        let (mut b, comp, layer) = comp_with_layer();
        let snap = parse(&set_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "opacity",
            42.0,
        ));
        assert_eq!(snap["ok"], json!(true));
        // The value round-trips through the store: read the opacity back.
        let doc = b.store.snapshot();
        let c = doc.comp(comp).unwrap();
        let l = c.layers.iter().find(|l| l.id == layer).unwrap();
        assert_eq!(l.transform.opacity.value_at(0.0), 42.0);
        // Undo restores the default (100).
        undo(&mut b);
        let doc = b.store.snapshot();
        let l = doc
            .comp(comp)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap();
        assert_eq!(l.transform.opacity.value_at(0.0), 100.0);
    }

    #[test]
    fn set_transform_rejects_an_unknown_property() {
        let (mut b, comp, layer) = comp_with_layer();
        let reply = parse(&set_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "wobble",
            1.0,
        ));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("unknown property"));
    }

    #[test]
    fn preview_transform_never_touches_undo_or_journal() {
        let (mut b, comp, layer) = comp_with_layer();
        let before = b.store.journal_ops().len();
        for v in [10.0, 20.0, 30.0] {
            let reply = parse(&preview_transform(
                &mut b,
                &comp.to_string(),
                &layer.to_string(),
                "position_x",
                v,
            ));
            assert_eq!(reply, json!({ "ok": true }));
        }
        // No undo entry, no change to the REAL document.
        assert_eq!(b.store.journal_ops().len(), before);
        let doc = b.store.snapshot();
        let l = doc
            .comp(comp)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap();
        assert_eq!(l.transform.position_x.value_at(0.0), 0.0);
    }

    #[test]
    fn commit_after_preview_is_one_undo_step_to_the_pre_drag_value() {
        let (mut b, comp, layer) = comp_with_layer();
        let before = b.store.journal_ops().len();
        preview_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "position_x",
            10.0,
        );
        preview_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "position_x",
            55.0,
        );
        // Drag-release: the real, one-shot commit.
        let reply = parse(&set_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "position_x",
            55.0,
        ));
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(b.store.journal_ops().len(), before + 1);
        let doc = b.store.snapshot();
        let l = doc
            .comp(comp)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap();
        assert_eq!(l.transform.position_x.value_at(0.0), 55.0);
        // One undo restores the pre-drag value.
        undo(&mut b);
        let doc = b.store.snapshot();
        let l = doc
            .comp(comp)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap();
        assert_eq!(l.transform.position_x.value_at(0.0), 0.0);
    }

    #[test]
    fn cancel_preview_restores_pre_drag_render_with_zero_undo_entries() {
        let (mut b, comp, layer) = comp_with_layer();
        let before = b.store.journal_ops().len();
        preview_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "position_x",
            42.0,
        );
        assert!(b.preview.is_some());
        let reply = parse(&cancel_transform_preview(&mut b));
        assert_eq!(reply, json!({ "ok": true }));
        assert!(b.preview.is_none());
        assert_eq!(b.store.journal_ops().len(), before);
        // The pre-drag document renders unchanged (no preview staged any more).
        assert_eq!(snapshot_with_preview(&b), b.store.snapshot());
    }

    #[test]
    fn snapshot_with_preview_matches_a_real_commit() {
        let (mut b, comp, layer) = comp_with_layer();
        preview_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "position_x",
            7.5,
        );
        let previewed = snapshot_with_preview(&b);

        // Commit the SAME value for real and compare against the preview render
        // — both should reach the pixel-identical document (same `ops::apply`
        // call underneath).
        set_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "position_x",
            7.5,
        );
        let committed = b.store.snapshot();

        assert_eq!(*previewed, *committed);
    }

    #[test]
    fn linked_axes_preview_holds_both_entries() {
        let (mut b, comp, layer) = comp_with_layer();
        preview_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "scale_x",
            50.0,
        );
        preview_transform(
            &mut b,
            &comp.to_string(),
            &layer.to_string(),
            "scale_y",
            50.0,
        );
        let preview = b.preview.as_ref().expect("a preview is staged");
        assert_eq!(preview.edits.len(), 2);
    }

    #[test]
    fn add_marker_appears_in_the_comp_snapshot_as_a_frame() {
        let (mut b, comp, _layer) = comp_with_layer();
        let snap = parse(&add_marker(&mut b, &comp.to_string(), 90));
        assert_eq!(snap["ok"], json!(true));
        let comp_item = find_comp(&snap);
        assert_eq!(comp_item["comp"]["markers"], json!([90]));
        // Undo removes it.
        let after = parse(&undo(&mut b));
        assert_eq!(find_comp(&after)["comp"]["markers"], json!([]));
    }

    #[test]
    fn ops_on_a_bad_uuid_are_calm_errors() {
        let mut b = Bridge::new();
        let reply = parse(&set_transform(&mut b, "not-a-uuid", "nope", "opacity", 1.0));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("UUID"));
    }

    /// Locate the single composition item in a snapshot (nested one level under
    /// the auto-folder).
    fn find_comp(snap: &Value) -> Value {
        for item in snap["items"].as_array().unwrap() {
            if item["kind"] == json!("composition") {
                return item.clone();
            }
            for child in item["children"].as_array().unwrap() {
                if child["kind"] == json!("composition") {
                    return child.clone();
                }
            }
        }
        panic!("no composition in snapshot");
    }
}
