//! What a vector export needs from the application: footage decoded the way
//! the Viewer decodes it, a way to be stopped, and the memory of how each
//! comp was last exported.
//!
//! Runs on a bridge worker (the preview and the export are not `sync`
//! calls), never the UI thread. The project's lock is held only to copy out
//! the snapshot and the footage paths, never across a decode.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lumit_core::model::{Document, ProjectItem, WorkingSpace};
use uuid::Uuid;

/// The newest preview asked for. An older one still running sees it has been
/// overtaken and stops, so a fast run of edits leaves one preview working
/// rather than a queue of them.
pub(crate) static PREVIEW_EPOCH: AtomicU64 = AtomicU64::new(0);

/// The host one export runs with.
pub(crate) struct Host {
    /// Every footage item's source, resolved while the project was locked.
    sources: BTreeMap<Uuid, Source>,
    /// The preview this is, or `None` for an export that runs to the end.
    epoch: Option<u64>,
    /// The working space to linear Rec.709, when the project composites in
    /// its config's `scene_linear`.
    to_rec709: Option<lumit_colour::matrix::Matrix34>,
}

/// Where a footage item's picture comes from.
struct Source {
    #[cfg(feature = "media")]
    media: Option<lumit_media::sequence::MediaSource>,
    still: bool,
}

impl Host {
    /// A host for `doc`, its footage found from `project`, the file the
    /// project is saved in.
    pub(crate) fn new(project: Option<&Path>, doc: &Document, epoch: Option<u64>) -> Host {
        let sources = doc
            .items
            .iter()
            .filter_map(|item| match item {
                ProjectItem::Footage(f) => Some((f.id, source_of(project, f))),
                _ => None,
            })
            .collect();
        let to_rec709 = (doc.colour.working_space == WorkingSpace::ConfigSceneLinear)
            .then(|| {
                let mut colour = lumit_render::colour::ColourState::default();
                colour.sync(doc);
                let m = colour.loaded()?.rec709_to_working()?;
                lumit_colour::matrix::invert(m).ok()
            })
            .flatten();
        Host {
            sources,
            epoch,
            to_rec709,
        }
    }
}

/// Where `f`'s picture is read from: the same answer
/// `FootageReference::resolve_source` gives the Viewer, from the project's
/// path rather than its open state, so the command line can ask too.
#[cfg(feature = "media")]
fn source_of(project: Option<&Path>, f: &lumit_core::model::FootageItem) -> Source {
    let path = if f.media.absolute_path.is_empty() {
        project
            .and_then(Path::parent)
            .map(|dir| dir.join(&f.media.relative_path))
            .and_then(|p| p.canonicalize().ok())
    } else {
        Some(PathBuf::from(&f.media.absolute_path))
    };
    let media = path.map(|path| lumit_media::sequence::MediaSource {
        path,
        sequence_fps: f.sequence_fps(),
        source_layer: f.source_layer,
        render_pass: f.interpretation.render_pass.clone(),
        interpret: lumit_render::source::interpretation(f),
        stamp: None,
    });
    let still = f.sequence.is_none()
        && media
            .as_ref()
            .is_some_and(|m| is_still_file(&m.path) || m.source_layer.is_some());
    Source { media, still }
}

#[cfg(not(feature = "media"))]
fn source_of(_project: Option<&Path>, _f: &lumit_core::model::FootageItem) -> Source {
    Source { still: false }
}

/// A still image, or a layered file whose layers are stills.
#[cfg(feature = "media")]
fn is_still_file(path: &Path) -> bool {
    if lumit_media::sequence::is_still(path) {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| matches!(e.as_str(), "psd" | "psb" | "ai" | "pdf" | "svg"))
}

impl lumit_lottie::Host for Host {
    fn picture(&self, item: Uuid) -> lumit_lottie::Picture {
        let Some(source) = self.sources.get(&item) else {
            return lumit_lottie::Picture::Missing;
        };
        if !source.still {
            #[cfg(feature = "media")]
            if source.media.is_none() {
                return lumit_lottie::Picture::Missing;
            }
            return lumit_lottie::Picture::Video;
        }
        #[cfg(feature = "media")]
        if let Some(src) = &source.media {
            if let Some(frame) = crate::media::decode_frame(src, 0, None) {
                return lumit_lottie::Picture::Still(still_of(frame));
            }
        }
        lumit_lottie::Picture::Missing
    }

    fn display(&self, linear: [f64; 3]) -> [f64; 3] {
        let rec709 = match &self.to_rec709 {
            Some(m) => lumit_colour::matrix::apply(m, linear.map(|v| v as f32)).map(f64::from),
            None => linear,
        };
        lumit_lottie::colour::rec709_display(rec709)
    }

    fn cancelled(&self) -> bool {
        self.epoch
            .is_some_and(|mine| PREVIEW_EPOCH.load(Ordering::Relaxed) > mine)
    }
}

/// A decoded frame as display sRGB, eight bits, straight alpha.
#[cfg(feature = "media")]
fn still_of(frame: lumit_media::decode::DecodedFrame) -> lumit_lottie::Still {
    let rgba = match frame.format {
        lumit_media::decode::PixelFormat::Srgb8 => frame.rgba,
        // A deep file decodes to linear floats; the file wants eight bits.
        lumit_media::decode::PixelFormat::LinearF32 => (0..frame.rgba.len() / 16)
            .flat_map(|n| {
                let px = lumit_core::pixels::f32_px(&frame.rgba, n);
                lumit_core::pixels::solid_rgba(lumit_core::model::LinearColour(px))
            })
            .collect(),
    };
    lumit_lottie::Still {
        width: frame.width,
        height: frame.height,
        rgba,
    }
}

// ---- Memory: how each comp was last exported --------------------------------

/// What the window opens on for a comp it has exported before: where the file
/// went and the settings it went with. A person's own preference, so it lives
/// beside the export defaults in Lumit's own folder and not in the project.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Remembered {
    pub path: String,
    pub settings: Saved,
}

/// The window's settings as the memory file keeps them. Every field defaults,
/// so a file written by another Lumit still opens and the rest is the window's
/// own defaults.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Saved {
    pub format: String,
    pub text_as_shapes: bool,
    pub embed_images: bool,
    pub fps_num: u32,
    pub fps_den: u32,
    pub work_area: bool,
    pub minify: bool,
    pub players: Vec<String>,
    pub loop_playback: bool,
    pub image_quality: u32,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            format: "lottie".into(),
            text_as_shapes: true,
            embed_images: true,
            fps_num: 0,
            fps_den: 0,
            work_area: true,
            minify: false,
            players: lumit_lottie::check::Players::ALL
                .keys()
                .into_iter()
                .map(String::from)
                .collect(),
            loop_playback: true,
            image_quality: 0,
        }
    }
}

/// The file: comps by id, the one used most recently last.
///
/// Compacts by recency: past [`KEPT`] comps the one exported longest ago is
/// dropped, so the file never grows with every comp a person ever made.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Memory {
    #[serde(default)]
    comps: Vec<(Uuid, Remembered)>,
}

/// How many comps are remembered.
const KEPT: usize = 256;

fn memory_path() -> Option<PathBuf> {
    Some(
        lumit_project::export_defaults_path()?
            .parent()?
            .join("vector-export.json"),
    )
}

fn read_memory() -> Memory {
    memory_path()
        .and_then(|p| lumit_ingress::files::read(&p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// How `comp` was last exported, if it has been.
pub(crate) fn remembered(comp: Uuid) -> Option<Remembered> {
    read_memory()
        .comps
        .into_iter()
        .find(|(id, _)| *id == comp)
        .map(|(_, r)| r)
}

/// Remember how `comp` was exported, newest last. A file that can't be
/// written is not an error anyone has to hear about: the window simply opens
/// on the defaults next time, as it did before there was a memory.
pub(crate) fn remember(comp: Uuid, entry: Remembered) -> std::io::Result<()> {
    let Some(path) = memory_path() else {
        return Ok(());
    };
    let mut memory = read_memory();
    memory.comps.retain(|(id, _)| *id != comp);
    memory.comps.push((comp, entry));
    let over = memory.comps.len().saturating_sub(KEPT);
    memory.comps.drain(..over);
    if let Some(dir) = path.parent() {
        lumit_ingress::files::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec_pretty(&memory).map_err(std::io::Error::other)?;
    replace(&path, &bytes)
}

/// Write `bytes` to `path` beside it first and move them over, so a write
/// that fails leaves the old file as it was.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".part");
    let temp = PathBuf::from(temp);
    let done = lumit_ingress::files::write(&temp, bytes)
        .and_then(|()| lumit_ingress::files::rename(&temp, path));
    if done.is_err() {
        let _ = lumit_ingress::files::remove_file(&temp);
    }
    done
}

/// Write an export to `file`, its pictures beside it. Answers the files
/// written, `file` first.
pub(crate) fn write_out(out: &lumit_lottie::Exported, file: &Path) -> std::io::Result<Vec<String>> {
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    if !dir.as_os_str().is_empty() {
        lumit_ingress::files::create_dir_all(&dir)?;
    }
    let mut written = vec![file.to_string_lossy().into_owned()];
    for (name, bytes) in &out.beside {
        let beside = dir.join(name);
        replace(&beside, bytes)?;
        written.push(beside.to_string_lossy().into_owned());
    }
    replace(file, &out.bytes)?;
    Ok(written)
}

/// Export `comp` of `doc` to `file`, with any pictures beside it, and answer
/// what won't come across as lines to tell. The command line and scripts
/// write through this; the window's export also remembers how it went.
pub(crate) fn write_to(
    doc: &Arc<Document>,
    project: Option<&Path>,
    comp: Uuid,
    settings: &lumit_lottie::settings::Settings,
    file: &Path,
) -> Result<Vec<String>, String> {
    let source = Host::new(project, doc, None);
    let out = lumit_lottie::export(doc, comp, settings, &source).map_err(|e| e.to_string())?;
    write_out(&out, file).map_err(|e| e.to_string())?;
    Ok(out
        .issues
        .iter()
        .map(|issue| format!("{}: {}", issue.place.join(" > "), issue.english))
        .collect())
}

/// The document a preview or an export reads, and its host, taken under the
/// project's read lock and released straight away.
pub(crate) fn snapshot(
    project: &Arc<std::sync::RwLock<crate::api::state::LumitBridgeState>>,
    epoch: Option<u64>,
) -> Option<(Arc<Document>, Host, Option<PathBuf>)> {
    let (doc, path) = {
        let state = project.read().ok()?;
        (state.store.snapshot(), state.path.clone())
    };
    let host = Host::new(path.as_deref(), &doc, epoch);
    Some((doc, host, path))
}
