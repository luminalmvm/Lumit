//! Stills: frames kept with the grade that made them, in the user's library
//! where every project sees them.
//!
//! A still is two files in an album's folder, `<library>/Stills/<album>/`:
//! `<name>.lumstill`, which says where it came from and holds the grade as the
//! text Copy effects makes, and `<name>.png`, the comp's frame as the Viewer
//! showed it. The folders are the only record of the albums.
//!
//! Every call here reads or writes files, so none of them runs on the
//! interface's thread.

use std::path::{Path, PathBuf};

use flutter_rust_bridge::frb;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::composition::{colour_view_pair, CompositionReference};
use crate::api::effect::BridgeEffectInstance;
use crate::api::grade_shots::{shot_effects, shot_in, shot_name};
use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// The folder in the preset library the albums live in.
#[frb(ignore)]
pub(crate) const STILLS_FOLDER: &str = "Stills";

const STILL_EXTENSION: &str = "lumstill";
const STILL_FORMAT: u32 = 1;

/// One still, as the gallery lists it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeStill {
    /// The `.lumstill` file, which names the still in every call here.
    pub path: String,
    pub png_path: String,
    pub album: String,
    pub name: String,
    pub label: u8,
    /// Where it came from, by name.
    pub project: String,
    pub comp: String,
    pub shot: String,
    /// The comp frame, and its timecode.
    pub frame: i64,
    pub timecode: String,
}

/// What a `.lumstill` file holds. Fields a later Lumit adds are kept.
#[frb(ignore)]
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StillFile {
    format: u32,
    name: String,
    label: u8,
    project: String,
    comp: String,
    shot: String,
    frame: i64,
    timecode: String,
    /// The shot's Colour effects, as the text Copy effects makes.
    grade: String,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl CompositionReference {
    /// Keep comp `frame` as a still named `name` in `album`: the shot's grade
    /// and where it came from, and the frame at the comp's size seen through
    /// `colour_view` (`[display, view]`, as the Viewer's first view is). An
    /// empty name takes the shot's. The album is made if it is not there,
    /// and a name already taken in it gets " 2", " 3" after it.
    pub fn grab_still(
        &self,
        frame: u64,
        layer: Uuid,
        clip: Option<Uuid>,
        album: String,
        name: String,
        colour_view: Option<Vec<String>>,
    ) -> Result<BridgeStill, BridgeError> {
        let (doc, project) = {
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let project = state
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            (state.store.snapshot(), project)
        };
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let (shown, clip_shown) = shot_in(comp, layer, clip)?;
        let shot = shot_name(&doc, shown, clip_shown);
        let folder = album_folder(&album)?;
        let name = match name.trim() {
            "" => shot.clone(),
            name => name.to_owned(),
        };

        // A packed project's LUTs are copied out with the paths they had
        // before it was packed, as Copy effects copies them.
        let mut original = None;
        if doc.packed.values().any(|entry| entry.original.is_some()) {
            let mut kept = (*doc).clone();
            lumit_project::keep_original_paths(&mut kept);
            original = Some(kept);
        }
        let source = original.as_ref().unwrap_or(&doc);
        let grade = grade_text(source, self.id, layer, clip)?;

        let start = comp.frame_rate.frame_at(comp.start_timecode());
        let timecode = lumit_core::interpretation::StartTimecode {
            frame: start.saturating_add(frame as i64).max(0) as u64,
            drop_frame: false,
        }
        .text(comp.frame_rate);
        let still = StillFile {
            format: STILL_FORMAT,
            name: name.clone(),
            label: clip_shown.and_then(|c| c.label).unwrap_or(shown.label),
            project,
            comp: comp.name.clone(),
            shot,
            frame: frame as i64,
            timecode,
            grade,
            extra: serde_json::Map::new(),
        };
        let text = serde_json::to_string_pretty(&still).map_err(|_| BridgeError::WriteFailed)?;

        let (rgba, width, height) =
            crate::render::viewed_frame(&doc, self.id, frame, colour_view_pair(colour_view))
                .ok_or(BridgeError::InvalidWorkerState)?;
        let png =
            crate::render::png_of(&rgba, width, height).map_err(|_| BridgeError::WriteFailed)?;

        std::fs::create_dir_all(&folder).map_err(|_| BridgeError::WriteFailed)?;
        let stem = free_stem(&folder, &safe_name(&name)?, None)?;
        let (path, png_path) = still_paths(&folder, &stem);
        // The picture first, so a listed still always has one.
        write_new(&png_path, &png)?;
        if let Err(e) = write_new(&path, text.as_bytes()) {
            let _ = std::fs::remove_file(&png_path);
            return Err(e);
        }
        info_of(&path, still)
    }
}

/// Every album, by name.
pub fn list_still_albums() -> Result<Vec<String>, BridgeError> {
    let Ok(entries) = std::fs::read_dir(stills_root()?) else {
        return Ok(Vec::new());
    };
    let mut albums: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    albums.sort_by_key(|a| a.to_lowercase());
    Ok(albums)
}

/// Make a new album, and answer the name it was given: `name` made safe for a
/// folder, with " 2", " 3" after it if an album has it already.
pub fn make_still_album(name: String) -> Result<String, BridgeError> {
    let root = stills_root()?;
    let album = free_name(&root, &safe_name(&name)?, None)?;
    std::fs::create_dir_all(root.join(&album)).map_err(|_| BridgeError::WriteFailed)?;
    Ok(album)
}

/// Give an album a new name, and answer the name it was given, as
/// [`make_still_album`] does.
pub fn rename_still_album(album: String, name: String) -> Result<String, BridgeError> {
    let from = album_folder(&album)?;
    let root = stills_root()?;
    let renamed = free_name(&root, &safe_name(&name)?, Some(&album))?;
    if renamed != album {
        std::fs::rename(&from, root.join(&renamed)).map_err(|_| BridgeError::WriteFailed)?;
    }
    Ok(renamed)
}

/// Delete an album and every still in it.
pub fn delete_still_album(album: String) -> Result<(), BridgeError> {
    std::fs::remove_dir_all(album_folder(&album)?).map_err(|_| BridgeError::WriteFailed)
}

/// The stills in an album, in the order they were grabbed. A file that
/// isn't a still, or whose picture is missing, is passed over.
pub fn list_stills(album: String) -> Result<Vec<BridgeStill>, BridgeError> {
    let folder = album_folder(&album)?;
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(Vec::new());
    };
    let mut stills: Vec<(std::time::SystemTime, BridgeStill)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == STILL_EXTENSION))
        .filter_map(|p| {
            let still = info_of(&p, read_still(&p).ok()?).ok()?;
            // A rename keeps a file's time, so the picture's is when it was
            // grabbed.
            let grabbed = std::fs::metadata(&still.png_path)
                .and_then(|m| m.modified())
                .ok()?;
            Some((grabbed, still))
        })
        .collect();
    stills.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    Ok(stills.into_iter().map(|(_, still)| still).collect())
}

/// Give a still a new name. Its files take the name too, with " 2", " 3"
/// after it if another still in the album has it.
pub fn rename_still(path: String, name: String) -> Result<BridgeStill, BridgeError> {
    let path = own_still(&path)?;
    let mut still = read_still(&path)?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let folder = path.parent().ok_or(BridgeError::InvalidItem)?;
    let stem = free_stem(folder, &safe_name(&name)?, Some(&path))?;
    still.name = name;
    let text = serde_json::to_string_pretty(&still).map_err(|_| BridgeError::WriteFailed)?;
    let (to, to_png) = still_paths(folder, &stem);
    let (_, png) = still_paths(folder, &stem_of(&path)?);
    if to == path {
        // Only the name inside changes. Written beside it and moved over it,
        // so a failed write leaves the old file whole.
        let part = path.with_extension("part");
        std::fs::write(&part, text)
            .and_then(|()| std::fs::rename(&part, &path))
            .map_err(|_| {
                let _ = std::fs::remove_file(&part);
                BridgeError::WriteFailed
            })?;
        return info_of(&path, still);
    }
    // The new file first, then the picture, and only then is the old file let
    // go, so a failure on the way leaves the still where it was.
    write_new(&to, text.as_bytes())?;
    if std::fs::rename(&png, &to_png).is_err() {
        let _ = std::fs::remove_file(&to);
        return Err(BridgeError::WriteFailed);
    }
    let _ = std::fs::remove_file(&path);
    info_of(&to, still)
}

/// Delete a still: its file and its picture.
pub fn delete_still(path: String) -> Result<(), BridgeError> {
    let path = own_still(&path)?;
    let (_, png) = still_paths(
        path.parent().ok_or(BridgeError::InvalidItem)?,
        &stem_of(&path)?,
    );
    std::fs::remove_file(&path).map_err(|_| BridgeError::WriteFailed)?;
    let _ = std::fs::remove_file(png);
    Ok(())
}

/// Move a still into another album, made if it is not there. Its name gets
/// " 2", " 3" after it if a still there has it.
pub fn move_still(path: String, album: String) -> Result<BridgeStill, BridgeError> {
    let path = own_still(&path)?;
    let still = read_still(&path)?;
    let from = path.parent().ok_or(BridgeError::InvalidItem)?;
    let folder = album_folder(&album)?;
    if folder == from {
        return info_of(&path, still);
    }
    std::fs::create_dir_all(&folder).map_err(|_| BridgeError::WriteFailed)?;
    let stem = stem_of(&path)?;
    let (_, png) = still_paths(from, &stem);
    let (to, to_png) = still_paths(&folder, &free_stem(&folder, &stem, None)?);
    std::fs::rename(&png, &to_png).map_err(|_| BridgeError::WriteFailed)?;
    if std::fs::rename(&path, &to).is_err() {
        let _ = std::fs::rename(&to_png, &png);
        return Err(BridgeError::WriteFailed);
    }
    info_of(&to, still)
}

/// A still's grade, as the text Copy effects makes, for Paste effects.
pub fn read_still_grade(path: String) -> Result<String, BridgeError> {
    Ok(read_still(&own_still(&path)?)?.grade)
}

impl LayerReference {
    /// This shot's stack with the still at `path` graded on it in place of
    /// its own grade, to preview and never to write: the old grade stays in
    /// the list switched off, and the still's comes right after it. The list
    /// starts with the shot's own first effect, which is how a clip's preview
    /// is found on its clip, so a clip with an empty stack has none.
    #[frb(sync)]
    pub fn still_grade_preview(
        &self,
        clip: Option<Uuid>,
        path: String,
    ) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let text = read_still(&own_still(&path)?)?.grade;
        let preset =
            lumit_core::preset::from_json(&text).map_err(|_| BridgeError::InvalidPreset)?;
        let mut fresh = lumit_core::preset::instantiated(&preset);
        self.migrate_in(&mut fresh, &|_, _| false)?;

        let comp = self.composition()?;
        let (layer, clip) = shot_in(&comp, self.layer_id, clip)?;
        let mut offset = layer.start_offset.0;
        if let Some(clip) = clip {
            offset = offset
                .checked_add(clip.place_start)
                .map_err(|_| BridgeError::InvalidTime)?;
        }
        let mut stack = shot_effects(layer, clip).to_vec();
        if clip.is_some() && stack.is_empty() {
            return Err(BridgeError::InvalidEffect);
        }
        let at = stack
            .iter()
            .rposition(lumit_render::grade_lut::in_grade)
            .map_or(stack.len(), |i| i + 1);
        for e in stack
            .iter_mut()
            .filter(|e| lumit_render::grade_lut::in_grade(e))
        {
            e.enabled = false;
        }
        stack.splice(at..at, fresh);
        Ok(stack
            .into_iter()
            .map(|e| BridgeEffectInstance::new(e, offset))
            .collect())
    }
}

/// The shot's Colour effects as the text Copy effects makes of them, wiring
/// and all for a layer's.
fn grade_text(
    doc: &lumit_core::model::Document,
    comp: Uuid,
    layer: Uuid,
    clip: Option<Uuid>,
) -> Result<String, BridgeError> {
    let comp = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
    let (shown, clip_shown) = shot_in(comp, layer, clip)?;
    let stack = shot_effects(shown, clip_shown);
    let grade: Vec<_> = stack
        .iter()
        .filter(|e| lumit_render::grade_lut::in_grade(e))
        .cloned()
        .collect();
    let name = grade
        .first()
        .map(|e| e.effect.match_name.clone())
        .unwrap_or_default();
    // A clip has no graph, so nothing is wired to its effects.
    let wiring = match clip_shown {
        Some(_) => None,
        None => {
            let ids: Vec<Uuid> = grade.iter().map(|e| e.id).collect();
            lumit_core::preset::wiring_of(&shown.graph, stack, &ids)
        }
    };
    lumit_core::preset::to_json_wired(&name, &grade, wiring).map_err(|_| BridgeError::InvalidPreset)
}

/// Where the albums are: a folder in the preset library.
fn stills_root() -> Result<PathBuf, BridgeError> {
    Ok(lumit_project::presets_dir()
        .ok_or(BridgeError::WriteFailed)?
        .join(STILLS_FOLDER))
}

/// An album's folder. Refused for a name that is not one an album could have,
/// so no album reaches outside the library.
fn album_folder(album: &str) -> Result<PathBuf, BridgeError> {
    if safe_name(album)? != album {
        return Err(BridgeError::InvalidItem);
    }
    Ok(stills_root()?.join(album))
}

/// `path` as a still in the library, refused for any other file.
fn own_still(path: &str) -> Result<PathBuf, BridgeError> {
    let path = PathBuf::from(path);
    let inside = std::fs::canonicalize(&path)
        .ok()
        .zip(std::fs::canonicalize(stills_root()?).ok())
        .is_some_and(|(file, root)| file.starts_with(root));
    if !inside || !path.is_file() || path.extension().is_none_or(|x| x != STILL_EXTENSION) {
        return Err(BridgeError::InvalidItem);
    }
    Ok(path)
}

/// A still's picture as eight-bit RGBA, straight alpha, display sRGB.
#[frb(ignore)]
pub(crate) fn still_png(path: &str) -> Result<Vec<u8>, BridgeError> {
    let path = own_still(path)?;
    let folder = path.parent().ok_or(BridgeError::InvalidItem)?;
    let (_, png) = still_paths(folder, &stem_of(&path)?);
    let file = std::fs::File::open(png).map_err(|_| BridgeError::ReadFailed)?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| BridgeError::ReadFailed)?;
    let mut data = vec![0u8; reader.output_buffer_size().ok_or(BridgeError::ReadFailed)?];
    let info = reader
        .next_frame(&mut data)
        .map_err(|_| BridgeError::ReadFailed)?;
    data.truncate(info.buffer_size());
    Ok(match info.color_type {
        png::ColorType::Rgba => data,
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        _ => data.iter().flat_map(|&v| [v, v, v, 255]).collect(),
    })
}

#[frb(ignore)]
fn read_still(path: &Path) -> Result<StillFile, BridgeError> {
    let text = std::fs::read_to_string(path).map_err(|_| BridgeError::ReadFailed)?;
    serde_json::from_str(&text).map_err(|_| BridgeError::ReadFailed)
}

#[frb(ignore)]
fn info_of(path: &Path, still: StillFile) -> Result<BridgeStill, BridgeError> {
    let folder = path.parent().ok_or(BridgeError::InvalidItem)?;
    let (_, png) = still_paths(folder, &stem_of(path)?);
    Ok(BridgeStill {
        path: path.to_string_lossy().into_owned(),
        png_path: png.to_string_lossy().into_owned(),
        album: folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        name: still.name,
        label: still.label,
        project: still.project,
        comp: still.comp,
        shot: still.shot,
        frame: still.frame,
        timecode: still.timecode,
    })
}

fn stem_of(path: &Path) -> Result<String, BridgeError> {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or(BridgeError::InvalidItem)
}

/// The two files a still named `stem` is in `folder`.
fn still_paths(folder: &Path, stem: &str) -> (PathBuf, PathBuf) {
    (
        folder.join(format!("{stem}.{STILL_EXTENSION}")),
        folder.join(format!("{stem}.png")),
    )
}

/// A name every system takes for a file or a folder: no character any of them
/// refuses, no space or dot at the end, none of Windows' device names, and
/// not too long.
fn safe_name(name: &str) -> Result<String, BridgeError> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .take(100)
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).trim_start().to_owned();
    if cleaned.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let device = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && device.ends_with(|c: char| c.is_ascii_digit()));
    Ok(if reserved {
        format!("{cleaned} still")
    } else {
        cleaned
    })
}

/// `base`, or `base 2`, `base 3` and on, the first that no still in `folder`
/// has. `own` is a still's own file, whose name it may keep.
fn free_stem(folder: &Path, base: &str, own: Option<&Path>) -> Result<String, BridgeError> {
    numbered(base, |stem| {
        let (still, png) = still_paths(folder, stem);
        own.is_some_and(|own| own == still) || (!still.exists() && !png.exists())
    })
}

/// `base`, or `base 2` and on, the first that is no other folder's in `root`.
fn free_name(root: &Path, base: &str, own: Option<&str>) -> Result<String, BridgeError> {
    numbered(base, |name| own == Some(name) || !root.join(name).exists())
}

fn numbered(base: &str, free: impl Fn(&str) -> bool) -> Result<String, BridgeError> {
    if free(base) {
        return Ok(base.to_owned());
    }
    (2..10_000)
        .map(|n| format!("{base} {n}"))
        .find(|name| free(name))
        .ok_or(BridgeError::WriteFailed)
}

/// Write a file that must not be there already, so nothing is written over.
/// One that fails part way is taken away again.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), BridgeError> {
    use std::io::Write;
    let mut file = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| BridgeError::WriteFailed)?;
    if file.write_all(bytes).is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(BridgeError::WriteFailed);
    }
    Ok(())
}
