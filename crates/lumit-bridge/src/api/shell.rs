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

use crate::api::{project::ProjectReference, BridgeError};
use crate::frb_generated::StreamSink;

/// What this build can truthfully say about itself at load time.
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

/// How coarsely playback is currently rendering.
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

/// Render live drags at the Viewer's own resolution instead of the drag budget.
///
/// The drag budget caps a preview at a 640x360 raster so the picture keeps up
/// with the pointer, and normally no flag from the frontend touches it. This
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

/// Let footage be decoded by the graphics card's video unit, or keep it on the
/// processor.
///
/// On is quicker. Off is the way out on a machine whose graphics driver
/// misbehaves with it. It reaches footage opened from here on, so a project
/// already open wants reopening. Held the way the drag setting above is.
#[frb(sync)]
pub fn set_hardware_decode(on: bool) {
    #[cfg(feature = "media")]
    lumit_media::set_hardware_decode(on);
    #[cfg(not(feature = "media"))]
    let _ = on;
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
    /// True when the replay was stopped. The project and its journal are then
    /// as they were, so the edits can still be restored.
    pub cancelled: bool,
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

    /// Whether the journal holds edits from a run that never closed this
    /// project, which is a crash or a power cut. Asked once, as the project
    /// opens, to decide whether to offer them back.
    #[frb(sync)]
    #[must_use]
    pub fn ended_badly(&self) -> bool {
        self.journal_file()
            .is_some_and(|journal| journal.ended_badly())
    }

    /// Note that the project is being left on purpose, for the application
    /// quitting. Closing or replacing a project notes it by itself.
    #[frb(sync)]
    pub fn note_clean_exit(&self) {
        // Quitting drops nothing, so sharing is let go of as a close does it.
        crate::api::share::stop(self.id);
        if let Ok(state) = self.state() {
            if let Ok(state) = state.read() {
                crate::api::state::discard_unsaved_journal(&state);
            }
        }
    }

    /// Throw away the edits a crash left in the journal, for somebody who was
    /// offered them and said no.
    #[frb(sync)]
    pub fn discard_journal(&self) {
        if let Some(journal) = self.journal_file() {
            let _ = journal.clear();
        }
    }

    fn journal_file(&self) -> Option<lumit_project::JournalFile> {
        let state = self.state().ok()?;
        let state = state.read().ok()?;
        let journal = state.journal.lock().ok()?;
        journal.clone()
    }

    /// Open `project_path` and replay its crash journal on top of it.
    ///
    /// This is the whole point of the journal: a session that ended badly left
    /// its edits there, and this is what puts them back. The replay stops at the
    /// first op that no longer applies — see [`BridgeRecovery::replayed`].
    ///
    /// Not sync: it reads the whole project again. `on_progress` hears the
    /// fraction of the edits replayed, and
    /// [`crate::api::import::cancel_import`] stops it before anything is
    /// swapped.
    pub fn restore_journal(
        &self,
        project_path: String,
        on_progress: Option<StreamSink<f64>>,
    ) -> Result<BridgeRecovery, BridgeError> {
        crate::packing::begin();
        let path = std::path::PathBuf::from(project_path);
        let (mut doc, _manifest) =
            lumit_project::open(&path).map_err(|_| BridgeError::ReadFailed)?;

        let ops = crate::api::state::journal_file(doc.id, Some(&path))
            .and_then(|journal| journal.read().ok())
            .unwrap_or_default();
        let found = ops.len() as u32;
        let mut replayed = 0_u32;
        let mut report = crate::packing::progress(|fraction| {
            if let Some(sink) = &on_progress {
                let _ = sink.add(fraction);
            }
        });
        for op in &ops {
            if !report(u64::from(replayed), u64::from(found))
                || lumit_core::ops::apply(&mut doc, op).is_err()
            {
                break;
            }
            replayed += 1;
        }
        // The switch for packing every save, as an open leaves it: believed
        // only when this machine switched it on for this file.
        let auto_pack_here = doc.auto_pack && crate::packing::auto_packs(doc.id, &path);
        doc.auto_pack = auto_pack_here;
        // Packed footage whose file has gone is read from the copy the open
        // made. Nothing is copied here unless that copy has been deleted.
        let dir = path.parent().unwrap_or_else(|| std::path::Path::new(""));
        crate::packing::restore(&mut doc, &path, dir, |_| {});
        if crate::packing::cancelled() {
            return Ok(BridgeRecovery {
                found,
                replayed,
                cancelled: true,
            });
        }

        // The document is about to be swapped for another, which nobody else
        // in a shared project would hear of. Sharing is let go of as it is
        // when a project closes, and starting it again merges with whoever
        // was here.
        crate::api::share::stop(self.id);
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
            *journal = crate::api::state::journal_file(doc.id, Some(&path));
        }
        state.store.replace_document(doc);
        state.auto_pack_here = auto_pack_here;
        state.path = Some(path);
        state.media.clear();

        Ok(BridgeRecovery {
            found,
            replayed,
            cancelled: false,
        })
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
