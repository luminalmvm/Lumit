//! The command line's [`Host`]: one project open with no window.
//!
//! It opens, saves and exports the way Lumit does, through the same crates:
//! packed footage is read out and carried over, footage is found again by
//! the resolver, imported and placed by the bridge's own code, and exports
//! go through the one exporter. What it leaves out is everything a window is
//! for: no journal, no autosave, no cache on disk.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lumit_core::model::{Composition, Document, FootageItem, Layer, ProjectItem};
use lumit_core::time::CompTime;
use lumit_core::{ops::Op, store::DocumentStore};
use lumit_project::OpenClaim;
use lumit_render::export::{ExportEvent, ExportHandle, ExportSpec};
use lumit_script::{ExportState, ExportStatus, Host, LibraryTemplate, ScriptError};
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::project::ProjectReference;
use crate::api::state::LumitBridgeState;
use crate::api::streams::BridgeStreamsArrival;
use crate::api::template::library;
use crate::api::BridgeError;

/// How long `export.status` waits for news of a running export, so a
/// script asking in a loop is not spinning.
const STATUS_WAIT: Duration = Duration::from_millis(100);
/// How long a script sent over MCP may run. Nobody is there to press Stop,
/// so one that loops for ever stops here instead of holding the program.
const SCRIPT_TIME: Duration = Duration::from_secs(60);

pub struct Headless {
    store: Arc<DocumentStore>,
    /// Where the project was opened from or last saved to.
    path: RefCell<Option<PathBuf>>,
    /// The file the project was opened from. A save copies the footage it
    /// packs out of this one.
    archive: Option<PathBuf>,
    /// The files this program is saving to, so Lumit does not open one of
    /// them meanwhile. Let go when the program ends.
    claims: RefCell<Vec<OpenClaim>>,
    exports: RefCell<Vec<Export>>,
    /// The graphics template's own comp, for a project opened from a
    /// `.lumt` file. The comp chosen when nobody names one.
    main: Option<Uuid>,
    plugins_scanned: Cell<bool>,
    /// Keeps the folder packed footage was read out to from being cleared
    /// by a Lumit tidying up.
    _read_out: Option<File>,
}

struct Export {
    /// `None` while it waits its turn.
    handle: Option<ExportHandle>,
    /// A row's version waiting its turn: the project to export it from, and
    /// the comp.
    waiting: Option<(Arc<Document>, Uuid)>,
    status: ExportStatus,
    /// The encoder the export settled on, once it has.
    encoder: Option<&'static str>,
}

impl Headless {
    /// A new, empty project with no file yet.
    pub fn new_project() -> Self {
        Self::with(Document::new(), None, None)
    }

    /// A project from a document in hand, with no file.
    pub fn from_document(doc: Document) -> Self {
        Self::with(doc, None, None)
    }

    /// Open the project saved at `path`, or the graphics template in a
    /// `.lumt` file as a project of its own. Also answers the names of the
    /// footage items whose files could not be found.
    pub fn open(path: &Path) -> Result<(Self, Vec<String>), String> {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        library::register_fonts();
        let template = lumit_project::template_file::EXTENSION;
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(template))
        {
            // Its footage is read out where Lumit reads it out to. It has no
            // project file, so a save needs a path and is never over it.
            let (mut doc, info) = library::opened(&path).map_err(|_| {
                format!(
                    "{} could not be opened as a graphics template.",
                    path.display()
                )
            })?;
            let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let (_, missing) = lumit_project::resolve_all_media(&mut doc, &dir, &[]);
            let mut host = Self::with(doc, None, None);
            host.main = Some(info.comp);
            return Ok((host, missing));
        }
        let (mut doc, _) = lumit_project::open(&path)
            .map_err(|e| format!("{} could not be opened: {e}.", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        // Packed footage first, so the resolver finds those items placed.
        let read_out = read_out_packed(&mut doc, &path, &dir);
        let (_, missing) = lumit_project::resolve_all_media(&mut doc, &dir, &[]);
        let host = Self::with(doc, Some(path), read_out);
        // What an open in Lumit reads back besides the document: the solves,
        // mattes and planes its analyses made, and where its simulations are
        // baked. Without them an export draws none of those.
        crate::project_cache::point_bakes_at(&host.bridge_state());
        lumit_render::sent::warm(&host.store.snapshot());
        Ok((host, missing))
    }

    fn with(doc: Document, path: Option<PathBuf>, read_out: Option<File>) -> Self {
        Self {
            store: Arc::new(DocumentStore::new(doc)),
            archive: path.clone(),
            path: RefCell::new(path),
            claims: RefCell::default(),
            exports: RefCell::default(),
            main: None,
            plugins_scanned: Cell::new(false),
            _read_out: read_out,
        }
    }

    /// Hold `target` for this program, so Lumit does not open it while it
    /// is being written. Refused when Lumit has it open already: two
    /// programs writing one project lose each other's work.
    pub fn claim(&self, target: &Path) -> Result<(), ScriptError> {
        if self.claims.borrow().iter().any(|c| c.covers(target)) {
            return Ok(());
        }
        match OpenClaim::take(target) {
            Ok(Some(claim)) => self.claims.borrow_mut().push(claim),
            // Nowhere to keep a claim is no reason to refuse.
            Ok(None) => {}
            Err(holder) if holder == std::process::id() => {}
            Err(_) => {
                return Err(ScriptError::Failed(format!(
                    "{} is open in Lumit, or in another lumit-cli. Close it there, or save under another name.",
                    target.display()
                )))
            }
        }
        Ok(())
    }

    /// Where a save to `path`, or to the project's own file, would go.
    pub fn save_target(&self, path: Option<&Path>) -> Result<PathBuf, ScriptError> {
        let mut target = match path {
            Some(path) => std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
            None => self.path.borrow().clone().ok_or_else(|| {
                ScriptError::BadArgs("The project has no file yet, so a save needs a path.".into())
            })?,
        };
        // A name with no extension is a name, as in Lumit's own save.
        if target.extension().is_none() {
            target.set_extension("lum");
        }
        Ok(target)
    }

    /// Every comp in the project, in the Project panel's order.
    pub fn comps(&self) -> Vec<(Uuid, String)> {
        let doc = self.store.snapshot();
        doc.items
            .iter()
            .filter_map(|item| match item {
                ProjectItem::Composition(c) => Some((c.id, c.name.clone())),
                _ => None,
            })
            .collect()
    }

    /// The comp called `key`, by id or name. With no key, the project's only
    /// comp.
    pub fn find_comp(&self, key: Option<&str>) -> Result<(Uuid, String), String> {
        let doc = self.store.snapshot();
        let comps: Vec<&Composition> = doc
            .items
            .iter()
            .filter_map(|item| match item {
                ProjectItem::Composition(c) => Some(c),
                _ => None,
            })
            .collect();
        let found = match key {
            Some(key) => {
                let id = Uuid::try_parse(key).ok();
                comps
                    .iter()
                    .find(|c| Some(c.id) == id)
                    .or_else(|| comps.iter().find(|c| c.name == key))
                    .ok_or_else(|| format!("There is no comp called \"{key}\"."))?
            }
            None => match comps.as_slice() {
                _ if self.main.is_some() => comps
                    .iter()
                    .find(|c| Some(c.id) == self.main)
                    .ok_or("The graphics template has no comp of its own.")?,
                [only] => only,
                [] => return Err("The project has no comp.".into()),
                more => {
                    let names: Vec<&str> = more.iter().map(|c| c.name.as_str()).collect();
                    return Err(format!(
                        "The project has {} comps. Name one with --comp: {}.",
                        more.len(),
                        names.join(", ")
                    ));
                }
            },
        };
        Ok((found.id, found.name.clone()))
    }

    /// Whether any export is still running or waiting its turn.
    pub fn exporting(&self) -> bool {
        self.next_in_turn();
        self.exports
            .borrow_mut()
            .iter_mut()
            .any(|e| drain(e, Duration::ZERO).state == ExportState::Running)
    }

    /// The encoder export `job` settled on, once it has.
    pub fn export_encoder(&self, job: u64) -> Option<&'static str> {
        let mut exports = self.exports.borrow_mut();
        job_of(&mut exports, job).ok()?.encoder
    }

    /// Hold export `job` between two frames, or let it go on.
    pub fn export_hold(&self, job: u64, on: bool) {
        let mut exports = self.exports.borrow_mut();
        if let Ok(Export {
            handle: Some(handle),
            ..
        }) = job_of(&mut exports, job)
        {
            handle.hold(on);
        }
    }

    /// Stop every running export, and every one waiting its turn.
    pub fn cancel_exports(&self) {
        for export in self.exports.borrow_mut().iter_mut() {
            cancel(export);
        }
    }

    /// Start the first export waiting its turn, once none is running.
    fn next_in_turn(&self) {
        let mut exports = self.exports.borrow_mut();
        let running = exports
            .iter_mut()
            .any(|e| e.handle.is_some() && drain(e, Duration::ZERO).state == ExportState::Running);
        if running {
            return;
        }
        let Some(export) = exports.iter_mut().find(|e| e.waiting.is_some()) else {
            return;
        };
        if let Some((doc, comp)) = export.waiting.take() {
            let out = export.status.path.clone();
            export.handle = Some(self.launch(doc, comp, out, ExportSpec::default()));
        }
    }

    /// Start exporting `comp` to `out` with `spec`. Answers a number to ask
    /// after it with, as [`Host::export_start`] does.
    pub fn start_export(
        &self,
        comp: Uuid,
        out: PathBuf,
        spec: ExportSpec,
    ) -> Result<u64, ScriptError> {
        if self.exporting() {
            return Err(ScriptError::Failed(
                "Another export is still running. One runs at a time.".into(),
            ));
        }
        let doc = self.store.snapshot();
        if doc.comp(comp).is_none() {
            return Err(ScriptError::NotFound("There is no such comp.".into()));
        }
        let handle = self.launch(doc, comp, out.clone(), spec);
        let mut exports = self.exports.borrow_mut();
        exports.push(Export {
            handle: Some(handle),
            waiting: None,
            encoder: None,
            status: ExportStatus {
                state: ExportState::Running,
                done: 0,
                total: 0,
                path: out,
                message: None,
            },
        });
        Ok(exports.len() as u64)
    }

    /// Start the exporter on `comp` of `doc`.
    fn launch(
        &self,
        doc: Arc<Document>,
        comp: Uuid,
        out: PathBuf,
        spec: ExportSpec,
    ) -> ExportHandle {
        self.load_plugins(&doc);
        // The sound comes from what is delivered, as in Lumit: a guide
        // layer's is no more exported than its picture.
        let delivery = lumit_render::export::apply_render_overrides(&doc, &spec.render_options());
        let doc = delivery.unwrap_or(doc);
        let audio = doc
            .comp(comp)
            .map(|c| lumit_render::headless::AudioJobsBuilder::new().audio_jobs(&doc, c))
            .unwrap_or_default();
        lumit_render::export::start(doc, comp, audio, out, spec)
    }

    /// `path` as an mp4, or the first of `name 2.mp4`, `name 3.mp4` and on
    /// that is neither on disk nor an export this program has started.
    fn free_mp4(&self, path: &Path) -> PathBuf {
        let mut file = path.as_os_str().to_owned();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        {
            file.push(".mp4");
        }
        let first = PathBuf::from(file);
        let exports = self.exports.borrow();
        let taken = |p: &Path| p.exists() || exports.iter().any(|e| e.status.path == p);
        let stem = first.file_stem().unwrap_or_default().to_string_lossy();
        let mut next = first.clone();
        let mut n = 2;
        while taken(&next) {
            next = first.with_file_name(format!("{stem} {n}.mp4"));
            n += 1;
        }
        next
    }

    /// Scan for plugin effects once, when the project uses an effect Lumit
    /// does not have built in, as the app does at start-up. A scan starts a
    /// process for each plugin, which is seconds on a machine with many.
    fn load_plugins(&self, doc: &Document) {
        let unknown = doc.items.iter().any(|item| match item {
            ProjectItem::Composition(c) => c.effects().any(|fx| {
                lumit_core::fx::catalogue::BUILTIN_DEFS
                    .get(&fx.effect.match_name)
                    .is_none()
            }),
            _ => false,
        });
        if !unknown || self.plugins_scanned.replace(true) {
            return;
        }
        note!("Looking for plugins");
        crate::api::effect::rescan_plugins(None);
    }

    /// The project as the bridge's own placement code reads it, with no
    /// journal and no Viewer behind it.
    fn bridge_state(&self) -> LumitBridgeState {
        LumitBridgeState {
            store: Arc::clone(&self.store),
            path: self.path.borrow().clone(),
            saved_revision: self.store.revision(),
            media: Default::default(),
            journal: Arc::default(),
            auto_pack_here: false,
            sender: None,
            colour: Mutex::default(),
        }
    }

    /// Where an export of the comp called `name` goes when nobody says: the
    /// comp's name beside the project, never over a file already there.
    fn export_path(&self, name: &str) -> PathBuf {
        let dir = self
            .path
            .borrow()
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let safe: String = name
            .chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                c if c.is_control() => '_',
                c => c,
            })
            .collect();
        let safe = safe.trim().trim_end_matches('.');
        let safe = if safe.is_empty() { "Export" } else { safe };
        let mut path = dir.join(format!("{safe}.mp4"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{safe} {n}.mp4"));
            n += 1;
        }
        path
    }
}

/// Point the packed items of a just-opened `doc` at files to read, copying
/// out of `archive` whatever is no longer on disk. Answers the hold on the
/// folder they were copied to.
fn read_out_packed(doc: &mut Document, archive: &Path, dir: &Path) -> Option<File> {
    let dest = lumit_project::packed_media_dir(doc.id).unwrap_or_else(|| {
        std::env::temp_dir()
            .join("lumit-packed")
            .join(doc.id.to_string())
    });
    let held = dest
        .parent()
        .and_then(|root| lumit_project::pack::hold_read_out(root, doc.id, !doc.packed.is_empty()));
    if !doc.packed.is_empty() {
        let archives: Vec<PathBuf> = std::iter::once(archive.to_path_buf())
            .chain(lumit_project::pack::autosave_owner(archive))
            .collect();
        // An item that could not be read out is left for the resolver.
        let _ = lumit_project::pack::restore_packed(doc, &archives, dir, &dest, &mut |_, _| true);
    }
    held
}

/// Take in whatever `export` has said since it was last asked, waiting up
/// to `wait` for something when it is running.
fn drain(export: &mut Export, wait: Duration) -> &ExportStatus {
    let Some(handle) = &export.handle else {
        return &export.status;
    };
    let mut wait = (export.status.state == ExportState::Running).then_some(wait);
    loop {
        let event = match wait.take() {
            Some(wait) if !wait.is_zero() => handle.events.recv_timeout(wait).ok(),
            _ => handle.events.try_recv().ok(),
        };
        let Some(event) = event else {
            break;
        };
        let status = &mut export.status;
        match event {
            ExportEvent::Encoder(label) => export.encoder = Some(label),
            ExportEvent::Progress { frame, total } => {
                status.done = frame as u64;
                status.total = total as u64;
            }
            ExportEvent::Done(path) => {
                status.state = ExportState::Done;
                status.done = status.total;
                status.path = path;
            }
            ExportEvent::Failed(why) if why == "cancelled" => {
                status.state = ExportState::Cancelled;
            }
            ExportEvent::Failed(why) => {
                status.state = ExportState::Failed;
                status.message = Some(why);
            }
        }
    }
    &export.status
}

/// Stop `export`, or take it out of its turn when it is waiting.
fn cancel(export: &mut Export) {
    if let Some(handle) = &export.handle {
        handle.cancel();
    }
    if export.waiting.take().is_some() {
        export.status.state = ExportState::Cancelled;
    }
}

impl Host for Headless {
    fn store(&self) -> &DocumentStore {
        &self.store
    }

    fn export_vector(
        &self,
        comp: Uuid,
        out: &Path,
        settings: &lumit_lottie::settings::Settings,
    ) -> Result<Vec<String>, ScriptError> {
        let path = self.path.borrow().clone();
        crate::vector_host::write_to(&self.store.snapshot(), path.as_deref(), comp, settings, out)
            .map_err(ScriptError::Failed)
    }

    fn path(&self) -> Option<PathBuf> {
        self.path.borrow().clone()
    }

    fn save(&self, path: Option<&Path>) -> Result<PathBuf, ScriptError> {
        let target = self.save_target(path)?;
        self.claim(&target)?;
        let dir = target.parent().unwrap_or(Path::new("."));
        let doc = lumit_project::rebase_for_save(&self.store.snapshot(), dir);
        let saved = if doc.packed.is_empty() {
            lumit_project::save(&doc, &target)
        } else {
            // What the project packs comes over from the file it was opened
            // from. Nothing new is packed: that is a choice made in Lumit.
            let previous = self.archive.as_deref();
            lumit_project::pack::save_packed(&doc, &target, previous, &[], &mut |_, _| true)
                .map(|_| ())
        };
        saved.map_err(|e| {
            ScriptError::Failed(format!("{} could not be saved: {e}.", target.display()))
        })?;
        *self.path.borrow_mut() = Some(target.clone());
        Ok(target)
    }

    fn import_footage(&self, paths: &[PathBuf]) -> Result<Vec<Uuid>, ScriptError> {
        let doc = self.store.snapshot();
        let mut ids = Vec::new();
        let mut ops = Vec::new();
        let mut added: Vec<FootageItem> = Vec::new();
        for path in paths {
            let picked = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            if !picked.is_file() {
                return Err(ScriptError::NotFound(format!(
                    "{} could not be found.",
                    path.display()
                )));
            }
            // As Lumit's import makes it: a still with numbered neighbours
            // comes in as the whole run.
            let item =
                ProjectReference::footage_from_disk(&picked.to_string_lossy()).map_err(|_| {
                    ScriptError::Failed(format!("{} could not be imported.", path.display()))
                })?;
            // A second file of a run already in comes back as that item.
            let same = |f: &FootageItem| {
                f.sequence.is_some() && f.media.absolute_path == item.media.absolute_path
            };
            let existing = doc
                .items
                .iter()
                .filter_map(|i| match i {
                    ProjectItem::Footage(f) => Some(f),
                    _ => None,
                })
                .chain(&added)
                .find(|f| item.sequence.is_some() && same(f));
            if let Some(existing) = existing {
                ids.push(existing.id);
                continue;
            }
            #[cfg(feature = "media")]
            if !crate::api::footage::FootageReference::resolve_source(&self.bridge_state(), &item)
                .is_some_and(crate::probe::reads)
            {
                return Err(crate::script::unreadable(path));
            }
            ids.push(item.id);
            ops.push(Op::AddItem {
                index: doc.items.len() + added.len(),
                item: Box::new(ProjectItem::Footage(item.clone())),
            });
            added.push(item);
        }
        if !ops.is_empty() {
            self.store.commit(Op::Batch { ops })?;
        }
        Ok(ids)
    }

    fn footage_layers(&self, comp: &Composition, item: Uuid) -> Result<Vec<Layer>, ScriptError> {
        // Placed as dropping it on the Timeline places it. A file that will
        // not read still places, at the comp's size and length, so it can be
        // relinked.
        let doc = self.store.snapshot();
        let (layer, beside) = CompositionReference::footage_layers(
            &self.bridge_state(),
            &doc,
            comp,
            item,
            false,
            false,
            BridgeStreamsArrival::First,
        )
        .map_err(|e| match e {
            BridgeError::InvalidItem => {
                ScriptError::NotFound("There is no such footage item.".into())
            }
            _ => ScriptError::Failed("That footage cannot be placed.".into()),
        })?;
        Ok(std::iter::once(layer).chain(beside).collect())
    }

    fn export_start(&self, comp: Uuid, out: Option<&Path>) -> Result<u64, ScriptError> {
        let name = self
            .store
            .snapshot()
            .comp(comp)
            .map(|c| c.name.clone())
            .ok_or_else(|| ScriptError::NotFound("There is no such comp.".into()))?;
        let out = match out {
            Some(out) => {
                let mut out = std::path::absolute(out).unwrap_or_else(|_| out.to_path_buf());
                if out.extension().is_none() {
                    out.set_extension("mp4");
                }
                out
            }
            None => self.export_path(&name),
        };
        self.start_export(comp, out, ExportSpec::default())
    }

    fn export_status(&self, job: u64) -> Result<ExportStatus, ScriptError> {
        self.next_in_turn();
        let mut exports = self.exports.borrow_mut();
        let export = job_of(&mut exports, job)?;
        if export.waiting.is_some() {
            // Not spinning a script that asks in a loop.
            std::thread::sleep(STATUS_WAIT);
        }
        Ok(drain(export, STATUS_WAIT).clone())
    }

    fn export_cancel(&self, job: u64) -> Result<(), ScriptError> {
        let mut exports = self.exports.borrow_mut();
        cancel(job_of(&mut exports, job)?);
        Ok(())
    }

    // Made in a private copy of the project, which is never saved, and
    // exported in turn after the exports started before it.
    fn export_version(
        &self,
        comp: Uuid,
        ops: Vec<Op>,
        out: &Path,
    ) -> Result<(u64, PathBuf), ScriptError> {
        let mut doc = Document::clone(&self.store.snapshot());
        for op in &ops {
            lumit_core::ops::apply(&mut doc, op)
                .map_err(|_| ScriptError::Failed("That row's version could not be made.".into()))?;
        }
        if doc.comp(comp).is_none() {
            return Err(ScriptError::NotFound("There is no such comp.".into()));
        }
        let out = std::path::absolute(out).unwrap_or_else(|_| out.to_path_buf());
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                ScriptError::Failed(format!("{} could not be made: {e}.", dir.display()))
            })?;
        }
        let path = self.free_mp4(&out);
        let mut exports = self.exports.borrow_mut();
        exports.push(Export {
            handle: None,
            waiting: Some((Arc::new(doc), comp)),
            encoder: None,
            status: ExportStatus {
                state: ExportState::Running,
                done: 0,
                total: 0,
                path: path.clone(),
                message: None,
            },
        });
        let job = exports.len() as u64;
        drop(exports);
        self.next_in_turn();
        Ok((job, path))
    }

    fn template_library(&self) -> Vec<LibraryTemplate> {
        crate::script::template_library()
    }

    fn template_bring(&self, path: &Path) -> Result<Uuid, ScriptError> {
        let (from, info) = library::opened(path).map_err(|_| crate::script::not_brought(path))?;
        let doc = self.store.snapshot();
        if let Some(have) = library::in_project(&doc, info.id, Some(info.version)) {
            return Ok(have);
        }
        let (comp, ops, _) = library::brought(&doc, &from, &info, crate::script::TEMPLATES_FOLDER)
            .map_err(|_| crate::script::not_brought(path))?;
        self.store.commit(Op::Batch { ops })?;
        Ok(comp)
    }

    fn template_save(
        &self,
        comp: Uuid,
        name: &str,
        group: &str,
        pack_fonts: bool,
    ) -> Result<PathBuf, ScriptError> {
        let doc = self.store.snapshot();
        let beside = self
            .path
            .borrow()
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let (file, template) =
            library::save_to_library(&doc, comp, name, group, pack_fonts, &beside)
                .map_err(crate::script::not_saved)?;
        // The project's copy wears the marks of the save, as in Lumit.
        self.store.commit(Op::SetCompTemplate {
            comp,
            template: Some(Box::new(template)),
        })?;
        Ok(file)
    }

    fn render_frame(
        &self,
        comp: Uuid,
        frame: u64,
        scale: f32,
        out: &Path,
    ) -> Result<(u32, u32), ScriptError> {
        let doc = self.store.snapshot();
        self.load_plugins(&doc);
        crate::render::save_frame(&doc, comp, frame, scale, out).map_err(ScriptError::Failed)
    }

    fn parent_ops(&self, comp: Uuid, layer: Uuid, parent: Option<Uuid>, at: CompTime) -> Vec<Op> {
        let doc = self.store.snapshot();
        lumit_render::build::parent_ops(&doc, comp, layer, parent, at.0.to_f64())
    }

    #[cfg(feature = "media")]
    fn footage_info(&self, item: Uuid) -> Option<lumit_script::FootageInfo> {
        crate::script::footage_info(crate::script::footage_source(&self.bridge_state(), item)?)
    }

    fn stop_flag(&self) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("script time".into())
            .spawn(move || {
                std::thread::sleep(SCRIPT_TIME);
                flag.store(true, Ordering::Relaxed);
            });
        stop
    }

    /// There is no Viewer here, so a project with one comp has that one open.
    fn active_comp(&self) -> Option<Uuid> {
        let doc = self.store.snapshot();
        let mut comps = doc.items.iter().filter_map(|i| match i {
            ProjectItem::Composition(c) => Some(c.id),
            _ => None,
        });
        match (comps.next(), comps.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    }
}

/// Export `job`, numbered from 1 in the order they started.
fn job_of(exports: &mut [Export], job: u64) -> Result<&mut Export, ScriptError> {
    usize::try_from(job)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| exports.get_mut(n))
        .ok_or_else(|| ScriptError::NotFound(format!("There is no export {job}.")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::MediaRef;

    /// Footage packed into a project, whose original has gone, is still in
    /// the file after the command line has opened it, changed it and saved
    /// it over itself.
    #[test]
    fn a_save_keeps_the_footage_the_project_packs() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("footage").join("clip.mov");
        std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
        std::fs::write(&clip, vec![7u8; 100_000]).unwrap();
        let mut doc = Document::new();
        let id = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            id,
            name: "clip.mov".into(),
            media: MediaRef {
                relative_path: "footage/clip.mov".into(),
                absolute_path: clip.to_string_lossy().into_owned(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            sequence: None,
            extra: serde_json::Map::new(),
            colour_space: None,
            source_layer: None,
            interpretation: Default::default(),
        }));
        let lum = dir.path().join("scene.lum");
        let source = lumit_project::pack::PackSource {
            item: id,
            files: vec![clip.clone()],
        };
        lumit_project::pack::save_packed(&doc, &lum, None, &[source], &mut |_, _| true).unwrap();
        std::fs::remove_dir_all(dir.path().join("footage")).unwrap();

        let (host, missing) = Headless::open(&lum).unwrap();
        assert!(missing.is_empty(), "{missing:?}");
        lumit_script::dispatch(&host, "comp.new", serde_json::json!({ "name": "Added" })).unwrap();
        host.save(None).unwrap();

        let (mut saved, _) = lumit_project::open(&lum).unwrap();
        assert!(saved.items.iter().any(|item| match item {
            ProjectItem::Composition(c) => c.name == "Added",
            _ => false,
        }));
        assert!(saved.packed.contains_key(&id));
        // Its bytes are in the file, not only its entry.
        let out = dir.path().join("read-out");
        let restored = lumit_project::pack::restore_packed(
            &mut saved,
            std::slice::from_ref(&lum),
            dir.path(),
            &out,
            &mut |_, _| true,
        )
        .unwrap();
        assert_eq!(restored, (1, Vec::new()));
    }

    /// `comp.frame` writes a PNG at the size asked for, and never over a
    /// file already there.
    #[test]
    fn a_frame_is_a_png_of_the_size_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let host = Headless::new_project();
        let args = serde_json::json!({ "name": "Shot", "width": 64, "height": 32 });
        let made = lumit_script::dispatch(&host, "comp.new", args).unwrap();
        let comp = Uuid::try_parse(made["id"].as_str().unwrap()).unwrap();
        if crate::render::thumbnail(&host.store.snapshot(), comp, 0, 1.0).is_none() {
            eprintln!("no graphics adapter; skipping");
            return;
        }
        let out = dir.path().join("shot.png");
        let args = serde_json::json!({ "comp": "Shot", "scale": 0.5, "out": out });
        let answer = lumit_script::dispatch(&host, "comp.frame", args.clone()).unwrap();
        assert_eq!(
            (answer["width"].as_u64(), answer["height"].as_u64()),
            (Some(32), Some(16))
        );
        let bytes = std::io::Cursor::new(std::fs::read(&out).unwrap());
        let png = png::Decoder::new(bytes).read_info().unwrap();
        assert_eq!((png.info().width, png.info().height), (32, 16));
        assert!(lumit_script::dispatch(&host, "comp.frame", args).is_err());
    }
}
