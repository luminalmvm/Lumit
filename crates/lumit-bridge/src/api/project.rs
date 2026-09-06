use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::Op;
use uuid::Uuid;

use crate::api::{
    composition::{BridgeCompSettings, CompositionReference},
    folder::FolderReference,
    footage::FootageReference,
    project_item::{item_reference, ItemReference},
    state::{WorkerResponseStream, PROJECTS, STREAMS},
    worker_thread, BridgeError,
};

/// Whether undo and redo have anything to do, for greying the menu items.
#[frb(non_opaque)]
pub struct BridgeHistory {
    pub can_undo: bool,
    pub can_redo: bool,
}

/// One row of the History list: what the step is called, and whether it
/// has been undone.
///
/// The name is the engine's English (`Op::name`), translated on arrival like
/// every other engine word — an undone row is still on the list, greyed,
/// until a fresh commit clears the forward history.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHistoryEntry {
    pub name: String,
    pub undone: bool,
}

/// One colour on the project's shelf: four 0–1 channels and an
/// optional name. Empty `name` means unnamed, which is the ordinary case — a
/// shelf is read by eye.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSwatch {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub name: String,
}

impl BridgeSwatch {
    #[frb(ignore)]
    fn of(swatch: &lumit_core::model::Swatch) -> Self {
        let [r, g, b, a] = swatch.colour.0;
        Self {
            r,
            g,
            b,
            a,
            name: swatch.name.clone().unwrap_or_default(),
        }
    }

    #[frb(ignore)]
    fn to_model(&self) -> lumit_core::model::Swatch {
        lumit_core::model::Swatch {
            colour: lumit_core::model::LinearColour([self.r, self.g, self.b, self.a]),
            name: (!self.name.is_empty()).then(|| self.name.clone()),
        }
    }
}

#[derive(Debug, Clone)]
#[frb]
pub struct ProjectReference {
    #[frb(name = "internalid")]
    pub id: Uuid,
}

impl ProjectReference {
    #[frb(ignore)]
    pub fn new(id: Uuid) -> ProjectReference {
        ProjectReference { id }
    }

    #[frb(ignore)]
    pub fn state(
        &self,
    ) -> Result<Arc<std::sync::RwLock<super::state::LumitBridgeState>>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects.get(&self.id).ok_or(BridgeError::InvalidProject)?;

        Ok(project.clone())
    }

    #[frb(sync)]
    pub fn start_worker(&self, on_reponse: WorkerResponseStream) {
        worker_thread::run_worker(self.clone(), on_reponse);
    }

    /// Close this project: forget it in both registries, so every later call
    /// through this reference answers `InvalidProject` — and, with the state
    /// dropped, the request channel its render worker waits on is dropped too.
    /// The worker sees the disconnect, stops, and everything it held — most of
    /// all its renderer, a whole GPU device — is freed with it.
    ///
    /// [`LumitBridgeState::open_project`] already does this wholesale for every
    /// open project; this is the same farewell for one. Closing a project that
    /// is already gone is not an error: it is closed, which is what was asked.
    ///
    /// This is what stops a long-lived process from accumulating one worker and
    /// one GPU device per project it has ever made. The frb test suite was the
    /// proof: a test process makes a project per test, and without a close the
    /// Linux CI runner ran out of memory under the pile of live renderers.
    #[frb(sync)]
    pub fn close(&self) -> Result<(), BridgeError> {
        // One registry at a time, never nested — the lock order rule in
        // `state.rs`. The state's last strong reference is usually the one
        // removed here; binding it keeps the drop (and the worker channel's
        // disconnect) outside both registry locks.
        let removed = {
            let mut p = PROJECTS.write().map_err(|_| BridgeError::WriteFailed)?;
            p.remove(&self.id)
        };
        {
            let mut s = STREAMS.write().map_err(|_| BridgeError::WriteFailed)?;
            s.remove(&self.id);
        }
        // The camera solves go with the project. The `track/` sidecar
        // is untouched, so reopening reads every one of them straight back —
        // this is the session's copy being dropped, not the answer.
        lumit_render::track::clear();
        // The roto mattes go the same way and for the same reason: the
        // `roto/` sidecar is untouched, so reopening reads them back.
        lumit_render::roto::clear();
        // And so do the puppet wireframes the render left for the overlay:
        // they name layers this project owned.
        lumit_render::puppet::clear();
        drop(removed);
        Ok(())
    }

    #[frb(sync)]
    pub fn get_items(&self) -> Result<Vec<ItemReference>, BridgeError> {
        let s = self.state()?;
        let s = s.read().map_err(|_| BridgeError::ReadFailed)?;

        let snapshot = s.store.snapshot();

        // The panel's ROOTS only — items no folder lists. `Document::items` is the
        // flat set of every item in the project, so returning it whole made a
        // filed item appear twice: once at the top level and again under its
        // folder, because the panel recurses through
        // `FolderReference::get_children`. Nesting is that method's job; this one
        // answers "what does the tree start from".
        let filed: std::collections::HashSet<Uuid> = snapshot
            .items
            .iter()
            .filter_map(|i| match i {
                lumit_core::model::ProjectItem::Folder(f) => Some(f.children.iter().copied()),
                _ => None,
            })
            .flatten()
            .collect();

        Ok(snapshot
            .items
            .iter()
            .filter(|i| !filed.contains(&i.id()))
            .map(|i| item_reference(self.id, i))
            .collect())
    }

    /// The name a comp made right now would get, if nobody typed one — "Comp 3"
    /// when the project holds two. What the New composition dialog puts in its
    /// Name field before the user touches it, so the field shows the same name
    /// the engine would have chosen rather than a guess made in Dart.
    #[frb(sync)]
    pub fn next_comp_name(&self) -> Result<String, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(Self::next_comp_name_in(&state.store.snapshot()))
    }

    #[frb(ignore)]
    fn next_comp_name_in(doc: &lumit_core::Document) -> String {
        let existing = doc
            .items
            .iter()
            .filter(|i| matches!(i, lumit_core::model::ProjectItem::Composition(_)))
            .count();
        format!("Comp {}", existing + 1)
    }

    /// Add a folder, as one undo step — the Project panel's bottom-bar Folder
    /// button (docs/07 §3.1).
    ///
    /// Both decisions are the engine's ([`lumit_core::ops::new_folder_ops`]):
    /// a blank name becomes the next unused "Folder N", counted past the names
    /// already taken rather than off the number of folders, and `parent` files
    /// the new folder inside that one. A `parent` that no longer names a folder
    /// leaves it at the panel root rather than erroring — a stale selection is
    /// not a reason to refuse to make a folder.
    ///
    /// The ops are committed as one `Op::Batch`, which is what makes the folder
    /// and its filing arrive and leave together.
    #[frb(sync)]
    pub fn new_folder(
        &self,
        name: String,
        parent: Option<Uuid>,
    ) -> Result<FolderReference, BridgeError> {
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        let (id, ops) = lumit_core::ops::new_folder_ops(&state.store.snapshot(), &name, parent);
        state
            .store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(FolderReference::new(self.id, id))
    }

    /// Add a composition, filed into the Compositions auto-folder, as one undo
    /// step. A blank name gets the next "Comp N".
    ///
    /// `settings` is what the New composition dialog collected — size, rate and
    /// duration. `None` takes the defaults ([`BridgeCompSettings::defaults`]), which
    /// is what every caller that does not ask the user does. `settings.name` is
    /// ignored: the name comes from `name`, so there is one answer to "what is this
    /// comp called" rather than two that can disagree.
    ///
    /// It is one call rather than "create, then apply settings" because that would
    /// be two undo steps for one click, and undoing once would leave a comp behind
    /// at the wrong size.
    ///
    /// The folder is tracked by id, not by name, so renaming or nesting it keeps
    /// it the Compositions folder — the same habit the egui frontend has.
    #[frb(sync)]
    pub fn new_composition(
        &self,
        name: String,
        settings: Option<BridgeCompSettings>,
    ) -> Result<CompositionReference, BridgeError> {
        use lumit_core::model::{Composition, Folder, LinearColour, MotionBlur, ProjectItem};
        use lumit_core::ops::AutoFolderKind;

        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        let doc = state.store.snapshot();

        let name = if name.trim().is_empty() {
            Self::next_comp_name_in(&doc)
        } else {
            name
        };

        let settings = settings.unwrap_or_else(BridgeCompSettings::defaults);
        let (frame_rate, duration) = settings.to_engine().ok_or(BridgeError::InvalidFrameRate)?;

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
            master_volume_db: 0.0,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name,
            width: settings.width.clamp(16, 16384),
            height: settings.height.clamp(16, 16384),
            frame_rate,
            duration,
            // The dialog's own answers: the Background row and the
            // Motion blur section decide these at creation, and a settings
            // block that was never filled in carries the defaults anyway.
            background: LinearColour(settings.background),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: MotionBlur {
                shutter_angle: settings.shutter_angle,
                samples: settings.motion_blur_samples.max(1),
                ..MotionBlur::default()
            },
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;

        // The comp's index has to account for any AddItem already queued ahead of
        // it in this same batch.
        let queued = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        ops.push(Op::AddItem {
            index: doc.items.len() + queued,
            item: Box::new(ProjectItem::Composition(comp)),
        });

        // The folder may have been created earlier in this same batch, so it is
        // absent from `doc` and its children start empty.
        let mut children = doc
            .folder(folder_id)
            .map(|f| f.children.clone())
            .unwrap_or_default();
        children.push(comp_id);
        ops.push(Op::SetFolderChildren {
            folder: folder_id,
            children,
        });

        state
            .store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;

        Ok(CompositionReference::new(self.id, comp_id))
    }

    /// Record `path` as a footage item, as one undo step.
    ///
    /// Importing only *records* the file — it does not decode it or read its size.
    /// It does **ask the probe worker to read it**, which is not the same thing:
    /// the request returns immediately and the file's statistics are read on a
    /// background thread, so by the time the user drags the item into a
    /// composition — `add_footage_layer`, which is synchronous and needs the
    /// media's real size and length — the answer is usually already waiting
    /// (`crate::probe`).
    /// Footage has no auto-folder (only solids and comps do), so the item lands at
    /// the panel root, matching the egui frontend exactly.
    ///
    /// The bare file name becomes the relative path; saving rebases it against the
    /// project folder.
    ///
    /// **A still that has numbered neighbours imports as one image sequence**,
    /// not as one item per file: picking `frames0001.png` out of a
    /// folder of two thousand brings in the whole run, named for its span and
    /// pointed at its first file. Two guards keep that from surprising anyone —
    /// only still-image formats are considered at all (a folder of numbered mp4s
    /// is a folder of clips), and a numbered file with no numbered neighbours
    /// stays the single still it is. Picking a *second* file of a run that is
    /// already in the project hands back the item that is already there, so
    /// selecting the whole folder still imports it once.
    #[frb(sync)]
    pub fn import_footage(&self, path: String) -> Result<FootageReference, BridgeError> {
        use lumit_core::model::{FootageItem, MediaRef, ProjectItem, SequenceRef};

        if path.trim().is_empty() {
            return Err(BridgeError::MediaPathUnresolved);
        }
        let picked = std::path::PathBuf::from(&path);

        // A run of one is a still, and a still is not a sequence: it decodes,
        // saves and relinks exactly as it did before this existed.
        //
        // What is kept is what the import needs: the run's first file and the
        // name to file it under. A build with no decoder finds no run at all —
        // recognising a sequence and playing one are the same crate's work, so
        // a build that cannot play a run does not pretend to import one, and
        // the picked file comes in as the single still it is.
        #[cfg(feature = "media")]
        let run: Option<(std::path::PathBuf, String)> = lumit_media::sequence::is_still(&picked)
            .then(|| lumit_media::sequence::detect(&picked))
            .flatten()
            .filter(|run| run.count > 1)
            .map(|run| (run.first.clone(), run.display_name()));
        #[cfg(not(feature = "media"))]
        let run: Option<(std::path::PathBuf, String)> = None;

        let (file, name) = match &run {
            Some((first, display)) => (first.clone(), display.clone()),
            None => (
                picked,
                std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "footage".into()),
            ),
        };
        let on_disk = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());

        let item = FootageItem {
            id: Uuid::now_v7(),
            name,
            media: MediaRef {
                relative_path: on_disk,
                absolute_path: file.to_string_lossy().into_owned(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            sequence: run.as_ref().map(|_| SequenceRef::default()),
            extra: serde_json::Map::new(),
            colour_space: None,
        };
        let item_id = item.id;

        let state = self.state()?;
        {
            let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
            let doc = state.store.snapshot();

            // Selecting a whole folder calls this once per file. Every call
            // after the first finds the run already imported and returns it,
            // rather than filing two thousand items that all mean one shot.
            if run.is_some() {
                if let Some(existing) = doc.items.iter().find_map(|i| match i {
                    ProjectItem::Footage(f)
                        if f.sequence.is_some()
                            && std::path::Path::new(&f.media.absolute_path) == file =>
                    {
                        Some(f.id)
                    }
                    _ => None,
                }) {
                    return Ok(FootageReference::new(self.id, existing));
                }
            }

            let index = doc.items.len();
            state
                .store
                .commit(Op::AddItem {
                    index,
                    item: Box::new(ProjectItem::Footage(item)),
                })
                .map_err(BridgeError::OpError)?;
        }

        // Outside the lock, and after the item exists: start reading the file
        // now so the first question about it is a look-up.
        #[cfg(feature = "media")]
        crate::probe::request(lumit_media::MediaSource {
            path: file,
            sequence_fps: run.map(|_| {
                let r = SequenceRef::default().frame_rate;
                (r.num(), r.den())
            }),
        });
        // Nothing to probe without a decoder, and the call says so itself.
        #[cfg(not(feature = "media"))]
        crate::probe::request(file);

        Ok(FootageReference::new(self.id, item_id))
    }

    /// Save to `path`, or to wherever the project was last saved when `path` is
    /// empty. Answers the path actually written, so Dart can show it and stop
    /// asking where to put the file.
    ///
    /// Deliberately **not** `#[frb(sync)]`: this writes a whole document to disk
    /// and fsyncs it, so it must not run on Dart's UI isolate. Budget S5
    /// (docs/13 §2.1) asks for a stress-document save to be non-blocking, and an
    /// async frb call is that for free.
    ///
    /// Media paths are rebased against the destination directory before writing,
    /// so a project saved somewhere new keeps relative links that work.
    /// A successful save clears the crash journal: the journal covers work
    /// *between* saves, so once the document is on disk it is redundant.
    pub fn save(&self, path: String) -> Result<String, BridgeError> {
        let project = self.state()?;

        // **The destination, the revision and an `Arc` clone of the document
        // under the guard, then let it go.** Serialising a whole project and
        // fsyncing it is seconds of work on a large one, and a guard held
        // across that is the interface stopped — a Rust `RwLock` turns new
        // readers away once a writer is waiting, so every `#[frb(sync)]`
        // question queues behind the disk (docs/14 §1, the freeze of
        // 9d96a24f). This is the shape the autosave sweep already writes in
        // (`crate::autosave::sweep_one`); the lock comes back at the end only
        // to record where the file went.
        let (document, target, revision) = {
            let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
            let target = if path.trim().is_empty() {
                // Never saved and no path given: the caller has to pick one.
                state.path.clone().ok_or(BridgeError::NoProjectPath)?
            } else {
                let mut target = std::path::PathBuf::from(path);
                // A name typed without any extension is a name, not a choice:
                // the save dialogue on Windows hands back exactly what was
                // typed, and a bare `my comp` would sit on disk unopenable by
                // its own file type. A *different* extension the user typed is
                // a choice, and stands.
                if target.extension().is_none() {
                    target.set_extension("lum");
                }
                target
            };
            (state.store.snapshot(), target, state.store.revision())
        };

        // Everything from here is outside the lock.
        let dir = target.parent().unwrap_or_else(|| std::path::Path::new(""));
        let doc = lumit_project::rebase_for_save(&document, dir);
        lumit_project::save(&doc, &target).map_err(|_| BridgeError::WriteFailed)?;

        let written = target.to_string_lossy().into_owned();
        let mut state = project.write().map_err(|_| BridgeError::WriteFailed)?;

        // The journal covers work *between* saves, so once the document is on
        // disk it is redundant — and keeping it would mean a later recovery
        // replaying edits the saved file already contains.
        if let Ok(mut journal) = state.journal.lock() {
            if let Some(file) = journal.as_ref() {
                let _ = file.clear();
            }
            *journal = None;
        }
        state.path = Some(target);
        // The revision the file *contains*, read before the write — not
        // whatever the store is on now. An edit made while the disk was busy
        // is not in the file, and must still read as unsaved.
        state.saved_revision = revision;
        Ok(written)
    }

    /// Whether the document has moved since it was last saved (or opened).
    /// The status bar's saved/unsaved readout. An undo after a save reads as
    /// dirty: the revision moved, and only a save proves the file matches.
    #[frb(sync)]
    pub fn is_dirty(&self) -> Result<bool, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.revision() != state.saved_revision)
    }

    /// Where this project was last saved, or null when it never has been. The
    /// menu bar needs it to decide between Save and Save as.
    #[frb(sync)]
    pub fn path(&self) -> Result<Option<String>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()))
    }

    /// Where *this project* parks its rendered frames, overriding the
    /// application-wide choice — or `None` when it follows that choice, which is
    /// the ordinary case (docs/06-RENDER-PIPELINE.md §5.4).
    ///
    /// Returned as the enum plus a folder, the same pair
    /// [`Self::set_cache_location`] takes; the folder is empty unless the
    /// location is `Custom`.
    #[frb(sync)]
    pub fn cache_location(
        &self,
    ) -> Result<Option<crate::api::cache::BridgeProjectCacheLocation>, BridgeError> {
        use lumit_core::model::CacheLocation;
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state
            .store
            .snapshot()
            .cache_location
            .as_ref()
            .map(|held| match held {
                CacheLocation::AppData => crate::api::cache::BridgeProjectCacheLocation {
                    location: crate::api::cache::BridgeCacheLocation::AppData,
                    folder: String::new(),
                },
                CacheLocation::BesideProject => crate::api::cache::BridgeProjectCacheLocation {
                    location: crate::api::cache::BridgeCacheLocation::BesideProject,
                    folder: String::new(),
                },
                CacheLocation::Custom { folder } => crate::api::cache::BridgeProjectCacheLocation {
                    location: crate::api::cache::BridgeCacheLocation::Custom,
                    folder: folder.clone(),
                },
            }))
    }

    /// Give this project its own cache location, or clear it so the project
    /// follows the application-wide choice again (`location: None`).
    ///
    /// An ordinary op, so it is undoable, journalled, and saved inside the `.lum`
    /// — which is the point of it being in the document at all: the choice travels
    /// with a copy of the project and survives being opened on another machine.
    /// Nothing already cached is moved or deleted; the frames in the old folder
    /// simply stop being addressed, and that folder may be deleted by hand at any
    /// time.
    #[frb(sync)]
    pub fn set_cache_location(
        &self,
        location: Option<crate::api::cache::BridgeProjectCacheLocation>,
    ) -> Result<(), BridgeError> {
        use lumit_core::model::CacheLocation;
        let location = location.and_then(|chosen| match chosen.location {
            crate::api::cache::BridgeCacheLocation::AppData => Some(CacheLocation::AppData),
            crate::api::cache::BridgeCacheLocation::BesideProject => {
                Some(CacheLocation::BesideProject)
            }
            // A custom location with no folder chosen is not a location: leave
            // the project following the application rather than pointing its
            // cache at nothing.
            crate::api::cache::BridgeCacheLocation::Custom if chosen.folder.is_empty() => None,
            crate::api::cache::BridgeCacheLocation::Custom => Some(CacheLocation::Custom {
                folder: chosen.folder,
            }),
        });
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(Op::SetCacheLocation { location })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// How hard the renderer works at the edges of transformed layers, as the
    /// number of coverage samples per pixel: 1, 2, 4 or 8, where 1 is off
    /// (docs/impl/anti-aliasing.md).
    ///
    /// The project's own setting, exactly as stored — **what the current
    /// machine can actually draw is a separate question**, answered by
    /// [`Self::anti_aliasing_in_use`]. Keeping the two apart is what stops a
    /// card that cannot manage the asked-for count from quietly rewriting the
    /// project when it is opened.
    #[frb(sync)]
    pub fn anti_aliasing(&self) -> Result<u32, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.snapshot().anti_aliasing.samples())
    }

    /// The count this machine is actually drawing with — the project's setting
    /// resolved against what the graphics card offers.
    ///
    /// Equal to [`Self::anti_aliasing`] on any adapter that can manage what was
    /// asked for, which is the ordinary case. Where it differs, the difference
    /// is a fact about the machine and never an error: the Settings row shows
    /// what is being used beside what is set, in the calm voice
    /// (docs/15-DESIGN.md), and the project keeps the value its author chose.
    #[frb(sync)]
    pub fn anti_aliasing_in_use(&self) -> Result<u32, BridgeError> {
        let asked = self.anti_aliasing()?;
        // The adapter's own answer where one has been opened, and the
        // project's setting until then — never a lock on the renderer behind a
        // panel's read.
        Ok(lumit_render::adapter_sample_count(asked).unwrap_or(asked))
    }

    /// Set how hard the renderer works at the edges of transformed layers.
    ///
    /// Takes a sample count — 1, 2, 4 or 8. Anything else reads as 1 (off)
    /// rather than failing: an unknown count is not a reason to refuse an edit.
    /// An ordinary op, so it is undoable, journalled and saved in the `.lum`,
    /// which is the point of it living in the document — it changes what the
    /// comp looks like, so it must travel with the file and match on another
    /// machine.
    #[frb(sync)]
    pub fn set_anti_aliasing(&self, samples: u32) -> Result<(), BridgeError> {
        let anti_aliasing = lumit_core::model::AntiAliasing::from_samples(samples);
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(Op::SetAntiAliasing { anti_aliasing })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// How many bits a channel this project's compositor works in — 8, 16 or
    /// 32 (docs/06 §3.4).
    #[frb(sync)]
    pub fn colour_depth(&self) -> Result<u32, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.snapshot().colour_depth.bits())
    }

    /// The depth this machine is actually working at — the project's setting
    /// resolved against what the graphics card offers.
    ///
    /// Equal to [`Self::colour_depth`] on any adapter that can sample a
    /// thirty-two-bit float texture, which is nearly all of them. Where it
    /// differs the difference is a fact about the machine and never an error,
    /// exactly as with the sample count beside it: the Settings row shows what
    /// is being used beside what is set, and the project keeps the value its
    /// author chose.
    #[frb(sync)]
    pub fn colour_depth_in_use(&self) -> Result<u32, BridgeError> {
        let asked = self.colour_depth()?;
        Ok(lumit_render::adapter_colour_depth(asked).unwrap_or(asked))
    }

    /// Set how many bits a channel the compositor works in.
    ///
    /// Takes 8, 16 or 32. Anything else reads as 16 rather than failing: an
    /// unknown depth is not a reason to refuse an edit. An ordinary op, so it
    /// is undoable, journalled and saved in the `.lum`.
    #[frb(sync)]
    pub fn set_colour_depth(&self, bits: u32) -> Result<(), BridgeError> {
        let colour_depth = lumit_core::model::ColourDepth::from_bits(bits);
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(Op::SetColourDepth { colour_depth })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// The project's colour shelf, in the order the colours were kept.
    ///
    /// Empty for a project nobody has kept a colour in. The picker reads this
    /// when it opens — a shelf is a handful of colours, so there is nothing to
    /// stream and nothing to cache.
    #[frb(sync)]
    pub fn project_swatches(&self) -> Result<Vec<BridgeSwatch>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state
            .store
            .snapshot()
            .swatches
            .iter()
            .map(BridgeSwatch::of)
            .collect())
    }

    /// Replace the colour shelf whole — which is how a colour is kept and how
    /// one is forgotten (`Op::SetProjectSwatches`).
    ///
    /// One ordinary op, so keeping a colour is one undo step and travels in the
    /// `.lum` with everything else.
    #[frb(sync)]
    pub fn set_project_swatches(&self, swatches: Vec<BridgeSwatch>) -> Result<(), BridgeError> {
        let swatches = swatches.iter().map(BridgeSwatch::to_model).collect();
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(Op::SetProjectSwatches { swatches })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// The project-wide *use proxies* master switch.
    ///
    /// On — the default, and what every project written before proxies existed
    /// opens as — means each item's own tick decides. Off reads the originals
    /// everywhere, however many proxies are attached: the one switch for "show
    /// me what I am actually delivering".
    ///
    /// **An export ignores it entirely** and delivers full resolution unless
    /// asked otherwise (`BridgeExportSpec::use_proxies`), so turning proxies on
    /// to work cannot quietly ship the small picture.
    #[frb(sync)]
    pub fn use_proxies(&self) -> Result<bool, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.snapshot().use_proxies)
    }

    /// Set the project-wide *use proxies* switch. An ordinary op, so it is
    /// undoable and travels in the `.lum` — it changes which file the pixels
    /// come out of, which is an edit like any other.
    #[frb(sync)]
    pub fn set_use_proxies(&self, use_proxies: bool) -> Result<(), BridgeError> {
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(Op::SetUseProxies { use_proxies })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// How the interface was arranged when this project was last saved, as the
    /// JSON the frontend itself wrote, or `None` for a project that has
    /// never carried one.
    ///
    /// The engine never looks inside it. It is the frontend's own record,
    /// travelling in the `.lum` so a project shared with someone else opens
    /// arranged the way its author left it.
    #[frb(sync)]
    pub fn ui_state(&self) -> Result<Option<String>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state
            .store
            .snapshot()
            .ui_state
            .as_ref()
            .map(|value| value.to_string()))
    }

    /// Record the arrangement to be written into the file on the next save.
    /// `None`, or JSON that does not parse, clears it rather than failing: a
    /// frontend that cannot describe itself must not be able to stop a save.
    ///
    /// Not an op — see `DocumentStore::set_ui_state`. It is not undoable, and it
    /// does not mark the project as having unsaved changes, because moving a
    /// panel is not an edit to the work.
    #[frb(sync)]
    pub fn set_ui_state(&self, ui_state: Option<String>) -> Result<(), BridgeError> {
        let parsed = ui_state.and_then(|json| serde_json::from_str(&json).ok());
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state.store.set_ui_state(parsed);
        Ok(())
    }

    /// Whether there is anything to undo or redo, for greying the menu items.
    #[frb(sync)]
    pub fn history(&self) -> Result<BridgeHistory, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(BridgeHistory {
            can_undo: state.store.can_undo(),
            can_redo: state.store.can_redo(),
        })
    }

    /// The History list: every step still applied, oldest first, then
    /// every step that has been undone, in the order redoing would put them
    /// back. [`Self::applied_steps`] says where the two halves meet.
    #[frb(sync)]
    pub fn history_entries(&self) -> Result<Vec<BridgeHistoryEntry>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state
            .store
            .history()
            .into_iter()
            .map(|e| BridgeHistoryEntry {
                name: e.name.to_string(),
                undone: e.undone,
            })
            .collect())
    }

    /// How many of [`Self::history_entries`]'s rows are applied — the point the
    /// document stands at on the list.
    #[frb(sync)]
    pub fn applied_steps(&self) -> Result<u32, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(u32::try_from(state.store.applied_steps()).unwrap_or(u32::MAX))
    }

    /// Take the document to the point where exactly `applied` steps have been
    /// applied — what clicking a row of the History list does.
    ///
    /// Undo and redo in a loop, so a jump reaches only states the keyboard
    /// could; a number past either end stops at that end rather than failing.
    #[frb(sync)]
    pub fn jump_history(&self, applied: u32) -> Result<(), BridgeError> {
        let s = self.state()?;
        let s = s.read().map_err(|_| BridgeError::ReadFailed)?;
        s.store
            .jump_to(applied as usize)
            .map_err(BridgeError::OpError)
    }

    #[frb(sync)]
    pub fn undo(&self) -> Result<(), BridgeError> {
        let s = self.state()?;
        let s = s.read().map_err(|_| BridgeError::ReadFailed)?;

        s.store.undo().map_err(BridgeError::OpError)?;

        Ok(())
    }

    #[frb(sync)]
    pub fn redo(&self) -> Result<(), BridgeError> {
        let s = self.state()?;
        let s = s.read().map_err(|_| BridgeError::ReadFailed)?;

        s.store.redo().map_err(BridgeError::OpError)?;

        Ok(())
    }

    /// Begin an undo group: everything committed until [`Self::end_undo_group`]
    /// becomes **one** undo step (docs/07 §4.7).
    ///
    /// For a gesture that is several ops by construction, because the ops are
    /// as coarse as a whole property's animation: stretching a selected block
    /// of keyframes across two layers, reversing it, staggering it, pasting a
    /// clipboard that came off three properties. The user made one drag or
    /// pressed one button, and expects one Ctrl-Z.
    ///
    /// The edits still land as they are made — only the history waits — so a
    /// read taken part-way through a group sees the document as it is.
    ///
    /// **Pair the two calls.** A group left open records nothing, so the
    /// frontend closes it in a `finally`. Calls nest: an inner pair inside an
    /// outer one folds into the outer group rather than closing it early.
    #[frb(sync)]
    pub fn begin_undo_group(&self) -> Result<(), BridgeError> {
        let s = self.state()?;
        let s = s.read().map_err(|_| BridgeError::ReadFailed)?;
        s.store.begin_undo_group();
        Ok(())
    }

    /// Close the group [`Self::begin_undo_group`] opened. Ending one that was
    /// never begun does nothing.
    #[frb(sync)]
    pub fn end_undo_group(&self) -> Result<(), BridgeError> {
        let s = self.state()?;
        let s = s.read().map_err(|_| BridgeError::ReadFailed)?;
        s.store.end_undo_group();
        Ok(())
    }
}
