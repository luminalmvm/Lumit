//! The shell's readouts and its crash-recovery surface.
//!
//! # In plain terms
//!
//! Three things the window around the panels needs and no panel does: what this
//! build of the engine actually is (the splash's boot lines), how hard the
//! engine is finding playback (the quality tier), and the safety net — the
//! rotating autosaves and the crash journal that together mean a session ending
//! badly does not end your work.
//!
//! None of it is undoable, because none of it changes the document. Asking is
//! always safe; the two that *do* write (autosave, restore) say so in their
//! names.

use flutter_rust_bridge::frb;
use lumit_project::JournalFile;

use crate::api::{project::ProjectReference, BridgeError};

/// What this build can truthfully say about itself at load time (K-008).
///
/// Facts only. The GPU adapter is not named, because it is not known until the
/// first render — a splash that claimed one would be inventing it.
#[frb(sync)]
pub fn boot_log() -> Vec<String> {
    vec![
        format!("lumit-bridge {}", env!("CARGO_PKG_VERSION")),
        format!(
            "media (decode/probe): {}",
            if cfg!(feature = "media") {
                "on — FFmpeg linked"
            } else {
                "off"
            }
        ),
        // Not conditional: rendering is not a feature. Said anyway, because the
        // splash is where somebody looks when nothing is drawing.
        "compositor: linked — GPU adapter probed on first render".to_owned(),
        format!(
            "zero-copy Viewer: {}",
            if cfg!(all(windows, feature = "shared-texture"))
                || cfg!(all(target_os = "linux", feature = "shared-texture-linux"))
            {
                "shared texture"
            } else {
                "read-back path"
            }
        ),
    ]
}

/// How coarsely playback is currently rendering (K-030/K-171).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgePlaybackTier {
    /// 1 = full, 2 = half, 3 = third, 4 = quarter.
    pub tier: u32,
    /// The render scale that tier means, `1.0 / tier`.
    pub scale: f32,
}

/// The tier in force. A readout, not a setting: the controller measures real
/// render costs and decides, so there is nothing here to set.
#[frb(sync)]
pub fn playback_tier() -> BridgePlaybackTier {
    let tier = crate::realtime::tier();
    BridgePlaybackTier {
        tier,
        scale: crate::realtime::tier_scale(tier),
    }
}

/// Start the controller again, optimistic at full.
///
/// Called when playback stops or the composition changes, so a fresh run does
/// not inherit a tier that a different, heavier comp earned.
#[frb(sync)]
pub fn reset_realtime() -> BridgePlaybackTier {
    crate::realtime::reset();
    playback_tier()
}

/// Render live drags at the Viewer's own resolution instead of the drag budget
/// (K-744, qualifying K-383).
///
/// K-383 caps a drag preview at a 640x360 raster so the picture keeps up with
/// the pointer, and said the reduction needed no flag from the frontend. This
/// is that flag, and only that: `true` renders a dragged frame exactly as a
/// committed one, sharp and as slow as the composition really is. Everything
/// else about a drag is unchanged.
///
/// A setting rather than a render argument, so it cannot be forgotten by a new
/// drag call site — the same reason the reduction itself lives in the worker.
/// The engine holds the live choice with no store behind it, so the settings
/// file carries it and hands it back at boot, as the cache budgets do.
#[frb(sync)]
pub fn set_full_res_drag_previews(full_res: bool) {
    crate::realtime::set_full_res_drags(full_res);
}

/// One rotating autosave beside a project.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAutosave {
    /// 1 is the newest.
    pub slot: u32,
    pub path: String,
}

/// How often Lumit writes a rotating copy of every open project, and how many
/// copies it keeps (docs/10-FILE-FORMAT.md §4).
///
/// `minutes` of 0 turns autosave off, which is a setting a user is entitled to
/// hold. Both values are application settings rather than project data — how
/// often this machine copies your work is a property of the machine — so the
/// frontend owns the file they live in and calls this at boot and on every
/// change. The timer itself is the engine's, because the document is.
///
/// Nothing is written for a project that has not moved since its last save or
/// its own last autosave, and nothing is written for one that has never been
/// saved: the copies live beside the project file, and the crash journal is
/// what covers a project with no file yet.
#[frb(sync)]
pub fn set_autosave(minutes: u32, keep: u32) {
    crate::autosave::schedule(minutes, keep);
}

/// The autosaves beside `project`, newest first.
///
/// An empty list is an ordinary answer, not an error: a project that has never
/// been open long enough to autosave simply has none. Stateless, so a free
/// function — the recovery dialogue runs before anything is loaded.
#[frb(sync)]
pub fn list_autosaves(project: String) -> Vec<BridgeAutosave> {
    let project = std::path::PathBuf::from(project);
    let dir = project
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .join("autosaves");
    let stem = project
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());

    let mut out = Vec::new();
    // Rotation keeps the slots contiguous from 1, so the first gap is the end.
    // The ceiling is belt and braces against a folder somebody filled by hand.
    for slot in 1_u32..=999 {
        let candidate = dir.join(format!("{stem}.autosave-{slot}.lum"));
        if !candidate.is_file() {
            break;
        }
        out.push(BridgeAutosave {
            slot,
            path: candidate.to_string_lossy().into_owned(),
        });
    }
    out
}

/// What a journal replay recovered.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeRecovery {
    /// Ops found in the journal.
    pub found: u32,
    /// Ops that still applied. Fewer than `found` means the journal ran past
    /// what the saved document can take — the replay stops at the first op that
    /// no longer applies rather than skipping it, because every op after one
    /// that failed was written against a document that no longer exists.
    pub replayed: u32,
}

impl ProjectReference {
    /// Write a rotating autosave beside `project_path`, keeping `keep` slots.
    ///
    /// Deliberately does **not** move the project's own path: an autosave is a
    /// safety copy, and the next Save must still write the file the user chose.
    /// The document is rebased against the project folder first, so no
    /// machine-specific path is written into a copy that may be opened
    /// elsewhere.
    ///
    /// The read guard covers the decision and an `Arc` clone of the document,
    /// and is dropped before anything touches the disk: serialising and fsyncing
    /// a project is far too slow to hold a lock across, and a lock held here is
    /// the whole interface waiting (docs/14 §5, and the shape `measure_document`
    /// was corrected into).
    #[frb(sync)]
    pub fn autosave(&self, project_path: String, keep: u32) -> Result<String, BridgeError> {
        let state = self.state()?;
        let (document, target) = {
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let target = if project_path.trim().is_empty() {
                state.path.clone().ok_or(BridgeError::NoProjectPath)?
            } else {
                std::path::PathBuf::from(project_path)
            };
            (state.store.snapshot(), target)
        };

        let dir = target.parent().unwrap_or_else(|| std::path::Path::new(""));
        let doc = lumit_project::rebase_for_save(&document, dir);

        lumit_project::autosave(&doc, &target, keep.max(1) as usize)
            .map(|written| written.to_string_lossy().into_owned())
            .map_err(|_| BridgeError::WriteFailed)
    }

    /// Open `project_path` and replay its crash journal on top of it.
    ///
    /// This is the whole point of the journal: a session that ended badly left
    /// its edits there, and this is what puts them back. The replay stops at the
    /// first op that no longer applies — see [`BridgeRecovery::replayed`].
    #[frb(sync)]
    pub fn restore_journal(&self, project_path: String) -> Result<BridgeRecovery, BridgeError> {
        let path = std::path::PathBuf::from(project_path);
        let (mut doc, _manifest) =
            lumit_project::open(&path).map_err(|_| BridgeError::ReadFailed)?;

        let ops = JournalFile::for_document(doc.id)
            .and_then(|journal| journal.read().ok())
            .unwrap_or_default();
        let found = ops.len() as u32;
        let mut replayed = 0_u32;
        for op in &ops {
            if lumit_core::ops::apply(&mut doc, op).is_err() {
                break;
            }
            replayed += 1;
        }

        let state = self.state()?;
        let mut state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        // The observer is attached to the old store, so the recovered document
        // is installed *through* it rather than replacing it — otherwise every
        // panel would stop hearing about changes the moment recovery ran.
        // Re-arm the journal on the recovered document *before* installing it.
        // The document's identity changed, so the observer's shared handle now
        // points at the wrong file — and every edit from here is journalled
        // against the recovered document or not at all.
        if let Ok(mut journal) = state.journal.lock() {
            *journal = lumit_project::JournalFile::for_document(doc.id);
        }
        state.store.replace_document(doc);
        state.path = Some(path);
        state.media.clear();

        Ok(BridgeRecovery { found, replayed })
    }
}

/// Show a finished file in the desktop's own file manager.
///
/// The export dialogue's *Open folder* is the only caller that asks directly;
/// a queued export with that tick set reveals itself as it lands. Returns
/// whether the request was handed over — a machine with no file manager (a
/// headless CI box, most obviously) says no rather than failing, because
/// nothing depends on the window appearing.
#[frb(sync)]
pub fn reveal_in_folder(path: String) -> bool {
    crate::export::reveal_in_folder(&path)
}
