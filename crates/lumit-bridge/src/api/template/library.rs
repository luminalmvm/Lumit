//! The library of saved graphics templates: the `.lumt` files in the
//! templates folder, saving a composition there, and bringing one into a
//! project (`lumit_project::template_file`).
//!
//! Listing runs on the UI thread and reads only each file's first two
//! entries. Saving, the pictures, placing and updating read or write whole
//! files and render, so they are not `sync` and run off it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use flutter_rust_bridge::frb;
use image::ImageEncoder;
use lumit_core::model::{Document, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::template::Template;
use lumit_project::template_file::{self, FontFile, Pictures, TemplateInfo};
use uuid::Uuid;

use crate::api::{composition::CompositionReference, project::ProjectReference, BridgeError};

/// How wide the poster is, and each frame of the strip a tile scrubs.
const POSTER_WIDTH: u32 = 640;
const STRIP_WIDTH: u32 = 320;
/// How many frames the strip holds, spread evenly over the template.
const STRIP_FRAMES: u32 = 24;

/// One template in the library, as a tile shows it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLibraryTemplate {
    /// The `.lumt` file.
    pub path: String,
    pub name: String,
    /// The folder it is in, under the templates folder, parted by `/`. Empty
    /// at the top.
    pub group: String,
    /// The same through every version of one template.
    pub id: Uuid,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: f64,
    pub seconds: f64,
    /// Its controls' names, which a search looks in.
    pub controls: Vec<String>,
    /// The fonts it names that are neither on this machine nor in the file.
    pub missing_fonts: Vec<String>,
}

/// A font a composition's text uses, for the Save dialogue.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFontUse {
    pub family: String,
    pub face: String,
    /// It is on this machine, so it can be packed.
    pub installed: bool,
}

/// A template in the project that the library has a newer version of.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTemplateUpdate {
    /// The template's composition in the project.
    pub comp: Uuid,
    pub name: String,
    /// The version the project has, and the library's.
    pub version: u32,
    pub latest: u32,
    /// The library's file.
    pub path: String,
}

/// The templates folder, made if it is not there. `None` where Lumit has no
/// folders of its own.
fn folder() -> Option<PathBuf> {
    let dir = lumit_project::templates_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Make the fonts that came inside templates known, once for the life of the
/// program: a project that places a template with a packed font draws it
/// from the first frame, before the library is ever listed.
#[frb(ignore)]
pub(crate) fn register_fonts() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        if let Some(dir) = template_file::fonts_dir().filter(|d| d.is_dir()) {
            lumit_text::fonts::register_dir(&dir);
        }
    });
}

/// Every `.lumt` under `dir`, in any sub-folder but the two the library keeps
/// for itself.
fn files_under(dir: &Path, top: bool, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let kept = top
                && path
                    .file_name()
                    .is_some_and(|n| n == "media" || n == "fonts");
            if !kept {
                files_under(&path, false, found);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(template_file::EXTENSION))
        {
            found.push(path);
        }
    }
}

/// The group a file is in: its folder under the templates folder.
fn group_of(root: &Path, file: &Path) -> String {
    file.parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|rel| {
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

fn listed(root: &Path, file: &Path, info: TemplateInfo) -> BridgeLibraryTemplate {
    let missing_fonts = info
        .fonts
        .iter()
        .filter(|f| f.file.is_none() && !lumit_text::fonts::installed(&f.family, &f.face))
        .map(|f| f.family.clone())
        .collect();
    BridgeLibraryTemplate {
        path: file.to_string_lossy().into_owned(),
        name: info.name,
        group: group_of(root, file),
        id: info.id,
        version: info.version,
        width: info.width,
        height: info.height,
        frames_per_second: info.frame_rate.fps(),
        seconds: info.duration.0.to_f64(),
        controls: info.controls.into_iter().map(|c| c.label).collect(),
        missing_fonts,
    }
}

/// Every template in the library, by name. A file that will not read is left
/// out.
#[frb(sync)]
pub fn list_templates() -> Vec<BridgeLibraryTemplate> {
    register_fonts();
    let Some(root) = folder() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    files_under(&root, true, &mut files);
    let mut list: Vec<BridgeLibraryTemplate> = files
        .iter()
        .filter_map(|file| {
            template_file::read_info(file)
                .ok()
                .map(|info| listed(&root, file, info))
        })
        .collect();
    list.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    list
}

/// The templates folder, for **Show in folder**.
#[frb(sync)]
pub fn templates_folder() -> Option<String> {
    folder().map(|dir| dir.to_string_lossy().into_owned())
}

/// A template's poster, as PNG bytes.
pub fn template_poster(path: String) -> Result<Vec<u8>, BridgeError> {
    template_file::read_poster(Path::new(&path)).map_err(|_| BridgeError::InvalidPreset)
}

/// A template's strip of frames for scrubbing its tile, as JPEG bytes: the
/// frames side by side, each as wide as the strip is tall times the
/// template's own shape. Empty when the file has none.
pub fn template_preview(path: String) -> Result<Vec<u8>, BridgeError> {
    template_file::read_preview(Path::new(&path)).map_err(|_| BridgeError::InvalidPreset)
}

/// A file name made of a template's name: the characters a file system
/// refuses become underscores.
fn file_name_of(name: &str) -> String {
    let plain: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let plain = plain.trim_matches(['.', ' ']).to_owned();
    if plain.is_empty() {
        "template".to_owned()
    } else {
        plain
    }
}

/// Where a template called `name` in `group` is kept.
fn path_for(root: &Path, group: &str, name: &str) -> PathBuf {
    let mut dir = root.to_path_buf();
    for part in group.split('/').filter(|p| !p.is_empty() && *p != "..") {
        dir.push(file_name_of(part));
    }
    dir.join(format!(
        "{}.{}",
        file_name_of(name),
        template_file::EXTENSION
    ))
}

/// The library file a template called `name` in `group` would be saved to,
/// and the id of the template already there, when one is. What the Save
/// dialogue asks before it writes over another template.
#[frb(sync)]
pub fn template_at(name: String, group: String) -> Option<Uuid> {
    let root = folder()?;
    template_file::read_info(&path_for(&root, &group, &name))
        .ok()
        .map(|info| info.id)
}

/// Move a template to another group, answering where it is now. Refused when
/// one of its name is already there.
#[frb(sync)]
pub fn move_template(path: String, group: String) -> Result<String, BridgeError> {
    let root = folder().ok_or(BridgeError::NoProjectPath)?;
    let from = PathBuf::from(&path);
    let name = from
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(BridgeError::InvalidPreset)?;
    let to = path_for(&root, &group, &name);
    if to == from {
        return Ok(path);
    }
    if to.exists() {
        return Err(BridgeError::InvalidPreset);
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|_| BridgeError::WriteFailed)?;
    }
    std::fs::rename(&from, &to).map_err(|_| BridgeError::WriteFailed)?;
    Ok(to.to_string_lossy().into_owned())
}

/// Delete a template from the library. Projects that placed it keep theirs.
#[frb(sync)]
pub fn delete_template(path: String) -> Result<(), BridgeError> {
    let root = folder().ok_or(BridgeError::NoProjectPath)?;
    let file = PathBuf::from(path);
    // Only ever a file of the library's own.
    if !file.starts_with(&root) {
        return Err(BridgeError::InvalidPreset);
    }
    std::fs::remove_file(file).map_err(|_| BridgeError::WriteFailed)
}

/// Rename a template in the library, answering where it is now. Its id stays,
/// so projects that placed it still find it.
pub fn rename_template(path: String, name: String) -> Result<String, BridgeError> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let root = folder().ok_or(BridgeError::NoProjectPath)?;
    let from = PathBuf::from(&path);
    let to = path_for(&root, &group_of(&root, &from), &name);
    if to != from && to.exists() {
        return Err(BridgeError::InvalidPreset);
    }
    template_file::rename(&from, &to, &name).map_err(|_| BridgeError::WriteFailed)?;
    Ok(to.to_string_lossy().into_owned())
}

/// Put a `.lumt` from anywhere into the library, as opening one or dropping
/// one on Lumit does. Answers where it is, or `None` with nothing written
/// when another template of that name is there and `replace` is off. A newer
/// version of the same template takes the older one's place without asking.
pub fn install_template(path: String, replace: bool) -> Result<Option<String>, BridgeError> {
    let root = folder().ok_or(BridgeError::NoProjectPath)?;
    let from = PathBuf::from(&path);
    let info = template_file::read_info(&from).map_err(|_| BridgeError::InvalidPreset)?;
    if from.starts_with(&root) {
        return Ok(Some(path));
    }
    let to = path_for(&root, "", &info.name);
    if let Ok(there) = template_file::read_info(&to) {
        if there.id != info.id && !replace {
            return Ok(None);
        }
    }
    std::fs::copy(&from, &to).map_err(|_| BridgeError::WriteFailed)?;
    Ok(Some(to.to_string_lossy().into_owned()))
}

/// The newest version the library has of the template `id`, with its file.
fn newest(id: Uuid) -> Option<(PathBuf, TemplateInfo)> {
    let root = folder()?;
    let mut files = Vec::new();
    files_under(&root, true, &mut files);
    files
        .into_iter()
        .filter_map(|file| {
            let info = template_file::read_info(&file).ok()?;
            (info.id == id).then_some((file, info))
        })
        .max_by_key(|(_, info)| info.version)
}

/// RGBA bytes as a PNG.
fn png_of(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(bytes)
}

/// The poster and the strip of `comp` in `doc`. Empty on a machine with no
/// graphics adapter, where the file is written without pictures.
fn pictures(doc: &Arc<Document>, comp: Uuid, template: &Template) -> Pictures {
    let mut made = Pictures {
        poster_png: Vec::new(),
        preview_jpg: Vec::new(),
    };
    let Some(c) = doc.comp(comp) else {
        return made;
    };
    let fps = c.frame_rate.fps().max(1.0);
    let frames = ((c.duration.0.to_f64() * fps).round() as u64).max(1);
    let poster = template
        .poster
        .map_or(0, |t| (t.to_f64() * fps).round().max(0.0) as u64)
        .min(frames - 1);
    let scale_to = |width: u32| (f64::from(width) / f64::from(c.width.max(1))).min(1.0) as f32;
    if let Some((rgba, w, h)) = crate::render::thumbnail(doc, comp, poster, scale_to(POSTER_WIDTH))
    {
        made.poster_png = png_of(&rgba, w, h).unwrap_or_default();
    }
    // The strip: frames side by side, opaque over black, as a tile shows it.
    let mut strip: Vec<u8> = Vec::new();
    let mut cell: Option<(u32, u32)> = None;
    let mut cells = 0u32;
    for n in 0..STRIP_FRAMES {
        let frame = (u64::from(n) * frames / u64::from(STRIP_FRAMES)).min(frames - 1);
        let Some((rgba, w, h)) = crate::render::thumbnail(doc, comp, frame, scale_to(STRIP_WIDTH))
        else {
            return made;
        };
        let (cw, ch) = *cell.get_or_insert((w, h));
        if (w, h) != (cw, ch) {
            return made;
        }
        if strip.is_empty() {
            strip = vec![0u8; (cw * STRIP_FRAMES * ch * 3) as usize];
        }
        for y in 0..ch as usize {
            for x in 0..cw as usize {
                let from = (y * cw as usize + x) * 4;
                let to = (y * (cw * STRIP_FRAMES) as usize + (n * cw) as usize + x) * 3;
                let alpha = u32::from(rgba[from + 3]);
                for k in 0..3 {
                    strip[to + k] = ((u32::from(rgba[from + k]) * alpha) / 255) as u8;
                }
            }
        }
        cells += 1;
    }
    if let Some((cw, ch)) = cell.filter(|_| cells == STRIP_FRAMES) {
        let mut bytes = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82);
        if encoder
            .write_image(
                &strip,
                cw * STRIP_FRAMES,
                ch,
                image::ExtendedColorType::Rgb8,
            )
            .is_ok()
        {
            made.preview_jpg = bytes;
        }
    }
    made
}

/// Now, in UTC, as `template.json` writes it.
fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 to a date (Howard Hinnant's civil-from-days).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

impl CompositionReference {
    /// The fonts this composition's text uses, and the text of every
    /// composition it places, for the Save dialogue's **Pack fonts**.
    #[frb(sync)]
    pub fn template_fonts(&self) -> Result<Vec<BridgeFontUse>, BridgeError> {
        let doc = self.document()?;
        Ok(lumit_core::template::fonts::used(&doc, self.id)
            .into_iter()
            .map(|(family, face)| BridgeFontUse {
                installed: lumit_text::fonts::file_of(&family, &face).is_some(),
                family,
                face,
            })
            .collect())
    }

    /// **Save graphics template**: write this composition to the library as
    /// `name` in `group`, with its footage and, with `pack_fonts`, the font
    /// files its text names. Answers the file.
    ///
    /// Saved over the library's own copy of this template it is that
    /// template's next version, which other projects are then offered.
    /// Anywhere else it is a new template, and this composition is tied to
    /// that one from here.
    pub fn save_template(
        &self,
        name: String,
        group: String,
        pack_fonts: bool,
    ) -> Result<String, BridgeError> {
        let doc = self.document()?;
        let beside = {
            let state = self.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state
                .path
                .as_deref()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_default()
        };
        let (path, template) = save_to_library(&doc, self.id, &name, &group, pack_fonts, &beside)?;
        // The project's copy wears the same marks, so it knows which version
        // it is and that nothing has changed since.
        self.commit(Op::SetCompTemplate {
            comp: self.id,
            template: Some(Box::new(template)),
        })?;
        Ok(path.to_string_lossy().into_owned())
    }
}

/// Write composition `comp` of `doc` to the library as `name` in `group`:
/// the file's place, and the template as the project should now hold it,
/// wearing the marks of the save. `beside` is the project's folder, which
/// footage is looked for from.
///
/// What saving from the interface, a script and the command line all do.
#[frb(ignore)]
pub(crate) fn save_to_library(
    doc: &Arc<Document>,
    comp: Uuid,
    name: &str,
    group: &str,
    pack_fonts: bool,
    beside: &Path,
) -> Result<(PathBuf, Template), BridgeError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let root = folder().ok_or(BridgeError::NoProjectPath)?;
    let path = path_for(&root, group, name);
    let held_comp = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
    let mut template = held_comp
        .template()
        .map(|t| (*t).clone())
        .unwrap_or_default();

    // The same template saved again is its next version. Saved under
    // another name, or over another template, it is a new one.
    let there = template_file::read_info(&path).ok();
    let same = template
        .library
        .filter(|id| there.as_ref().is_none_or(|t| t.id == *id))
        .filter(|_| there.is_some() || template.version == 0);
    let (id, version) = match same {
        Some(id) => (
            id,
            newest(id).map_or(template.version, |(_, info)| {
                info.version.max(template.version)
            }) + 1,
        ),
        None => (Uuid::now_v7(), 1),
    };
    template.library = Some(id);
    template.version = version;
    template.saved = lumit_core::template::design_stamp(doc, comp);

    // What the file holds: this composition as it is, wearing the marks of
    // the save, and everything it places.
    let mut stamped = Document::clone(doc);
    if let Some(c) = stamped.comp_mut(comp) {
        c.set_template(Some(&template));
    }
    let held = template_file::document_of(&stamped, comp).map_err(|_| BridgeError::InvalidComp)?;
    let info = template_file::info_of(&held, comp, name, id, version, &now_utc())
        .ok_or(BridgeError::InvalidComp)?;
    let fonts: Vec<FontFile> = if pack_fonts {
        lumit_core::template::fonts::used(&held, comp)
            .into_iter()
            .filter_map(|(family, face)| {
                let path = lumit_text::fonts::file_of(&family, &face)?;
                Some(FontFile { family, face, path })
            })
            .collect()
    } else {
        Vec::new()
    };
    let pictures = pictures(doc, comp, &template);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| BridgeError::WriteFailed)?;
    }
    // The bridge's own list of what to pack, which knows every frame of an
    // image sequence and every file a 3D model reads.
    let sources = crate::packing::sources(&held, beside, true);
    template_file::write_from(
        &path,
        &held,
        &info,
        &pictures,
        &fonts,
        &sources,
        &mut |_, _| {},
    )
    .map_err(|e| BridgeError::ExportFailed(e.to_string()))?;
    Ok((path, template))
}

/// The composition in `doc` that is version `version` of library template
/// `id`, or any version of it with `version` as `None`.
#[frb(ignore)]
pub(crate) fn in_project(doc: &Document, id: Uuid, version: Option<u32>) -> Option<Uuid> {
    doc.items.iter().find_map(|item| match item {
        ProjectItem::Composition(c) => {
            let t = c.template()?;
            (t.library == Some(id) && version.is_none_or(|v| t.version == v)).then_some(c.id)
        }
        _ => None,
    })
}

/// A library file opened for bringing into a project: its document, with its
/// footage read out and its fonts registered, and what it says of itself.
#[frb(ignore)]
pub(crate) fn opened(path: &Path) -> Result<(Document, TemplateInfo), BridgeError> {
    let info = template_file::read_info(path).map_err(|_| BridgeError::InvalidPreset)?;
    let media =
        template_file::media_dir(info.id, info.version).ok_or(BridgeError::NoProjectPath)?;
    std::fs::create_dir_all(&media).map_err(|_| BridgeError::WriteFailed)?;
    let (doc, info) = template_file::open(path, &media).map_err(|_| BridgeError::InvalidPreset)?;
    if let Some(fonts) = template_file::fonts_dir() {
        if std::fs::create_dir_all(&fonts).is_ok() {
            for file in template_file::unpack_fonts(path, &fonts).unwrap_or_default() {
                lumit_text::fonts::register_file(&file);
            }
        }
    }
    Ok((doc, info))
}

impl ProjectReference {
    /// Bring the library template at `path` into this project, answering its
    /// composition. A project that already holds that version of it answers
    /// the one it has. The composition is filed in a folder called `folder`,
    /// made if the project has none of that name.
    ///
    /// Placing it on a timeline is the caller's next step, as placing any
    /// composition is.
    pub fn place_template(
        &self,
        path: String,
        folder: String,
    ) -> Result<CompositionReference, BridgeError> {
        let file = PathBuf::from(path);
        let (from, info) = opened(&file)?;
        let state = self.state()?;
        let (comp, footage) = {
            let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
            let doc = state.store.snapshot();
            if let Some(have) = in_project(&doc, info.id, Some(info.version)) {
                return Ok(CompositionReference::new(self.id, have));
            }
            let (comp, ops, footage) = brought(&doc, &from, &info, &folder)?;
            state
                .store
                .commit(Op::Batch { ops })
                .map_err(BridgeError::OpError)?;
            (comp, footage)
        };
        for file in footage {
            crate::probe::request(file);
        }
        Ok(CompositionReference::new(self.id, comp))
    }

    /// The templates in this project that the library has a newer version of.
    #[frb(sync)]
    pub fn template_updates(&self) -> Result<Vec<BridgeTemplateUpdate>, BridgeError> {
        let state = self.state()?;
        let doc = state
            .read()
            .map_err(|_| BridgeError::ReadFailed)?
            .store
            .snapshot();
        let library = list_templates();
        Ok(doc
            .items
            .iter()
            .filter_map(|item| match item {
                ProjectItem::Composition(c) => {
                    let t = c.template()?;
                    let id = t.library?;
                    let latest = library
                        .iter()
                        .filter(|l| l.id == id)
                        .max_by_key(|l| l.version)?;
                    (latest.version > t.version).then(|| BridgeTemplateUpdate {
                        comp: c.id,
                        name: c.name.clone(),
                        version: t.version,
                        latest: latest.version,
                        path: latest.path.clone(),
                    })
                }
                _ => None,
            })
            .collect())
    }

    /// **Update**: put the library's newest version of a template in the
    /// place of the one this project has. The composition keeps its id, so
    /// every copy still places it, and every copy keeps the values it set, by
    /// control. One undo step.
    pub fn update_template(&self, comp: Uuid, folder: String) -> Result<(), BridgeError> {
        let state = self.state()?;
        let id = {
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            doc.comp(comp)
                .and_then(|c| c.template())
                .and_then(|t| t.library)
                .ok_or(BridgeError::InvalidComp)?
        };
        let (file, _) = newest(id).ok_or(BridgeError::InvalidPreset)?;
        let (from, info) = opened(&file)?;
        let footage = {
            let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
            let doc = state.store.snapshot();
            let index = doc
                .items
                .iter()
                .position(|i| i.id() == comp)
                .ok_or(BridgeError::InvalidComp)?;
            let (new, mut ops, footage) = brought(&doc, &from, &info, &folder)?;
            // The arrival takes the old one's id and its place in the panel,
            // and the old one goes.
            let mut arrival: Option<lumit_core::model::Composition> = None;
            ops.retain(|op| match op {
                Op::AddItem { item, .. } => match &**item {
                    ProjectItem::Composition(c) if c.id == new => {
                        arrival = Some(c.clone());
                        false
                    }
                    _ => true,
                },
                _ => true,
            });
            let mut arrival = arrival.ok_or(BridgeError::InvalidComp)?;
            arrival.id = comp;
            if let Some(old) = doc.comp(comp) {
                arrival.name.clone_from(&old.name);
                // Data linked in this project stays linked.
                arrival.set_template_data(old.template_data().as_ref());
            }
            // Whatever named the arrival by its own id names the old id now.
            for op in &mut ops {
                if let Op::SetFolderChildren { children, .. } = op {
                    children.retain(|c| *c != new);
                }
                if let Op::SetCompTemplate { comp: named, .. } = op {
                    if *named == new {
                        *named = comp;
                    }
                }
            }
            ops.insert(0, Op::RemoveItem { id: comp });
            ops.insert(
                1,
                Op::AddItem {
                    index,
                    item: Box::new(ProjectItem::Composition(arrival)),
                },
            );
            state
                .store
                .commit(Op::Batch { ops })
                .map_err(BridgeError::OpError)?;
            footage
        };
        for file in footage {
            crate::probe::request(file);
        }
        Ok(())
    }
}

/// The ops that bring the template in `from` into `doc`, filed in the folder
/// called `folder`: the new composition's id, the ops, and the footage files
/// to probe.
#[frb(ignore)]
pub(crate) fn brought(
    doc: &Document,
    from: &Document,
    info: &TemplateInfo,
    folder: &str,
) -> Result<(Uuid, Vec<Op>, Vec<PathBuf>), BridgeError> {
    let brought = lumit_project::bring::bring(doc, from, &[info.comp], "")
        .map_err(|_| BridgeError::InvalidPreset)?;
    let comp = *brought.comps.first().ok_or(BridgeError::InvalidPreset)?;
    let mut ops = brought.ops;
    let mut comps = Vec::new();
    let mut solids = Vec::new();
    let mut footage = Vec::new();
    for op in &ops {
        let Op::AddItem { item, .. } = op else {
            continue;
        };
        match &**item {
            ProjectItem::Composition(c) => comps.push(c.id),
            ProjectItem::Solid(s) => solids.push(s.id),
            ProjectItem::Footage(f) => {
                comps.push(f.id);
                footage.push(PathBuf::from(&f.media.absolute_path));
            }
            ProjectItem::Folder(_) => {}
        }
    }
    // The template and what it places go in the templates folder, and its
    // solids with the project's other solids.
    let (home, make): (Uuid, Vec<Op>) =
        match lumit_core::template::author::library_folder(doc, folder) {
            Some(id) => (id, Vec::new()),
            None => {
                let id = Uuid::now_v7();
                (
                    id,
                    vec![Op::AddItem {
                        index: doc.items.len() + ops.len(),
                        item: Box::new(ProjectItem::Folder(lumit_core::model::Folder {
                            id,
                            name: folder.to_owned(),
                            children: Vec::new(),
                            extra: serde_json::Map::new(),
                        })),
                    }],
                )
            }
        };
    ops.extend(make);
    let mut children = doc
        .folder(home)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.extend(comps);
    ops.push(Op::SetFolderChildren {
        folder: home,
        children,
    });
    if !solids.is_empty() {
        let (solid_home, make) =
            crate::edits::ensure_auto_folder_ops(doc, lumit_core::ops::AutoFolderKind::Solids);
        ops.extend(make);
        let mut children = doc
            .folder(solid_home)
            .map(|f| f.children.clone())
            .unwrap_or_default();
        children.extend(solids);
        ops.push(Op::SetFolderChildren {
            folder: solid_home,
            children,
        });
    }
    // The ids are new here, so the design's stamp is taken again: the copy
    // this project holds is the library's, unchanged.
    let mut scratch = doc.clone();
    for op in &ops {
        // A scratch copy to read the stamp from. An op that will not apply
        // here will not apply for real either, and the commit says so.
        let _ = lumit_core::ops::apply(&mut scratch, op);
    }
    if let Some(mut template) = scratch
        .comp(comp)
        .and_then(|c| c.template())
        .map(|t| (*t).clone())
    {
        template.library = Some(info.id);
        template.version = info.version;
        template.saved = lumit_core::template::design_stamp(&scratch, comp);
        ops.push(Op::SetCompTemplate {
            comp,
            template: Some(Box::new(template)),
        });
    }
    Ok((comp, ops, footage))
}
