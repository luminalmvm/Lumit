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

    /// One layer's decode job inside a comp render request.
    pub struct CompJob {
        pub layer: Uuid,
        pub item: Uuid,
        pub path: PathBuf,
        pub source_frame: usize,
        pub target_width: Option<u32>,
    }

    pub struct CompLayerPixels {
        pub layer: Uuid,
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
    }

    pub struct CompFrame {
        pub comp: Uuid,
        pub frame: usize,
        /// Top-of-stack first (document order); the renderer draws bottom-up.
        pub layers: Vec<CompLayerPixels>,
    }

    pub enum PreviewResult {
        Footage(FramePixels),
        Comp(CompFrame),
    }

    pub struct PreviewEngine {
        tx: Sender<Message>,
        pub results: Receiver<Result<PreviewResult, String>>,
        generation: Arc<AtomicU64>,
    }

    enum Message {
        Footage(Request),
        Comp {
            generation: u64,
            comp: Uuid,
            frame: usize,
            jobs: Vec<CompJob>,
        },
    }

    impl Default for PreviewEngine {
        fn default() -> Self {
            let (tx, rx) = channel::<Message>();
            let (result_tx, results) = channel();
            let generation = Arc::new(AtomicU64::new(0));
            let live = generation.clone();
            std::thread::spawn(move || {
                let mut decoders: HashMap<Uuid, kiriko_media::VideoDecoder> = HashMap::new();
                // Decoded-frame RAM cache (K-016 tier seed): recently shown
                // frames re-display instantly instead of re-decoding.
                let mut frame_cache: kiriko_cache::ByteLru<
                    (Uuid, usize, Option<u32>),
                    CachedFrame,
                > = kiriko_cache::ByteLru::new(512 * 1024 * 1024);
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
                    let generation = match &req {
                        Message::Footage(r) => r.generation,
                        Message::Comp { generation, .. } => *generation,
                    };
                    if generation != live.load(Ordering::Relaxed) {
                        continue; // superseded while queued
                    }
                    let result = match req {
                        Message::Footage(r) => {
                            decode(&mut decoders, &mut frame_cache, &r).map(PreviewResult::Footage)
                        }
                        Message::Comp {
                            comp, frame, jobs, ..
                        } => decode_comp(&mut decoders, &mut frame_cache, comp, frame, &jobs)
                            .map(PreviewResult::Comp),
                    };
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

    struct CachedFrame {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    impl kiriko_cache::ByteSized for CachedFrame {
        fn byte_size(&self) -> usize {
            self.rgba.len() + 16
        }
    }

    fn decode(
        decoders: &mut HashMap<Uuid, kiriko_media::VideoDecoder>,
        cache: &mut kiriko_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
        req: &Request,
    ) -> Result<FramePixels, String> {
        let cache_key = (req.item, req.frame, req.target_width);
        if let Some(hit) = cache.get(&cache_key) {
            return Ok(FramePixels {
                width: hit.width,
                height: hit.height,
                rgba: hit.rgba.clone(),
                frame: req.frame,
                item: req.item,
            });
        }
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
        cache.insert(
            cache_key,
            CachedFrame {
                width: out.width,
                height: out.height,
                rgba: out.rgba.clone(),
            },
        );
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
            let _ = self.tx.send(Message::Footage(Request {
                generation,
                item,
                path,
                frame,
                target_width,
            }));
        }

        /// Ask for every layer frame of a comp at one comp frame (latest wins).
        pub fn request_comp(&self, comp: Uuid, frame: usize, jobs: Vec<CompJob>) {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = self.tx.send(Message::Comp {
                generation,
                comp,
                frame,
                jobs,
            });
        }
    }

    fn decode_comp(
        decoders: &mut HashMap<Uuid, kiriko_media::VideoDecoder>,
        cache: &mut kiriko_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
        comp: Uuid,
        frame: usize,
        jobs: &[CompJob],
    ) -> Result<CompFrame, String> {
        let mut layers = Vec::with_capacity(jobs.len());
        for job in jobs {
            let req = Request {
                generation: 0,
                item: job.item,
                path: job.path.clone(),
                frame: job.source_frame,
                target_width: job.target_width,
            };
            let px = decode(decoders, cache, &req)?;
            layers.push(CompLayerPixels {
                layer: job.layer,
                width: px.width,
                height: px.height,
                rgba: px.rgba,
            });
        }
        Ok(CompFrame {
            comp,
            frame,
            layers,
        })
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
    #[cfg(feature = "media")]
    audio_engine: Option<kiriko_audio::AudioEngine>,
    #[cfg(feature = "media")]
    audio_cache: std::collections::HashMap<Uuid, std::sync::Arc<kiriko_media::AudioBuffer>>,
    #[cfg(feature = "media")]
    audio_loaded: Option<Uuid>,
    #[cfg(feature = "media")]
    audio_rx: std::sync::mpsc::Receiver<(Uuid, Result<kiriko_media::AudioBuffer, String>)>,
    #[cfg(feature = "media")]
    audio_tx: std::sync::mpsc::Sender<(Uuid, Result<kiriko_media::AudioBuffer, String>)>,
    /// In-flight property drag (layer, property, provisional value): commits
    /// once on release so a drag is ONE undo step, not hundreds.
    pub prop_edit: Option<(Uuid, kiriko_core::model::TransformProp, f64)>,
    /// In-flight bar-edge trim: (layer, trimming_out_edge, provisional seconds).
    pub trim_edit: Option<(Uuid, bool, f64)>,
    /// Layer whose properties the graph editor shows (clicked in the Timeline).
    pub selected_layer: Option<Uuid>,
    /// Property shown in the graph editor.
    pub graph_prop: Option<kiriko_core::model::TransformProp>,
    /// In-flight keyframe drag: (key index, provisional layer-time, value).
    pub graph_edit: Option<(usize, f64, f64)>,
    /// Graph editor lens: false = value graph, true = speed graph
    /// (docs/01-GLOSSARY.md §3: two views of the same data, never separate).
    pub graph_speed_view: bool,
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
    /// View zoom (1.0 = fit) and pan, in screen pixels. View controls only —
    /// never part of any render (07-UI-SPEC: Viewer).
    pub view_zoom: f32,
    pub view_pan: egui::Vec2,
    /// Screen pixels per native image pixel at last paint (Auto res input).
    pub last_display_scale: f32,
    last_autosave: Instant,
    comp_counter: usize,
}

impl Default for AppState {
    fn default() -> Self {
        let doc = Document::new();
        let journal = JournalFile::for_document(doc.id);
        #[cfg(feature = "media")]
        let (audio_tx, audio_rx) = std::sync::mpsc::channel();
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
            audio_rx,
            #[cfg(feature = "media")]
            audio_tx,
            prop_edit: None,
            trim_edit: None,
            selected_layer: None,
            graph_prop: None,
            graph_edit: None,
            graph_speed_view: false,
            preview_comp: None,
            comp_playback: None,
            preview_item: None,
            preview_frame: 0,
            preview_divisor: 1,
            preview_auto_res: false,
            view_zoom: 1.0,
            view_pan: egui::Vec2::ZERO,
            last_display_scale: 1.0,
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
        self.import_paths(files);
    }

    /// Import media files (dialogue or drag-and-drop onto the window).
    pub fn import_paths(&mut self, files: Vec<PathBuf>) {
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

    /// Add a footage item as a new top layer of the target comp
    /// (docs/16-ROADMAP.md phase 1: comps become buildable by hand).
    pub fn add_footage_to_comp(&mut self, item_id: Uuid) {
        use kiriko_core::model::{Layer, LayerKind, Switches, TransformGroup};
        use kiriko_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(ProjectItem::Footage(f)) = doc.item(item_id) else {
            return;
        };

        // Span: media duration when known (frame-exact via the comp grid),
        // else the full comp.
        let comp_dur = comp.duration.0;
        #[cfg(feature = "media")]
        let out = match self.media.map.get(&item_id) {
            Some(media::MediaStatus::Ready { probe, .. }) => {
                let frames = (probe.duration_seconds * comp.frame_rate.fps()).round() as i64;
                comp.frame_rate
                    .time_of_frame(frames.max(1))
                    .map(|t| if t.0 < comp_dur { t.0 } else { comp_dur })
                    .unwrap_or(comp_dur)
            }
            _ => comp_dur,
        };
        #[cfg(not(feature = "media"))]
        let out = comp_dur;

        let layer = Layer {
            id: Uuid::now_v7(),
            name: f.name.clone(),
            kind: LayerKind::Footage { item: item_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(out),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Nest one comp inside another as a Precomp layer (cycle-guarded).
    pub fn add_precomp_to_comp(&mut self, nested_id: Uuid) {
        use kiriko_core::model::{Layer, LayerKind, Switches, TransformGroup};
        use kiriko_core::time::CompTime;
        let Some(target_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        if target_id == nested_id || self.would_cycle(nested_id, target_id) {
            self.error = Some("that nesting would loop compositions".into());
            return;
        }
        let doc = self.store.snapshot();
        let (Some(target), Some(nested)) = (doc.comp(target_id), doc.comp(nested_id)) else {
            return;
        };
        let out = if nested.duration.0 < target.duration.0 {
            nested.duration.0
        } else {
            target.duration.0
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: nested.name.clone(),
            kind: LayerKind::Precomp { comp: nested_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(out),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: target_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(target_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Would nesting `nested` inside `target` create a cycle? True if target
    /// is reachable from nested through Precomp layers.
    fn would_cycle(&self, nested: Uuid, target: Uuid) -> bool {
        use kiriko_core::model::LayerKind;
        let doc = self.store.snapshot();
        let mut stack = vec![nested];
        let mut seen = vec![];
        while let Some(id) = stack.pop() {
            if id == target {
                return true;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(c) = doc.comp(id) {
                for l in &c.layers {
                    if let LayerKind::Precomp { comp } = &l.kind {
                        stack.push(*comp);
                    }
                }
            }
        }
        false
    }

    /// Add a text layer with a starter document.
    pub fn add_text_layer(&mut self) {
        use kiriko_core::model::{
            Layer, LayerKind, LinearColour, Switches, TextDocument, TransformGroup,
        };
        use kiriko_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let transform = TransformGroup {
            position_x: kiriko_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
            position_y: kiriko_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Text".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: "Text".into(),
                    size: 72.0,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add a white comp-sized Solid layer (colour editing joins the layer
    /// properties panel).
    pub fn add_solid_layer(&mut self) {
        use kiriko_core::model::{Layer, LayerKind, LinearColour, Switches, TransformGroup};
        use kiriko_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Solid".into(),
            kind: LayerKind::Solid {
                colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
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
            work_area: None,
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

    /// Work-area frame span of a comp (start, end-exclusive); full when unset.
    pub fn work_area_frames(&self, comp: &Composition) -> (usize, usize) {
        let total = self.comp_frame_count(comp);
        let fps = comp.frame_rate.fps().max(1.0);
        match comp.work_area {
            Some((a, b)) => {
                let s = ((a.0.to_f64() * fps).round() as usize).min(total.saturating_sub(1));
                let e = ((b.0.to_f64() * fps).round() as usize).clamp(s + 1, total);
                (s, e)
            }
            None => (0, total),
        }
    }

    /// AE's B/N: set the work-area start or end at the playhead.
    pub fn set_work_area_edge(&mut self, end_edge: bool) {
        use kiriko_core::time::CompTime;
        let Some(comp_id) = self.preview_comp else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let fps = comp.frame_rate.fps().max(1.0);
        let t = self.preview_frame as f64 / fps;
        let dur = comp.duration.0.to_f64();
        let (mut a, mut b) = comp
            .work_area
            .map(|(a, b)| (a.0.to_f64(), b.0.to_f64()))
            .unwrap_or((0.0, dur));
        if end_edge {
            b = (t + 1.0 / fps).min(dur);
            if a >= b {
                a = 0.0;
            }
        } else {
            a = t.min(dur - 1.0 / fps);
            if b <= a {
                b = dur;
            }
        }
        let wa = if a <= 0.0 && (b - dur).abs() < 1e-9 {
            None // full span = no work area
        } else {
            Some((
                CompTime(
                    Rational::from_f64_on_grid(a, Rational::FLICK_DEN).unwrap_or(Rational::ZERO),
                ),
                CompTime(
                    Rational::from_f64_on_grid(b, Rational::FLICK_DEN).unwrap_or(comp.duration.0),
                ),
            ))
        };
        self.commit(Op::SetWorkArea {
            comp: comp_id,
            work_area: wa,
        });
    }

    /// Frame count of the comp preview (comp duration × comp rate).
    pub fn comp_frame_count(&self, comp: &Composition) -> usize {
        let dur = comp.duration.0.to_f64();
        (dur * comp.frame_rate.fps()).round().max(1.0) as usize
    }

    /// Build and send the batch request rendering `preview_comp` at the
    /// current frame (evaluator v0: footage layers, no retime yet).
    #[cfg(feature = "media")]
    pub fn refresh_comp_preview(&mut self) {
        use kiriko_core::model::LayerKind;
        let Some(comp_id) = self.preview_comp else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let frames = self.comp_frame_count(comp);
        self.preview_frame = self.preview_frame.min(frames.saturating_sub(1));
        let t = self.preview_frame as f64 / comp.frame_rate.fps();

        // Recursive job collection: visible layers, their matte sources, and
        // everything nested comps need at their mapped times (cycle-guarded).
        let mut jobs = Vec::new();
        let mut visited = vec![comp_id];
        self.collect_comp_jobs(&doc, comp, t, &mut jobs, &mut visited);
        let _ = LayerKind::Footage {
            item: Uuid::now_v7(),
        }; // keep the import used in both cfg modes
        self.preview_engine
            .request_comp(comp_id, self.preview_frame, jobs);
    }

    /// Decode target width for a source of `natural_w` px under the current
    /// resolution mode. Auto: displayed size, capped at 100% (never above
    /// native, however far the view is zoomed in).
    pub fn target_width_for(&self, natural_w: u32) -> Option<u32> {
        if self.preview_auto_res {
            let scale = self.last_display_scale.clamp(0.05, 1.0);
            let w = (natural_w as f32 * scale).round() as u32;
            (w < natural_w).then_some(w.max(16))
        } else {
            (self.preview_divisor > 1).then(|| natural_w / self.preview_divisor)
        }
    }

    /// Recursively collect decode jobs for a comp at time `t`
    /// (docs/06-RENDER-PIPELINE.md: Precomp evaluation).
    #[cfg(feature = "media")]
    fn collect_comp_jobs(
        &self,
        doc: &Document,
        comp: &Composition,
        t: f64,
        jobs: &mut Vec<preview::CompJob>,
        visited: &mut Vec<Uuid>,
    ) {
        use kiriko_core::model::LayerKind;
        let in_span = |l: &kiriko_core::model::Layer| {
            t >= l.in_point.0.to_f64() && t < l.out_point.0.to_f64()
        };
        let mut wanted: Vec<Uuid> = Vec::new();
        for l in &comp.layers {
            if l.switches.visible && in_span(l) {
                wanted.push(l.id);
                if let Some(m) = &l.matte {
                    if !wanted.contains(&m.layer) {
                        wanted.push(m.layer);
                    }
                }
            }
        }
        for layer in &comp.layers {
            if !wanted.contains(&layer.id) || !in_span(layer) {
                continue;
            }
            let lt = t - layer.start_offset.0.to_f64();
            match &layer.kind {
                LayerKind::Solid { .. } | LayerKind::Text { .. } => {}
                LayerKind::Precomp { comp: nested_id } => {
                    if visited.contains(nested_id) {
                        continue; // cycle guard
                    }
                    if let Some(nested) = doc.comp(*nested_id) {
                        visited.push(*nested_id);
                        self.collect_comp_jobs(doc, nested, lt, jobs, visited);
                        visited.pop();
                    }
                }
                LayerKind::Footage { item } => {
                    let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
                        continue;
                    };
                    let Some(media::MediaStatus::Ready {
                        probe,
                        frames: src_frames,
                        ..
                    }) = self.media.map.get(item)
                    else {
                        continue; // not probed yet; retried once Ready
                    };
                    let Some(video) = probe.video.as_ref() else {
                        continue;
                    };
                    let source_frame = ((lt * video.fps()).round().max(0.0) as usize)
                        .min(src_frames.saturating_sub(1));
                    jobs.push(preview::CompJob {
                        layer: layer.id,
                        item: *item,
                        path: PathBuf::from(&f.media.absolute_path),
                        source_frame,
                        target_width: self.target_width_for(video.width),
                    });
                }
            }
        }
    }

    /// Re-request the current preview frame (selection/scrub/resolution change).
    #[cfg(feature = "media")]
    pub fn refresh_preview(&mut self) {
        if self.preview_comp.is_some() {
            self.refresh_comp_preview();
            return;
        }
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
        let target = self.target_width_for(width);
        self.preview_engine.request(
            id,
            PathBuf::from(&f.media.absolute_path),
            self.preview_frame,
            target,
        );
    }

    /// Frames-per-second of the current preview item, once probed.
    #[cfg(feature = "media")]
    pub fn preview_fps(&self) -> Option<f64> {
        let id = self.preview_item?;
        match self.media.map.get(&id) {
            Some(media::MediaStatus::Ready { probe, .. }) => {
                probe.video.as_ref().map(|v| v.fps()).filter(|f| *f > 0.0)
            }
            _ => None,
        }
    }

    /// Kick off background audio decode for the preview item (idempotent).
    #[cfg(feature = "media")]
    pub fn request_preview_audio(&mut self) {
        let Some(id) = self.preview_item else { return };
        if self.audio_cache.contains_key(&id) {
            return;
        }
        let has_audio = matches!(
            self.media.map.get(&id),
            Some(media::MediaStatus::Ready { probe, .. }) if probe.audio.is_some()
        );
        if !has_audio {
            return;
        }
        let doc = self.store.snapshot();
        let Some(ProjectItem::Footage(f)) = doc.item(id) else {
            return;
        };
        let path = PathBuf::from(&f.media.absolute_path);
        let rate = self
            .ensure_audio_engine()
            .map(|e| e.device_rate())
            .unwrap_or(48_000);
        let tx = self.audio_tx.clone();
        std::thread::spawn(move || {
            let result = kiriko_media::audio::decode_all(&path, rate).map_err(|e| e.to_string());
            let _ = tx.send((id, result));
        });
    }

    #[cfg(feature = "media")]
    fn ensure_audio_engine(&mut self) -> Option<&kiriko_audio::AudioEngine> {
        if self.audio_engine.is_none() {
            match kiriko_audio::AudioEngine::new() {
                Ok(engine) => self.audio_engine = Some(engine),
                Err(e) => {
                    self.error = Some(format!("audio: {e}"));
                    return None;
                }
            }
        }
        self.audio_engine.as_ref()
    }

    /// Drain finished audio decodes; auto-load the current preview item's.
    #[cfg(feature = "media")]
    pub fn poll_audio(&mut self) {
        while let Ok((id, result)) = self.audio_rx.try_recv() {
            match result {
                Ok(buffer) => {
                    self.audio_cache.insert(id, std::sync::Arc::new(buffer));
                }
                Err(e) => self.error = Some(format!("audio decode: {e}")),
            }
        }
    }

    #[cfg(feature = "media")]
    pub fn is_playing(&self) -> bool {
        self.comp_playback.is_some() || self.audio_engine.as_ref().is_some_and(|e| e.is_playing())
    }

    /// Advance v0 comp playback; returns true while playing (UI keeps repainting).
    #[cfg(feature = "media")]
    pub fn comp_playback_tick(&mut self) -> bool {
        let Some((started, start_frame)) = self.comp_playback else {
            return false;
        };
        let Some(comp_id) = self.preview_comp else {
            self.comp_playback = None;
            return false;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            self.comp_playback = None;
            return false;
        };
        let (wa_start, wa_end) = self.work_area_frames(comp);
        let fps = comp.frame_rate.fps();
        let frame = start_frame + (started.elapsed().as_secs_f64() * fps) as usize;
        if frame >= wa_end {
            // Loop the work area (07-UI-SPEC transport: loop work area default).
            self.comp_playback = Some((Instant::now(), wa_start));
            self.preview_frame = wa_start;
            self.refresh_preview();
            return true;
        }
        if frame != self.preview_frame {
            self.preview_frame = frame;
            self.refresh_preview();
        }
        true
    }

    #[cfg(feature = "media")]
    pub fn playback_clock(&self) -> Option<f64> {
        self.audio_engine.as_ref().map(|e| e.clock_seconds())
    }

    /// Space: play/pause the previewed footage from the current frame.
    #[cfg(feature = "media")]
    pub fn toggle_play(&mut self) {
        let Some(id) = self.preview_item else { return };
        let Some(fps) = self.preview_fps() else {
            return;
        };
        if self.is_playing() {
            if let Some(engine) = &self.audio_engine {
                engine.pause();
            }
            return;
        }
        let Some(buffer) = self.audio_cache.get(&id).cloned() else {
            self.request_preview_audio(); // will be ready on a later press
            return;
        };
        let start = self.preview_frame as f64 / fps;
        if self.ensure_audio_engine().is_none() {
            return;
        }
        let needs_load = self.audio_loaded != Some(id);
        if needs_load {
            self.audio_loaded = Some(id);
        }
        if let Some(engine) = &self.audio_engine {
            if needs_load {
                engine.load(buffer);
            }
            engine.seek_seconds(start);
            engine.play();
        }
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
        let result = engine
            .results
            .recv_timeout(Duration::from_secs(20))
            .expect("engine replied")
            .expect("decode succeeded");
        let super::preview::PreviewResult::Footage(px) = result else {
            panic!("expected a footage frame");
        };
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
                Ok(Ok(super::preview::PreviewResult::Footage(px))) => {
                    last = Some(px.frame);
                    if px.frame == 59 {
                        break;
                    }
                }
                Ok(Ok(_)) => {}
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
