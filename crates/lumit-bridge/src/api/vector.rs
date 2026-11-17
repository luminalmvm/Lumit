//! Vector export and Lottie import: what the Lottie and SVG window and the
//! import route call.
//!
//! The window previews by asking for the whole export in memory
//! ([`CompositionReference::vector_preview`]), which is the same call the
//! export makes, so the player plays the bytes that would be written and the
//! check lists what they leave out. Neither is `sync`: an export bakes
//! expressions and decodes pictures, so it runs on a bridge worker, and a
//! newer preview stops an older one still running.
//!
//! An issue crosses as a stable reason id and its facts, never a sentence
//! (docs/17 §"Display text"); `english` rides along for an interface with no
//! sentence for the id yet.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use flutter_rust_bridge::frb;
use lumit_core::ops::Op;
use lumit_lottie::check::{Outcome, Players};
use lumit_lottie::settings::{Format, ImageMode, Settings, Span, TextMode};

use crate::api::composition::CompositionReference;
use crate::api::import::{
    BridgeImportArg, BridgeImportOutcome, BridgeImportReport, BridgeImportRow,
};
use crate::api::layer::LayerReference;
use crate::api::project::ProjectReference;
use crate::api::BridgeError;
use crate::vector_host::{self, Remembered, Saved, PREVIEW_EPOCH};

/// How a comp goes out as a vector file.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeVectorSettings {
    /// `lottie`, `dotlottie` or `svg`.
    pub format: String,
    /// Text as outlines rather than as text naming its font.
    pub text_as_shapes: bool,
    /// Pictures inside the file rather than beside it.
    pub embed_images: bool,
    /// The rate to export at, as a fraction; zero over zero is the comp's own.
    pub fps_num: u32,
    pub fps_den: u32,
    /// The work area when the comp has one, rather than the whole comp.
    pub work_area: bool,
    pub minify: bool,
    /// The players the check judges against, by key (`web`, `web_canvas`,
    /// `dotlottie`, `ios`, `android`, `flutter`). Never changes the bytes.
    pub players: Vec<String>,
    /// An SVG plays on repeat, and a `.lottie` asks its player to loop.
    pub loop_playback: bool,
    /// Pictures as JPEG at this quality, 1 to 100, where they have no
    /// see-through pixel; 0 keeps every picture lossless.
    pub image_quality: u32,
}

/// One thing that won't come across.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeVectorIssue {
    /// `left_out`, `some_players` or `changed`.
    pub outcome: String,
    /// The stable id of its sentence, such as `effect`.
    pub reason: String,
    pub args: Vec<BridgeImportArg>,
    /// The engine's English, for an id the interface has no sentence for.
    pub english: String,
    /// The comp the layer is in.
    pub comp: CompositionReference,
    /// The layer, when the issue is about one.
    pub layer: Option<LayerReference>,
    /// Comp, layer and part names, for the reader.
    pub place: Vec<String>,
    /// The ticked players that miss it, by key.
    pub players: Vec<String>,
}

/// An export made in memory: the file, what it leaves out, and its facts.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeVectorPreview {
    /// The file as it would be written.
    pub bytes: Vec<u8>,
    /// What it and the pictures beside it come to on disk.
    pub size: u64,
    pub issues: Vec<BridgeVectorIssue>,
    pub frames: u32,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    /// The comp second the file starts at: the work area's start, or 0.
    pub start: f64,
    /// The epoch this preview was asked with.
    pub epoch: u64,
}

/// The settings a fresh window opens on.
#[frb(sync)]
pub fn vector_settings_default() -> BridgeVectorSettings {
    from_saved(Saved::default())
}

/// The file extension for a format key, without the dot.
#[frb(sync)]
pub fn vector_extension(format: String) -> String {
    Format::from_key(&format)
        .unwrap_or_default()
        .extension()
        .to_owned()
}

impl CompositionReference {
    /// This comp exported in memory as `settings` ask. `epoch` names this
    /// request: a newer one stops it, and the window drops any answer older
    /// than the newest it asked for.
    pub fn vector_preview(
        &self,
        settings: BridgeVectorSettings,
        epoch: u64,
    ) -> Result<BridgeVectorPreview, BridgeError> {
        PREVIEW_EPOCH.fetch_max(epoch, Ordering::Relaxed);
        let project = self.project()?;
        let (doc, host, _) =
            vector_host::snapshot(&project, Some(epoch)).ok_or(BridgeError::ReadFailed)?;
        let mut engine = to_engine(&settings, "animation");
        let out = lumit_lottie::export(&doc, self.id, &engine, &host).map_err(failed)?;
        let size =
            out.bytes.len() as u64 + out.beside.iter().map(|(_, b)| b.len() as u64).sum::<u64>();
        let issues = out.issues.iter().map(|i| self.issue(i)).collect();
        // The player can't see pictures beside the file, so it is handed them inside.
        let out = if out.beside.is_empty() {
            out
        } else {
            engine.images = ImageMode::Embed;
            lumit_lottie::export(&doc, self.id, &engine, &host).map_err(failed)?
        };
        Ok(BridgeVectorPreview {
            size,
            issues,
            frames: out.frames,
            fps: out.fps,
            width: out.width,
            height: out.height,
            start: doc.comp(self.id).map_or(0.0, |comp| {
                lumit_lottie::time::Timebase::of(comp, &engine).start
            }),
            bytes: out.bytes,
            epoch,
        })
    }

    /// Write this comp to `path` as `settings` ask, with any pictures set to
    /// go beside it, and remember both for the next time. Answers the files
    /// written.
    pub fn vector_export(
        &self,
        settings: BridgeVectorSettings,
        path: String,
    ) -> Result<Vec<String>, BridgeError> {
        let file = PathBuf::from(&path);
        if path.trim().is_empty() {
            return Err(BridgeError::EmptyPath);
        }
        let project = self.project()?;
        let (doc, host, _) =
            vector_host::snapshot(&project, None).ok_or(BridgeError::ReadFailed)?;
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "animation".into());
        let engine = to_engine(&settings, &stem);
        let out = lumit_lottie::export(&doc, self.id, &engine, &host).map_err(failed)?;
        let written = vector_host::write_out(&out, &file).map_err(|_| BridgeError::WriteFailed)?;
        // Not being able to remember is not a failed export.
        let _ = vector_host::remember(
            self.id,
            Remembered {
                path,
                settings: to_saved(&settings),
            },
        );
        Ok(written)
    }

    /// The settings this comp was last exported with, or the window's own
    /// defaults when it never has been.
    #[frb(sync)]
    pub fn vector_settings(&self) -> BridgeVectorSettings {
        vector_host::remembered(self.id)
            .map_or_else(vector_settings_default, |r| from_saved(r.settings))
    }

    /// Where this comp's `format` file goes unless somebody says otherwise:
    /// where it went last time, with the format's extension; else where the
    /// export defaults put a file, named after the comp. `None` when the
    /// defaults ask every time.
    #[frb(sync)]
    pub fn vector_path(&self, format: String) -> Result<Option<String>, BridgeError> {
        let ext = vector_extension(format);
        if let Some(r) = vector_host::remembered(self.id) {
            return Ok(Some(
                PathBuf::from(r.path)
                    .with_extension(&ext)
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        let project = self.project()?;
        let (name, project_path) = {
            let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let name = doc
                .comp(self.id)
                .ok_or(BridgeError::InvalidComp)?
                .name
                .clone();
            (name, state.path.clone())
        };
        let defaults = lumit_render::export_defaults::ExportDefaults::load_default();
        let folder = match defaults.policy() {
            lumit_render::export_defaults::DESTINATION_FOLDER if !defaults.folder.is_empty() => {
                Some(PathBuf::from(&defaults.folder))
            }
            lumit_render::export_defaults::DESTINATION_PROJECT => {
                project_path.and_then(|p| p.parent().map(Path::to_path_buf))
            }
            _ => None,
        };
        Ok(folder.map(|f| {
            f.join(format!("{}.{ext}", file_name(&name)))
                .to_string_lossy()
                .into_owned()
        }))
    }

    /// `issue` as it crosses, its comp and layer as references the window can
    /// select.
    #[frb(ignore)]
    fn issue(&self, issue: &lumit_lottie::check::Issue) -> BridgeVectorIssue {
        BridgeVectorIssue {
            outcome: issue.outcome.key().to_owned(),
            reason: issue.reason.to_owned(),
            args: issue
                .args
                .iter()
                .map(|(name, value)| BridgeImportArg {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
            english: issue.english.clone(),
            comp: CompositionReference {
                project: self.project,
                id: issue.comp,
            },
            layer: issue
                .layer
                .map(|layer| LayerReference::new(self.project, issue.comp, layer)),
            place: issue.place.clone(),
            players: if issue.outcome == Outcome::SomePlayers {
                issue.players.keys().into_iter().map(String::from).collect()
            } else {
                Vec::new()
            },
        }
    }
}

/// Whether `path` is a Lottie: a `.lottie`, or a `.json` object with `fr`,
/// `ip` and `op` among its top-level keys. A file too large to import is not one.
pub fn is_lottie_file(path: String) -> bool {
    let path = Path::new(&path);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ext == "lottie" {
        return true;
    }
    if ext != "json" {
        return false;
    }
    let Ok(json) = lumit_ingress::read_capped(path, lumit_lottie::import::LIMITS.bytes) else {
        return false;
    };
    let json = json.strip_prefix("\u{feff}".as_bytes()).unwrap_or(&json);
    if json.trim_ascii_start().first() != Some(&b'{') {
        return false;
    }
    // Walks the bytes once, skipping strings by their quotes and nested values
    // by depth, and notes each key of the outer object that is wanted.
    let wanted: [&[u8]; 3] = [b"fr", b"ip", b"op"];
    let mut found = [false; 3];
    let mut depth = 0u32;
    let mut string = None;
    let mut escaped = false;
    for (i, &b) in json.iter().enumerate() {
        if let Some(from) = string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                string = None;
                let rest = json.get(i + 1..).unwrap_or_default();
                if depth == 1 && rest.trim_ascii_start().first() == Some(&b':') {
                    let key = json.get(from..i).unwrap_or_default();
                    for (seen, w) in found.iter_mut().zip(wanted) {
                        *seen |= key == w;
                    }
                }
            }
            continue;
        }
        match b {
            b'"' => string = Some(i + 1),
            b'{' | b'[' => depth = depth.saturating_add(1),
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    found.iter().all(|&f| f)
}

/// What a Lottie import made, and what didn't come across as it was.
#[frb(non_opaque)]
pub struct BridgeLottieImport {
    pub comp: CompositionReference,
    pub report: BridgeImportReport,
}

impl ProjectReference {
    /// Bring the Lottie at `path` into this project as a comp (its precomps as
    /// comps in a folder named after the file), as one undo step. Its pictures
    /// are written beside the file, in a folder named after it.
    pub fn import_lottie(&self, path: String) -> Result<BridgeLottieImport, BridgeError> {
        let source = PathBuf::from(&path);
        let bytes = lumit_ingress::read_capped(&source, lumit_lottie::import::LIMITS.bytes)
            .map_err(|e| match e {
                lumit_ingress::IngressError::Io { .. } => BridgeError::ReadFailed,
                e => failed(e.into()),
            })?;
        let imported = lumit_lottie::import::import(&bytes, &source).map_err(failed)?;
        for (file, data) in &imported.files {
            // The same picture from an earlier import of the same file is
            // left as it is.
            if lumit_ingress::files::read(file).is_ok_and(|old| old == *data) {
                continue;
            }
            if let Some(dir) = file.parent() {
                lumit_ingress::files::create_dir_all(dir).map_err(|_| BridgeError::WriteFailed)?;
            }
            lumit_ingress::files::write(file, data).map_err(|_| BridgeError::WriteFailed)?;
        }
        let footage: Vec<PathBuf> = imported
            .files
            .iter()
            .map(|(file, _)| file.clone())
            .collect();
        {
            let state = self.state()?;
            let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
            let base = state.store.snapshot().items.len();
            let ops = imported
                .items
                .into_iter()
                .enumerate()
                .map(|(n, item)| Op::AddItem {
                    index: base + n,
                    item: Box::new(item),
                })
                .collect();
            state
                .store
                .commit(Op::Batch { ops })
                .map_err(BridgeError::OpError)?;
        }
        // Outside the lock: the probe reads each picture's size in the background.
        #[cfg(feature = "media")]
        for file in footage {
            crate::probe::request(file);
        }
        #[cfg(not(feature = "media"))]
        let _ = footage;
        let rows: Vec<BridgeImportRow> =
            imported.report.iter().map(BridgeImportRow::from).collect();
        let count = |o: BridgeImportOutcome| rows.iter().filter(|r| r.outcome == o).count() as u32;
        Ok(BridgeLottieImport {
            comp: CompositionReference {
                project: self.id,
                id: imported.main,
            },
            report: BridgeImportReport {
                imported: imported.layers,
                adjusted: count(BridgeImportOutcome::Adjusted),
                placeholders: count(BridgeImportOutcome::Placeholder),
                skipped: count(BridgeImportOutcome::Skipped),
                rows,
            },
        })
    }
}

/// An export's or an import's failure, as the window shows it.
fn failed(e: lumit_lottie::LottieError) -> BridgeError {
    BridgeError::ExportFailed(e.to_string())
}

/// `name` as a file name: the characters no file system takes are dropped.
fn file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| {
            !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control()
        })
        .collect();
    let clean = clean.trim().trim_end_matches('.').to_owned();
    if clean.is_empty() {
        "animation".into()
    } else {
        clean
    }
}

fn to_engine(s: &BridgeVectorSettings, stem: &str) -> Settings {
    Settings {
        format: Format::from_key(&s.format).unwrap_or_default(),
        text: if s.text_as_shapes {
            TextMode::Shapes
        } else {
            TextMode::Text
        },
        images: if s.embed_images {
            ImageMode::Embed
        } else {
            ImageMode::Beside
        },
        frame_rate: (s.fps_num > 0 && s.fps_den > 0)
            .then(|| lumit_core::time::FrameRate::new(s.fps_num, s.fps_den).ok())
            .flatten(),
        span: if s.work_area {
            Span::WorkArea
        } else {
            Span::Whole
        },
        minify: s.minify,
        players: Players::from_keys(s.players.iter().map(String::as_str)),
        loop_playback: s.loop_playback,
        image_quality: u8::try_from(s.image_quality)
            .ok()
            .filter(|q| (1..=100).contains(q)),
        file_stem: stem.to_owned(),
    }
}

fn to_saved(s: &BridgeVectorSettings) -> Saved {
    Saved {
        format: s.format.clone(),
        text_as_shapes: s.text_as_shapes,
        embed_images: s.embed_images,
        fps_num: s.fps_num,
        fps_den: s.fps_den,
        work_area: s.work_area,
        minify: s.minify,
        players: s.players.clone(),
        loop_playback: s.loop_playback,
        image_quality: s.image_quality,
    }
}

fn from_saved(s: Saved) -> BridgeVectorSettings {
    BridgeVectorSettings {
        format: s.format,
        text_as_shapes: s.text_as_shapes,
        embed_images: s.embed_images,
        fps_num: s.fps_num,
        fps_den: s.fps_den,
        work_area: s.work_area,
        minify: s.minify,
        players: s.players,
        loop_playback: s.loop_playback,
        image_quality: s.image_quality,
    }
}
