//! The `.lumt` graphics template file, and the library folder they are kept
//! in.
//!
//! A `.lumt` is the same kind of archive a packed `.lum` is, with a manifest
//! that says [`FORMAT`]. Its `project.json` is a whole document holding the
//! template's composition and everything it places, and nothing else of the
//! project it came from. The footage sits under `media/` as a packed project
//! carries it. Beside those are `template.json`, which says enough to list
//! the template without reading the project, the fonts under `fonts/`, a
//! poster frame and a strip of frames to scrub.
//!
//! A template is a file from somebody else. Every entry is read up to a
//! limit, and nothing is written outside the folder it is read out to.
//!
//! Runs on whichever thread saves or opens the template, never the UI thread.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use lumit_core::model::{Document, PackedMedia, ProjectItem};
use lumit_core::template::ControlKind;
use lumit_core::time::{Duration, FrameRate, Rational};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::pack::{self, PackSource};
use crate::{Manifest, ProjectError};

/// What a template file's name ends in.
pub const EXTENSION: &str = "lumt";
/// What a template file's manifest says it is.
pub const FORMAT: &str = "lumit-template";

const INFO_ENTRY: &str = "template.json";
const POSTER_ENTRY: &str = "poster.png";
const PREVIEW_ENTRY: &str = "preview.jpg";
const FONTS_PREFIX: &str = "fonts/";

/// The most `template.json` may decompress to. It is a few lines for each
/// control.
const INFO_MAX_BYTES: u64 = 4 << 20;
/// The most the poster or the strip of frames may decompress to.
const PICTURE_MAX_BYTES: u64 = 64 << 20;
/// The most one packed font may decompress to. The largest families that
/// ship as one file run to a hundred megabytes or so.
const FONT_MAX_BYTES: u64 = 256 << 20;
/// The most fonts a template carries.
const MAX_FONTS: usize = 256;

/// Where a library template's footage is read out to, once per version:
/// `templates/media/<id>/<version>/`.
#[must_use]
pub fn media_dir(id: Uuid, version: u32) -> Option<PathBuf> {
    let media = crate::templates_dir()?.join("media");
    Some(media.join(id.to_string()).join(version.to_string()))
}

/// Where fonts that came inside templates are kept: `templates/fonts/`.
#[must_use]
pub fn fonts_dir() -> Option<PathBuf> {
    Some(crate::templates_dir()?.join("fonts"))
}

/// What `template.json` holds: enough to list, search and sync a template
/// without reading its project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateInfo {
    /// The library's name for the template, the same through every version.
    pub id: Uuid,
    /// Which save this is, counting up from 1.
    pub version: u32,
    /// When it was saved: UTC, RFC 3339.
    pub saved: String,
    /// What the library shows it as.
    pub name: String,
    /// The template's composition inside `project.json`.
    pub comp: Uuid,
    /// The composition's width in pixels.
    pub width: u32,
    /// The composition's height in pixels.
    pub height: u32,
    /// The composition's frame rate.
    pub frame_rate: FrameRate,
    /// How long the composition is.
    pub duration: Duration,
    /// The frame the library shows for it. `None` is the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<Rational>,
    /// The moment the intro ends. `None` is no intro.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<Rational>,
    /// The moment the outro starts. `None` is no outro.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outro: Option<Rational>,
    /// The controls, in the order a copy shows them.
    #[serde(default)]
    pub controls: Vec<ControlInfo>,
    /// The fonts it draws with.
    #[serde(default)]
    pub fonts: Vec<FontUse>,
    /// The archive entries of the footage it carries. [`write`] fills this in.
    #[serde(default)]
    pub footage: Vec<String>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One control, as a listing shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlInfo {
    /// What a copy's value is kept under.
    pub id: String,
    /// What a copy shows it as.
    pub label: String,
    /// What it is drawn as on a copy.
    pub kind: ControlKind,
    /// The folding heading it sits under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// A menu's options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One font a template draws with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontUse {
    /// The family.
    pub family: String,
    /// The face inside the family. Empty is the family's regular.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub face: String,
    /// The archive entry under `fonts/` the font is packed as, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The pictures a template file carries, as the caller made them.
#[derive(Debug, Clone, Default)]
pub struct Pictures {
    /// The poster frame, as a PNG. Empty leaves it out.
    pub poster_png: Vec<u8>,
    /// A strip of frames for scrubbing, as a JPEG. Empty leaves it out.
    pub preview_jpg: Vec<u8>,
}

/// One font file handed over for packing.
#[derive(Debug, Clone)]
pub struct FontFile {
    /// The family it is named by.
    pub family: String,
    /// The face inside the family.
    pub face: String,
    /// The file on disk.
    pub path: PathBuf,
}

/// The document a template file holds: `comp` and everything it places, with
/// the ids it has in `doc`.
///
/// The settings that change the picture come along: anti-aliasing, the
/// colour depth, colour management and the node graph's room. Nothing else of
/// the project does, and the items sit at the root. Data linked to a template
/// is somebody's table and stays behind.
pub fn document_of(doc: &Document, comp: Uuid) -> Result<Document, ProjectError> {
    let needed = crate::bring::needed(doc, &[comp])?;
    let mut held = Document::new();
    held.anti_aliasing = doc.anti_aliasing;
    held.colour_depth = doc.colour_depth;
    held.graph_room = doc.graph_room;
    held.colour = doc.colour.clone();
    held.items = doc
        .items
        .iter()
        .filter(|item| needed.contains_key(&item.id().to_string()))
        .cloned()
        .collect();
    for item in &mut held.items {
        if let ProjectItem::Composition(c) = item {
            c.set_template_data(None);
        }
    }
    Ok(held)
}

/// What `template.json` says of `comp` in `held` (a document made by
/// [`document_of`]). `None` when `held` has no such composition.
///
/// No font has a file and no footage is listed yet: [`write`] records both as
/// it packs them.
#[must_use]
pub fn info_of(
    held: &Document,
    comp: Uuid,
    name: &str,
    id: Uuid,
    version: u32,
    saved: &str,
) -> Option<TemplateInfo> {
    let c = held.comp(comp)?;
    let template = c.template();
    let template = template.as_deref();
    let controls = template.map(|t| t.controls.as_slice()).unwrap_or_default();
    Some(TemplateInfo {
        id,
        version,
        saved: saved.to_owned(),
        name: name.to_owned(),
        comp,
        width: c.width,
        height: c.height,
        frame_rate: c.frame_rate,
        duration: c.duration,
        poster: template.and_then(|t| t.poster),
        intro: template.and_then(|t| t.intro),
        outro: template.and_then(|t| t.outro),
        controls: controls
            .iter()
            .map(|control| ControlInfo {
                id: control.id.clone(),
                label: control.label.clone(),
                kind: control.kind,
                group: control.group.clone(),
                options: control.options.clone(),
                extra: serde_json::Map::new(),
            })
            .collect(),
        fonts: lumit_core::template::fonts::used(held, comp)
            .into_iter()
            .map(|(family, face)| FontUse {
                family,
                face,
                file: None,
                extra: serde_json::Map::new(),
            })
            .collect(),
        footage: Vec::new(),
        extra: serde_json::Map::new(),
    })
}

/// The files of `held` that are on disk, offered for packing: each footage
/// item's, and each file an effect reads.
// ponytail: one file an item. An image sequence packs its first frame, a 3D
// model goes without the files it pulls in, and the colour config stays
// behind. The bridge knows the whole of each, so hand its pack sources to
// `write_from` when a template with one of those has to travel.
fn sources(held: &Document) -> Vec<PackSource> {
    let footage = held.items.iter().filter_map(|item| match item {
        ProjectItem::Footage(f) => Some((f.id, f.media.on_disk())),
        _ => None,
    });
    footage
        .chain(held.effect_files())
        .filter(|(_, path)| Path::new(path).is_file())
        .map(|(item, path)| PackSource {
            item,
            files: vec![PathBuf::from(path)],
        })
        .collect()
}

/// Write the file, whole or not at all. Packs every footage file `held`
/// names that is on disk. A file that is missing is left out, and its item
/// keeps its path.
///
/// `info` is written with what the archive ended up holding: each font
/// handed over in `fonts` gets its entry, and `footage` lists the media
/// entries. `progress` hears the bytes of footage copied and the bytes there
/// are.
pub fn write(
    path: &Path,
    held: &Document,
    info: &TemplateInfo,
    pictures: &Pictures,
    fonts: &[FontFile],
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), ProjectError> {
    write_from(path, held, info, pictures, fonts, &sources(held), progress)
}

/// [`write`], packing the files `sources` offers and no others.
pub fn write_from(
    path: &Path,
    held: &Document,
    info: &TemplateInfo,
    pictures: &Pictures,
    fonts: &[FontFile],
    sources: &[PackSource],
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), ProjectError> {
    // Where a packed file was on the machine that made the template means
    // nothing anywhere else, so each one is written by its name alone.
    let mut written = held.clone();
    let offered: HashSet<Uuid> = sources.iter().map(|s| s.item).collect();
    for item in &mut written.items {
        let ProjectItem::Footage(f) = item else {
            continue;
        };
        let name = crate::file_name_of(f.media.on_disk()).map(str::to_owned);
        if let Some(name) = name.filter(|_| offered.contains(&f.id)) {
            f.media.relative_path = name;
        }
    }
    for file in written.effect_file_paths_mut() {
        let name = crate::file_name_of(file).map(str::to_owned);
        if let Some(name) = name.filter(|_| Path::new(file).is_file()) {
            *file = name;
        }
    }

    let mut info = info.clone();
    let manifest = Manifest {
        format: FORMAT.into(),
        ..Manifest::current()
    };
    let mut more = |zip: &mut ZipWriter<File>,
                    packed: &BTreeMap<Uuid, PackedMedia>|
     -> Result<(), ProjectError> {
        let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        // Several faces can sit in one file, which is packed once.
        let mut names: HashSet<String> = HashSet::new();
        let mut entries: HashMap<&Path, Option<String>> = HashMap::new();
        for font in fonts.iter().take(MAX_FONTS) {
            let entry = match entries.get(font.path.as_path()) {
                Some(entry) => entry.clone(),
                None => {
                    let entry = pack_font(zip, &font.path, &mut names, text)?;
                    entries.insert(&font.path, entry.clone());
                    entry
                }
            };
            let Some(entry) = entry else {
                continue;
            };
            let listed = info
                .fonts
                .iter_mut()
                .find(|used| used.family == font.family && used.face == font.face);
            match listed {
                Some(used) => used.file = Some(entry),
                None => info.fonts.push(FontUse {
                    family: font.family.clone(),
                    face: font.face.clone(),
                    file: Some(entry),
                    extra: serde_json::Map::new(),
                }),
            }
        }

        let footage: BTreeSet<&str> = packed.values().map(|m| m.entry.as_str()).collect();
        info.footage = footage.into_iter().map(str::to_owned).collect();
        zip.start_file(INFO_ENTRY, text)?;
        zip.write_all(serde_json::to_string_pretty(&info)?.as_bytes())?;

        // Compressed already, as footage is.
        for (entry, bytes) in [
            (POSTER_ENTRY, &pictures.poster_png),
            (PREVIEW_ENTRY, &pictures.preview_jpg),
        ] {
            if !bytes.is_empty() {
                zip.start_file(entry, stored)?;
                zip.write_all(bytes)?;
            }
        }
        Ok(())
    };
    pack::save_packed_with(
        &written,
        path,
        None,
        sources,
        &manifest,
        &mut more,
        &mut |done, total| {
            progress(done, total);
            true
        },
    )?;
    Ok(())
}

/// Whether a file of this name is a font Lumit reads.
fn is_font(name: &str) -> bool {
    let extension = Path::new(name).extension().and_then(|e| e.to_str());
    extension.is_some_and(|e| {
        ["ttf", "otf", "ttc"]
            .iter()
            .any(|font| e.eq_ignore_ascii_case(font))
    })
}

/// Pack the font file at `path` under `fonts/`, answering its entry. A file
/// that is not a font, is too big to be read back or will not read is left
/// out, and the words it sets draw in a stand-in where it is not installed.
fn pack_font(
    zip: &mut ZipWriter<File>,
    path: &Path,
    names: &mut HashSet<String>,
    options: SimpleFileOptions,
) -> Result<Option<String>, ProjectError> {
    let name = path.file_name().map(|n| n.to_string_lossy());
    let name: String = name
        .unwrap_or_default()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let size = fs::metadata(path).map(|meta| meta.len()).ok();
    if !pack::plain_name(&name) || !is_font(&name) || size.is_none_or(|s| s > FONT_MAX_BYTES) {
        return Ok(None);
    }
    let Ok(bytes) = fs::read(path) else {
        return Ok(None);
    };
    let entry = format!("{FONTS_PREFIX}{}", crate::unique_name(&name, names));
    zip.start_file(entry.as_str(), options)?;
    zip.write_all(&bytes)?;
    Ok(Some(entry))
}

/// The archive at `path`, once its manifest has said it is a template this
/// Lumit can read.
fn archive(path: &Path) -> Result<ZipArchive<lumit_ingress::files::Reader>, ProjectError> {
    let mut zip = ZipArchive::new(lumit_ingress::files::open(path)?)?;
    crate::read_manifest(&mut zip, FORMAT)?;
    Ok(zip)
}

fn info_in<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<TemplateInfo, ProjectError> {
    let entry = zip
        .by_name(INFO_ENTRY)
        .map_err(|_| ProjectError::NotALumitProject)?;
    Ok(serde_json::from_str(&crate::entry_text(
        entry,
        INFO_MAX_BYTES,
    )?)?)
}

/// The bytes of the entry called `name`, refusing one that decompresses past
/// `limit`. Empty when the archive has no such entry.
fn entry_bytes<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, ProjectError> {
    let Some(index) = zip.index_for_name(name) else {
        return Ok(Vec::new());
    };
    let entry = zip.by_index(index)?;
    let size = entry.size();
    if size > limit {
        return Err(ProjectError::EntryTooLarge { size, limit });
    }
    // One byte past the ceiling, so an entry that lied about its size is
    // caught by the byte that proves it.
    let mut bytes = Vec::new();
    entry
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(ProjectError::EntryTooLarge {
            size: limit.saturating_add(1),
            limit,
        });
    }
    Ok(bytes)
}

/// Give the template in the file at `from` the name `name`, written to `to`,
/// which may be `from` itself. Everything else in the file is carried over
/// as it is, so its id and its version stay and a project that placed it
/// still finds it. `from` is gone afterwards when `to` is another file.
pub fn rename(from: &Path, to: &Path, name: &str) -> Result<(), ProjectError> {
    let mut zip = archive(from)?;
    let mut info = info_in(&mut zip)?;
    info.name = name.to_owned();
    let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // Beside where it is going, so the last step is a rename on one disk.
    let mut part = to.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let written = (|| -> Result<(), ProjectError> {
        let mut out = ZipWriter::new(File::create(&part)?);
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index)?;
            if entry.name().is_ok_and(|name| name == INFO_ENTRY) {
                drop(entry);
                out.start_file(INFO_ENTRY, text)?;
                out.write_all(&serde_json::to_vec_pretty(&info)?)?;
            } else {
                out.raw_copy_file(entry)?;
            }
        }
        out.finish()?.sync_all()?;
        Ok(())
    })();
    drop(zip);
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    fs::rename(&part, to)?;
    if from != to {
        fs::remove_file(from)?;
    }
    Ok(())
}

/// The manifest and `template.json` only. Cheap: what listing the library
/// reads for every file.
pub fn read_info(path: &Path) -> Result<TemplateInfo, ProjectError> {
    info_in(&mut archive(path)?)
}

/// The poster frame, as a PNG. Empty when the file has none.
pub fn read_poster(path: &Path) -> Result<Vec<u8>, ProjectError> {
    entry_bytes(&mut archive(path)?, POSTER_ENTRY, PICTURE_MAX_BYTES)
}

/// The strip of frames for scrubbing, as a JPEG. Empty when the file has
/// none.
pub fn read_preview(path: &Path) -> Result<Vec<u8>, ProjectError> {
    entry_bytes(&mut archive(path)?, PREVIEW_ENTRY, PICTURE_MAX_BYTES)
}

/// The whole template: its document, with every footage item pointing at a
/// file read out under `media_root`, and its info. A file already there at
/// the same size is not read out again.
///
/// The document goes through the migrations a project's does. An item whose
/// file the archive does not hold keeps its path and shows as missing.
pub fn open(path: &Path, media_root: &Path) -> Result<(Document, TemplateInfo), ProjectError> {
    let (mut doc, info) = {
        let mut zip = ZipArchive::new(lumit_ingress::files::open(path)?)?;
        let manifest = crate::read_manifest(&mut zip, FORMAT)?;
        let info = info_in(&mut zip)?;
        (crate::read_project(&mut zip, &manifest)?, info)
    };
    if doc.comp(info.comp).is_none() {
        return Err(ProjectError::UnknownComposition);
    }
    let archives = [path.to_path_buf()];
    pack::restore_packed(&mut doc, &archives, media_root, media_root, &mut |_, _| {
        true
    })?;
    // What was packed is a file under `media_root` now, and the document is
    // nobody's packed project.
    doc.packed.clear();
    Ok((doc, info))
}

/// Read the packed fonts out into `dest`, answering where each one is. One
/// already there at the same size is not written again.
///
/// A font never replaces a file of another size: it takes the next free
/// numbered name, so a font a running Lumit is drawing with is left alone.
pub fn unpack_fonts(path: &Path, dest: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    let mut zip = archive(path)?;
    let mut out: Vec<PathBuf> = Vec::new();
    for index in 0..zip.len() {
        if out.len() >= MAX_FONTS {
            break;
        }
        let entry = zip.by_index(index)?;
        // A plain name and no folders: an archive is somebody else's file,
        // and the name is where the font is written.
        let Some(name) = entry.name().ok().and_then(|name| {
            let name = name.strip_prefix(FONTS_PREFIX)?;
            (pack::plain_name(name) && is_font(name)).then(|| name.to_owned())
        }) else {
            continue;
        };
        let size = entry.size();
        if size > FONT_MAX_BYTES {
            return Err(ProjectError::EntryTooLarge {
                size,
                limit: FONT_MAX_BYTES,
            });
        }
        let Some((place, there)) = font_place(dest, &name, size) else {
            continue;
        };
        if !there {
            // Written under another name and renamed, so a copy cut short is
            // never taken for the font.
            let part = place.with_file_name(format!(".{name}.part"));
            let copied = (|| -> Result<(), ProjectError> {
                fs::create_dir_all(dest)?;
                let mut to = File::create(&part)?;
                // One byte past what the entry says it holds, so one that
                // holds more is caught and not written.
                let read = std::io::copy(&mut entry.take(size.saturating_add(1)), &mut to)?;
                if read != size {
                    return Err(ProjectError::EntryTooLarge {
                        size: read,
                        limit: size,
                    });
                }
                to.sync_all()?;
                Ok(())
            })();
            if let Err(e) = copied {
                let _ = fs::remove_file(&part);
                return Err(e);
            }
            fs::rename(&part, &place)?;
        }
        if !out.contains(&place) {
            out.push(place);
        }
    }
    Ok(out)
}

/// Where in `dest` a font called `name` and `size` bytes long goes, and
/// whether it is there already: its own name when that is free or holds a
/// file of that size, otherwise the first numbered name that is or does.
fn font_place(dest: &Path, name: &str, size: u64) -> Option<(PathBuf, bool)> {
    let path = Path::new(name);
    let stem = path.file_stem()?.to_string_lossy();
    let extension = path.extension()?.to_string_lossy();
    (0..MAX_FONTS).find_map(|n| {
        let place = match n {
            0 => dest.join(name),
            n => dest.join(format!("{stem}-{n}.{extension}")),
        };
        match fs::metadata(&place) {
            Ok(meta) if meta.is_file() && meta.len() == size => Some((place, true)),
            Ok(_) => None,
            Err(_) => Some((place, false)),
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fixtures::{stress_document, StressParams};
    use lumit_core::model::{LayerKind, LinearColour, SolidDef};
    use lumit_core::template::feed::Data;
    use lumit_core::template::{Control, Target, Template};

    /// A project with a template in it: a composition placing a nested one
    /// and a solid, the nested one showing a file that is on disk and one
    /// that is not. A third composition and the rest of the footage are in
    /// the project and in nothing the template places.
    fn project(dir: &Path) -> (Document, Uuid, Vec<Uuid>) {
        let mut doc = stress_document(&StressParams::TINY);
        let ids = |comps: bool| -> Vec<Uuid> {
            let wanted = |item: &&ProjectItem| match item {
                ProjectItem::Composition(_) => comps,
                ProjectItem::Footage(_) => !comps,
                _ => false,
            };
            doc.items.iter().filter(wanted).map(|i| i.id()).collect()
        };
        let (comps, footage) = (ids(true), ids(false));
        let (plate, gone) = (footage[0], footage[1]);
        let on_disk = dir.join("plate.png");
        fs::write(&on_disk, b"a plate's worth of bytes").unwrap();
        for item in &mut doc.items {
            if let ProjectItem::Footage(f) = item {
                if f.id == plate {
                    f.media.absolute_path = on_disk.to_string_lossy().into_owned();
                } else if f.id == gone {
                    f.media.absolute_path = dir.join("gone.mov").to_string_lossy().into_owned();
                }
            }
        }
        let solid = SolidDef {
            id: Uuid::now_v7(),
            name: "Grey solid".into(),
            colour: LinearColour::BLACK,
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        };

        let nested = doc.comp_mut(comps[1]).unwrap();
        nested.layers.truncate(2);
        nested.layers[0].kind = LayerKind::Footage { item: plate };
        nested.layers[1].kind = LayerKind::Footage { item: gone };

        let titles = doc.comp_mut(comps[0]).unwrap();
        titles.layers.truncate(2);
        titles.layers[0].kind = LayerKind::Precomp { comp: comps[1] };
        titles.layers[1].kind = LayerKind::Solid { def: solid.id };
        let shown = Target::Visible {
            layer: titles.layers[1].id,
        };
        titles.set_template(Some(&Template {
            controls: vec![Control::new(
                "backing".into(),
                "Backing".into(),
                ControlKind::Checkbox,
                shown,
            )],
            ..Template::default()
        }));
        titles.set_template_data(Some(&Data::default()));

        doc.items.push(ProjectItem::Solid(solid.clone()));
        (doc, comps[0], vec![comps[1], solid.id, plate, gone])
    }

    #[test]
    fn a_template_written_and_read_back_is_the_same_template() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, comp, placed) = project(dir.path());
        let (plate, gone) = (placed[2], placed[3]);

        // The composition and what it places, and nothing else.
        let held = document_of(&doc, comp).unwrap();
        let mut kept: Vec<Uuid> = held.items.iter().map(ProjectItem::id).collect();
        let mut wanted: Vec<Uuid> = placed.iter().copied().chain([comp]).collect();
        kept.sort();
        wanted.sort();
        assert_eq!(kept, wanted);
        let template = held.comp(comp).unwrap();
        assert!(template.is_template() && template.template_data().is_none());

        let font = dir.path().join("Brand-Bold.otf");
        fs::write(&font, b"not really a font").unwrap();
        let fonts = [FontFile {
            family: "Brand".into(),
            face: "Bold".into(),
            path: font,
        }];
        let pictures = Pictures {
            poster_png: b"a poster".to_vec(),
            preview_jpg: Vec::new(),
        };
        let id = Uuid::now_v7();
        let info = info_of(&held, comp, "Lower third", id, 3, "2026-11-11T14:03:22Z").unwrap();
        let path = dir.path().join("lower-third.lumt");
        write(&path, &held, &info, &pictures, &fonts, &mut |_, _| {}).unwrap();

        // The listing reads what was written.
        let listed = read_info(&path).unwrap();
        assert_eq!((listed.id, listed.version, listed.comp), (id, 3, comp));
        assert_eq!(listed.name, "Lower third");
        assert_eq!(listed.controls.len(), 1);
        assert_eq!(listed.controls[0].kind, ControlKind::Checkbox);
        assert_eq!(listed.footage.len(), 1);
        assert_eq!(
            listed.fonts[0].file.as_deref(),
            Some("fonts/Brand-Bold.otf")
        );
        assert_eq!(read_poster(&path).unwrap(), b"a poster");
        assert!(read_preview(&path).unwrap().is_empty());

        // Opened, it is the same composition with its template, and the
        // footage that was on disk is a file under the folder given.
        let media = dir.path().join("media");
        let (opened, info) = open(&path, &media).unwrap();
        assert_eq!(info, listed);
        assert_eq!(opened.comp(comp), held.comp(comp));
        assert_eq!(opened.items.len(), held.items.len());
        let file_of = |doc: &Document, id: Uuid| match doc.item(id) {
            Some(ProjectItem::Footage(f)) => PathBuf::from(f.media.on_disk()),
            _ => panic!("no such footage"),
        };
        let unpacked = file_of(&opened, plate);
        assert!(unpacked.starts_with(&media));
        assert_eq!(fs::read(&unpacked).unwrap(), b"a plate's worth of bytes");
        assert!(!file_of(&opened, gone).is_file());
        assert!(opened.packed.is_empty());

        // The font comes out once, and a second time finds it there.
        let font_dir = dir.path().join("fonts");
        let out = unpack_fonts(&path, &font_dir).unwrap();
        assert_eq!(out, vec![font_dir.join("Brand-Bold.otf")]);
        assert_eq!(fs::read(&out[0]).unwrap(), b"not really a font");
        assert_eq!(unpack_fonts(&path, &font_dir).unwrap(), out);

        // A project is not a template.
        let project = dir.path().join("project.lum");
        crate::save(&held, &project).unwrap();
        assert!(matches!(
            open(&project, &media),
            Err(ProjectError::NotALumitProject)
        ));
        assert!(matches!(
            read_info(&project),
            Err(ProjectError::NotALumitProject)
        ));

        // Nor is a template that asks for a newer reader one this Lumit opens.
        let newer = dir.path().join("newer.lumt");
        let mut zip = ZipWriter::new(File::create(&newer).unwrap());
        let manifest = Manifest {
            format: FORMAT.into(),
            min_reader: "99.0.0".into(),
            ..Manifest::current()
        };
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();
        zip.finish().unwrap();
        assert!(matches!(
            read_info(&newer),
            Err(ProjectError::TooNew { .. })
        ));
        assert!(matches!(
            open(&newer, &media),
            Err(ProjectError::TooNew { .. })
        ));
    }
}
