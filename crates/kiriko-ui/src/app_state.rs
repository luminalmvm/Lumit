//! Application state behind the shell: the document store, project path,
//! journal, dirty tracking, autosave. Slice 3 of docs/impl/phase-0-kickoff.md.

use kiriko_core::model::{Composition, Document, FootageItem, LinearColour, MediaRef, ProjectItem};
use kiriko_core::ops::Op;
use kiriko_core::time::{Duration, FrameRate, Rational};
use kiriko_core::DocumentStore;
use kiriko_project::JournalFile;
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

pub const AUTOSAVE_INTERVAL_SECS: u64 = 300;
pub const AUTOSAVE_KEEP: usize = 5;

/// Latest-wins background frame decoding for the Viewer (slice 5).
/// In plain terms: the UI sends "show frame N of item X" requests down a
/// channel; a worker thread owns the decoders and answers with pixels; stale
/// requests are simply skipped (the epoch/latest-wins idea from
/// docs/impl/playback-scheduler.md, in miniature).
#[cfg(feature = "media")]
pub mod preview {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
    use std::sync::Arc;
    use uuid::Uuid;

    pub struct FramePixels {
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
        pub frame: usize,
        pub item: Uuid,
    }

    struct Request {
        generation: u64,
        item: Uuid,
        path: PathBuf,
        frame: usize,
        target_width: Option<u32>,
    }

    pub struct PreviewEngine {
        tx: Sender<Request>,
        pub results: Receiver<Result<FramePixels, String>>,
        generation: Arc<AtomicU64>,
    }

    impl Default for PreviewEngine {
        fn default() -> Self {
            let (tx, rx) = channel::<Request>();
            let (result_tx, results) = channel();
            let generation = Arc::new(AtomicU64::new(0));
            let live = generation.clone();
            std::thread::spawn(move || {
                let mut decoders: HashMap<Uuid, kiriko_media::VideoDecoder> = HashMap::new();
                loop {
                    // Block for one request, then drain to the newest (latest wins).
                    let mut req = match rx.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    };
                    loop {
                        match rx.try_recv() {
                            Ok(newer) => req = newer,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    if req.generation != live.load(Ordering::Relaxed) {
                        continue; // superseded while queued
                    }
                    let result = decode(&mut decoders, &req);
                    let _ = result_tx.send(result);
                }
            });
            Self {
                tx,
                results,
                generation,
            }
        }
    }

    fn decode(
        decoders: &mut HashMap<Uuid, kiriko_media::VideoDecoder>,
        req: &Request,
    ) -> Result<FramePixels, String> {
        let dec = match decoders.entry(req.item) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let index =
                    kiriko_media::index::build_frame_index(&req.path).map_err(|e| e.to_string())?;
                let dec = kiriko_media::VideoDecoder::open(&req.path, index)
                    .map_err(|e| e.to_string())?;
                e.insert(dec)
            }
        };
        let frame = req.frame.min(dec.frame_count().saturating_sub(1));
        let out = dec
            .frame_rgba(frame, req.target_width)
            .map_err(|e| e.to_string())?;
        Ok(FramePixels {
            width: out.width,
            height: out.height,
            rgba: out.rgba,
            frame,
            item: req.item,
        })
    }

    impl PreviewEngine {
        /// Ask for a frame; any not-yet-decoded older request is abandoned.
        pub fn request(&self, item: Uuid, path: PathBuf, frame: usize, target_width: Option<u32>) {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = self.tx.send(Request {
                generation,
                item,
                path,
                frame,
                target_width,
            });
        }
    }
}

/// Probe/index results for footage items, filled by background threads.
#[cfg(feature = "media")]
pub mod media {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use uuid::Uuid;

    pub enum MediaStatus {
        Probing,
        Ready {
            probe: kiriko_media::MediaProbe,
            frames: usize,
            vfr: bool,
        },
        Failed(String),
    }

    pub struct MediaRegistry {
        pub map: HashMap<Uuid, MediaStatus>,
        tx: Sender<(Uuid, MediaStatus)>,
        rx: Receiver<(Uuid, MediaStatus)>,
    }

    impl Default for MediaRegistry {
        fn default() -> Self {
            let (tx, rx) = channel();
            Self {
                map: HashMap::new(),
                tx,
                rx,
            }
        }
    }

    impl MediaRegistry {
        /// Drain background results into the map. Called once per UI frame.
        pub fn poll(&mut self) {
            while let Ok((id, status)) = self.rx.try_recv() {
                self.map.insert(id, status);
            }
        }

        pub fn any_probing(&self) -> bool {
            self.map.values().any(|s| matches!(s, MediaStatus::Probing))
        }

        /// Probe + build/load the frame index on a background thread
        /// (docs/impl/media-io.md §2 — never on the UI thread, K-017).
        pub fn spawn_probe(&mut self, id: Uuid, path: PathBuf) {
            self.map.insert(id, MediaStatus::Probing);
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let status = probe_and_index(&path);
                let _ = tx.send((id, status));
            });
        }
    }

    fn probe_and_index(path: &std::path::Path) -> MediaStatus {
        let probe = match kiriko_media::probe::probe(path) {
            Ok(p) => p,
            Err(e) => return MediaStatus::Failed(e.to_string()),
        };
        // Audio-only items need no frame index.
        if probe.video.is_none() {
            return MediaStatus::Ready {
                probe,
                frames: 0,
                vfr: false,
            };
        }
        let cache_dir = kiriko_project::media_index_dir();
        let cached = match (&cache_dir, kiriko_media::Fingerprint::of(path)) {
            (Some(dir), Ok(fp)) => kiriko_media::FrameIndex::load_cached(dir, &fp),
            _ => None,
        };
        let index = match cached {
            Some(index) => index,
            None => match kiriko_media::index::build_frame_index(path) {
                Ok(index) => {
                    if let Some(dir) = &cache_dir {
                        let _ = index.save_to(dir);
                    }
                    index
                }
                Err(e) => return MediaStatus::Failed(e.to_string()),
            },
        };
        MediaStatus::Ready {
            probe,
            frames: index.frame_count(),
            vfr: index.vfr,
        }
    }
}

/// Infallible constructor for small literal rationals.
fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap_or(Rational::ZERO)
}

/// A recovery offer: the saved document plus the journal ops beyond it.
pub struct PendingRecovery {
    pub doc: Document,
    pub path: PathBuf,
    pub ops: Vec<Op>,
}

pub struct AppState {
    pub store: DocumentStore,
    pub path: Option<PathBuf>,
    journal: Option<JournalFile>,
    pub dirty: bool,
    pub selected_comp: Option<Uuid>,
    pub pending_recovery: Option<PendingRecovery>,
    pub error: Option<String>,
    #[cfg(feature = "media")]
    pub media: media::MediaRegistry,
    #[cfg(feature = "media")]
    pub preview_engine: preview::PreviewEngine,
    /// Footage item currently shown in the Viewer, and the scrub position.
    pub preview_item: Option<Uuid>,
    pub preview_frame: usize,
    /// Preview resolution divisor: 1 = Full, 2 = Half, 3 = Third, 4 = Quarter.
    pub preview_divisor: u32,
    last_autosave: Instant,
    comp_counter: usize,
}

impl Default for AppState {
    fn default() -> Self {
        let doc = Document::new();
        let journal = JournalFile::for_document(doc.id);
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
            preview_item: None,
            preview_frame: 0,
            preview_divisor: 1,
            last_autosave: Instant::now(),
            comp_counter: 0,
        }
    }
}

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
            .add_filter("Kiriko project", &["kir"])
            .pick_file();
        if let Some(path) = picked {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        let Some((doc, _manifest)) = self.report(kiriko_project::open(path)) else {
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
                if kiriko_core::ops::apply(&mut doc, op).is_err() {
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
                .add_filter("Kiriko project", &["kir"])
                .set_file_name("untitled.kir")
                .save_file(),
        };
        let Some(path) = path else { return };
        let doc = self.store.snapshot();
        if self.report(kiriko_project::save(&doc, &path)).is_some() {
            if let Some(journal) = &self.journal {
                let _ = journal.clear();
            }
            self.path = Some(path);
            self.dirty = false;
        }
    }

    pub fn autosave_tick(&mut self) {
        if self.dirty
            && self.path.is_some()
            && self.last_autosave.elapsed().as_secs() >= AUTOSAVE_INTERVAL_SECS
        {
            self.last_autosave = Instant::now();
            if let Some(path) = self.path.clone() {
                let doc = self.store.snapshot();
                let _ = self.report(kiriko_project::autosave(&doc, &path, AUTOSAVE_KEEP));
            }
        }
    }

    pub fn import_footage_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Media",
                &[
                    "mp4", "mov", "mkv", "avi", "webm", "png", "jpg", "jpeg", "wav", "mp3", "flac",
                ],
            )
            .pick_files();
        let Some(files) = picked else { return };
        let base = self.store.snapshot().items.len();
        for (i, file) in files.into_iter().enumerate() {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "footage".into());
            let item = FootageItem {
                id: Uuid::now_v7(),
                name: name.clone(),
                extra: serde_json::Map::new(),
                media: MediaRef {
                    relative_path: name,
                    absolute_path: file.to_string_lossy().into_owned(),
                    extra: serde_json::Map::new(),
                },
            };
            #[cfg(feature = "media")]
            let probe_target = (item.id, file.clone());
            self.commit(Op::AddItem {
                index: base + i,
                item: Box::new(ProjectItem::Footage(item)),
            });
            #[cfg(feature = "media")]
            self.media.spawn_probe(probe_target.0, probe_target.1);
        }
    }

    pub fn new_composition(&mut self) {
        self.comp_counter += 1;
        let comp = Composition {
            id: Uuid::now_v7(),
            name: format!("Comp {}", self.comp_counter),
            width: 1920,
            height: 1080,
            frame_rate: match FrameRate::new(60, 1) {
                Ok(fr) => fr,
                Err(_) => return,
            },
            duration: Duration(rat(30, 1)),
            background: LinearColour::BLACK,
            layers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let id = comp.id;
        let index = self.store.snapshot().items.len();
        self.commit(Op::AddItem {
            index,
            item: Box::new(ProjectItem::Composition(comp)),
        });
        self.selected_comp = Some(id);
    }

    /// Re-request the current preview frame (selection/scrub/resolution change).
    #[cfg(feature = "media")]
    pub fn refresh_preview(&mut self) {
        let Some(id) = self.preview_item else { return };
        let doc = self.store.snapshot();
        let Some(ProjectItem::Footage(f)) = doc.item(id) else {
            return;
        };
        let (width, frames) = match self.media.map.get(&id) {
            Some(media::MediaStatus::Ready { probe, frames, .. }) => {
                (probe.video.as_ref().map(|v| v.width).unwrap_or(0), *frames)
            }
            _ => return, // not probed yet; selection will refresh on Ready
        };
        if frames == 0 || width == 0 {
            return;
        }
        self.preview_frame = self.preview_frame.min(frames - 1);
        let target = (self.preview_divisor > 1).then(|| width / self.preview_divisor);
        self.preview_engine.request(
            id,
            PathBuf::from(&f.media.absolute_path),
            self.preview_frame,
            target,
        );
    }

    pub fn project_title(&self) -> String {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        if self.dirty {
            format!("{name} • Kiriko")
        } else {
            format!("{name} — Kiriko")
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The slice 3 drill: save, edit past the save, crash (drop without
    /// saving), reopen — the journal restores every post-save change.
    #[test]
    fn kill_and_recover_drill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drill.kir");

        let doc_id;
        let final_json;
        {
            let mut app = AppState::default();
            doc_id = app.store.snapshot().id;
            app.new_composition();
            app.path = Some(path.clone());
            app.save();
            assert!(!app.dirty);

            // Edits after the save — journalled, never saved.
            app.new_composition();
            app.new_composition();
            assert!(app.dirty);
            final_json = serde_json::to_string(&*app.store.snapshot()).unwrap();
            // "kill -9": app dropped here with dirty state.
        }

        let mut app2 = AppState::default();
        app2.open_path(&path);
        let pending = app2.pending_recovery.as_ref().expect("recovery offered");
        assert_eq!(pending.ops.len(), 2);
        app2.resolve_recovery(true);
        assert_eq!(
            serde_json::to_string(&*app2.store.snapshot()).unwrap(),
            final_json,
            "recovered document equals the pre-crash document"
        );
        assert!(app2.dirty, "recovered state needs a save");

        // Saving clears the journal: a fresh open offers no recovery.
        app2.save();
        let mut app3 = AppState::default();
        app3.open_path(&path);
        assert!(app3.pending_recovery.is_none());

        let _ = JournalFile::for_document(doc_id).map(|j| j.clear());
    }

    #[test]
    fn discarding_recovery_opens_last_save_and_clears_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drill2.kir");
        let saved_json;
        {
            let mut app = AppState::default();
            app.new_composition();
            app.path = Some(path.clone());
            app.save();
            saved_json = serde_json::to_string(&*app.store.snapshot()).unwrap();
            app.new_composition(); // journalled, then "crash"
        }
        let mut app2 = AppState::default();
        app2.open_path(&path);
        assert!(app2.pending_recovery.is_some());
        app2.resolve_recovery(false);
        assert_eq!(
            serde_json::to_string(&*app2.store.snapshot()).unwrap(),
            saved_json
        );
        let mut app3 = AppState::default();
        app3.open_path(&path);
        assert!(
            app3.pending_recovery.is_none(),
            "journal cleared on discard"
        );
    }
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod preview_tests {
    use super::preview::PreviewEngine;
    use kiriko_media::index::tests_support::fixture;
    use std::time::Duration;

    /// End-to-end: request a frame the way the Viewer does; receive pixels.
    #[test]
    fn preview_engine_decodes_requested_frame_at_requested_size() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let engine = PreviewEngine::default();
        let id = uuid::Uuid::now_v7();
        engine.request(id, file, 45, Some(160));
        let px = engine
            .results
            .recv_timeout(Duration::from_secs(20))
            .expect("engine replied")
            .expect("decode succeeded");
        assert_eq!(px.item, id);
        assert_eq!(px.frame, 45);
        assert_eq!((px.width, px.height), (160, 120));
        assert_eq!(px.rgba.len(), 160 * 120 * 4);
    }

    /// Latest-wins: flood requests; the engine may skip stale ones and the
    /// final delivered frame is the newest request.
    #[test]
    fn preview_engine_latest_request_wins() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let engine = PreviewEngine::default();
        let id = uuid::Uuid::now_v7();
        for n in 0..60 {
            engine.request(id, file.clone(), n, None);
        }
        let mut last = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match engine.results.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(px)) => {
                    last = Some(px.frame);
                    if px.frame == 59 {
                        break;
                    }
                }
                Ok(Err(e)) => panic!("decode failed: {e}"),
                Err(_) => {
                    if last == Some(59) {
                        break;
                    }
                }
            }
        }
        assert_eq!(last, Some(59), "newest request must be the one served last");
    }
}
