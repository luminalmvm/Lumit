//! `AppState` document lifecycle: edit history (commit/undo/redo), open,
//! save, autosave and recovery.

use super::*;

impl AppState {
    fn report<T>(&mut self, r: Result<T, impl std::fmt::Display>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// Back to auto-fit for the value graph (K-079): drop any manual y-range
    /// (and the plot height it was framed at) so the graph re-fits the curve
    /// continuously. Called when the graphed channel or lens changes — a fresh
    /// channel always starts fitted — and by the Fit toggle switching back on.
    pub fn graph_reset_fit(&mut self) {
        self.graph_auto_fit = true;
        self.graph_view_y = None;
        self.graph_view_h = None;
    }

    /// Arm the eyedropper on a target parameter: the next Viewer click samples
    /// and commits to it. Resets the averaging region to a single pixel and
    /// leaves it unprimed for one frame (see [`AppState::eyedropper_primed`]).
    pub fn arm_eyedropper(&mut self, target: EyedropperTarget) {
        self.eyedropper = Some(target);
        self.eyedropper_region = 1;
        self.eyedropper_primed = false;
    }

    /// All document mutation funnels through here: commit, journal, dirty.
    pub fn commit(&mut self, op: Op) {
        match self.store.commit(op.clone()) {
            Ok(_) => {
                self.dirty = true;
                if let Some(journal) = &self.journal {
                    if let Err(e) = journal.append(&op) {
                        self.error = Some(format!("journal: {e}"));
                    }
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn undo(&mut self) {
        match self.store.undo() {
            Ok(Some(_)) => self.dirty = true,
            Ok(None) => {}
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn redo(&mut self) {
        match self.store.redo() {
            Ok(Some(_)) => self.dirty = true,
            Ok(None) => {}
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn install(&mut self, doc: Document, path: Option<PathBuf>, dirty: bool) {
        #[cfg(feature = "media")]
        for item in &doc.items {
            if let ProjectItem::Footage(f) = item {
                self.media
                    .spawn_probe(f.id, PathBuf::from(&f.media.absolute_path));
            }
        }
        self.journal = JournalFile::for_document(doc.id);
        self.selected_comp = doc.items.iter().find_map(|i| match i {
            ProjectItem::Composition(c) => Some(c.id),
            _ => None,
        });
        // Open the first comp as the sole Timeline tab; the rest open on demand.
        self.open_comps = self.selected_comp.into_iter().collect();
        self.preview_comp = None;
        self.preview_item = None;
        self.store = DocumentStore::new(doc);
        self.path = path;
        self.dirty = dirty;
        self.comp_counter = 0;
    }

    pub fn new_project(&mut self) {
        if let Some(journal) = &self.journal {
            let _ = journal.clear();
        }
        self.install(Document::new(), None, false);
    }

    pub fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Lumit project", &["kir"])
            .pick_file();
        if let Some(path) = picked {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        let Some((doc, _manifest)) = self.report(lumit_project::open(path)) else {
            return;
        };
        // Crash recovery: a non-empty journal for this document means the last
        // session ended without a save (docs/10-FILE-FORMAT.md §4).
        let ops = JournalFile::for_document(doc.id)
            .and_then(|j| j.read().ok())
            .unwrap_or_default();
        if ops.is_empty() {
            self.install(doc, Some(path.to_owned()), false);
        } else {
            self.pending_recovery = Some(PendingRecovery {
                doc,
                path: path.to_owned(),
                ops,
            });
        }
    }

    pub fn resolve_recovery(&mut self, recover: bool) {
        let Some(pending) = self.pending_recovery.take() else {
            return;
        };
        let mut doc = pending.doc;
        if recover {
            let mut replayed = 0usize;
            for op in &pending.ops {
                if lumit_core::ops::apply(&mut doc, op).is_err() {
                    break;
                }
                replayed += 1;
            }
            self.install(doc, Some(pending.path), true);
            if replayed < pending.ops.len() {
                self.error = Some(format!(
                    "recovered {replayed} of {} changes; the rest could not be replayed",
                    pending.ops.len()
                ));
            }
            // Journal stays until the user saves.
        } else {
            if let Some(journal) = JournalFile::for_document(doc.id) {
                let _ = journal.clear();
            }
            self.install(doc, Some(pending.path), false);
        }
    }

    pub fn save(&mut self) {
        let path = match &self.path {
            Some(p) => Some(p.clone()),
            None => rfd::FileDialog::new()
                .add_filter("Lumit project", &["kir"])
                .set_file_name("untitled.lum")
                .save_file(),
        };
        let Some(path) = path else { return };
        let doc = self.store.snapshot();
        if self.report(lumit_project::save(&doc, &path)).is_some() {
            if let Some(journal) = &self.journal {
                let _ = journal.clear();
            }
            self.path = Some(path);
            self.dirty = false;
        }
    }

    /// Autosave if due. `interval_secs` and `keep` come from Settings →
    /// General (defaulting to [`AUTOSAVE_INTERVAL_SECS`]/[`AUTOSAVE_KEEP`]);
    /// `interval_secs` is floored at 1 so a zero can never busy-save.
    pub fn autosave_tick(&mut self, interval_secs: u64, keep: usize) {
        if self.dirty
            && self.path.is_some()
            && self.last_autosave.elapsed().as_secs() >= interval_secs.max(1)
        {
            self.last_autosave = Instant::now();
            if let Some(path) = self.path.clone() {
                let doc = self.store.snapshot();
                let _ = self.report(lumit_project::autosave(&doc, &path, keep.max(1)));
            }
        }
    }
}
