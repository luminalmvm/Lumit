//! Finding the fonts the system has, and loading the one a layer names.
//!
//! A layer names its font by family and face, the two strings the Text panel
//! shows. A font that isn't installed here falls back to the built-in Inter, so
//! a project opened on another machine still draws its words. Inter's Bold and
//! Black are built in too, as the family "Inter", for when the system has none.
//!
//! A font file can also be handed over ([`register_file`]), which is how the
//! fonts packed in a graphics template draw without being installed. Those are
//! kept apart from the system's and looked in after them, so a packed file
//! never stands in front of an installed family.
//!
//! The fonts in the user's library are drawn with beside the system's. Their
//! files are read into memory, never mapped, so the library can replace or
//! delete one while Lumit runs. A library family hides an installed family of
//! the same name.
//!
//! ponytail: a missing glyph draws the font's own empty box, there is no
//! fallback to a second font for a script the first doesn't cover.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};

type Bytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// One face, loaded and ready to shape and draw.
#[derive(Clone)]
pub(crate) struct Face {
    bytes: Bytes,
    index: u32,
    /// The font at its default place, which every other place is built from.
    base: harfrust::Font,
    /// The named place the face is, for a face of a variable font.
    instance: Option<usize>,
    /// Where the face sits on each of the font's axes, in the font's own
    /// numbers: its named place, moved by the layer's own axes.
    place: Vec<([u8; 4], f32)>,
    /// The font where this face sits in a variable font's design space, which
    /// is what makes Bahnschrift's Bold a bold.
    pub(crate) shaper: harfrust::Font,
}

impl Face {
    fn new(bytes: Bytes, index: u32, instance: Option<usize>) -> Option<Self> {
        let font = harfrust::Font::new(bytes.clone(), index)?;
        let mut face = Self {
            bytes,
            index,
            shaper: font.clone(),
            base: font,
            instance,
            place: Vec::new(),
        };
        face.place = face.named_place();
        face.shaper = face.varied(&[]);
        Some(face)
    }

    /// The same font as the outlines and metrics read it.
    pub(crate) fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index((*self.bytes).as_ref(), self.index).ok()
    }

    /// The face's named place on every axis the font has.
    fn named_place(&self) -> Vec<([u8; 4], f32)> {
        let Some(font) = self.font() else {
            return Vec::new();
        };
        let named = self
            .instance
            .and_then(|i| font.named_instances().get(i))
            .map(|n| n.user_coords().collect::<Vec<f32>>());
        font.axes()
            .iter()
            .enumerate()
            .map(|(i, axis)| {
                let value = named
                    .as_ref()
                    .and_then(|n| n.get(i).copied())
                    .unwrap_or_else(|| axis.default_value());
                (axis.tag().to_be_bytes(), value)
            })
            .collect()
    }

    /// The font at this face's place with `moves` on top, clamped to each
    /// axis's own range by the font. An axis the font hasn't got is ignored.
    pub(crate) fn varied(&self, moves: &[([u8; 4], f32)]) -> harfrust::Font {
        let place: Vec<harfrust::font::Variation> = self
            .place
            .iter()
            .map(|(tag, v)| {
                let v = moves
                    .iter()
                    .rev()
                    .find(|(t, _)| t == tag)
                    .map_or(*v, |(_, m)| *m);
                (harfrust::Tag::new(tag), v).into()
            })
            .collect();
        if place.is_empty() {
            return self.base.clone();
        }
        self.base.instance_builder().variations(place).build()
    }

    /// This face with the layer's own axes set, which is the place every
    /// letter starts from.
    pub(crate) fn with_axes(&self, axes: &[lumit_core::text::FontAxisValue]) -> Face {
        #[allow(clippy::cast_possible_truncation)] // a place on an axis
        let moves: Vec<([u8; 4], f32)> = axes
            .iter()
            .filter(|a| a.value.is_finite())
            .filter_map(|a| Some((lumit_core::text::axis_tag(&a.tag)?, a.value as f32)))
            .collect();
        if moves.is_empty() || self.place.is_empty() {
            return self.clone();
        }
        let ranges = self.ranges();
        let place = self
            .place
            .iter()
            .map(|(tag, v)| {
                let v = moves
                    .iter()
                    .rev()
                    .find(|(t, _)| t == tag)
                    .map_or(*v, |(_, m)| *m);
                let v = ranges
                    .iter()
                    .find(|(t, ..)| t == tag)
                    .map_or(v, |(_, lo, hi)| v.clamp(*lo, *hi));
                (*tag, v)
            })
            .collect();
        let mut face = Face {
            place,
            ..self.clone()
        };
        face.shaper = face.varied(&[]);
        face
    }

    /// Where the face sits on `tag`, or `None` for an axis the font hasn't got.
    pub(crate) fn place_on(&self, tag: [u8; 4]) -> Option<f32> {
        self.place.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
    }

    /// Each axis's tag and range.
    fn ranges(&self) -> Vec<([u8; 4], f32, f32)> {
        self.font().map_or_else(Vec::new, |font| {
            font.axes()
                .iter()
                .map(|a| (a.tag().to_be_bytes(), a.min_value(), a.max_value()))
                .collect()
        })
    }
}

/// One axis of a variable font, as the Text panel and an animator show it.
#[derive(Debug, Clone, PartialEq)]
pub struct FontAxis {
    /// The four-letter tag, such as `wght`.
    pub tag: String,
    /// What the font calls it, such as "Weight".
    pub name: String,
    pub min: f64,
    pub max: f64,
    /// The font's own default place on it.
    pub default: f64,
    /// Where the face sits on it, which is what an unset axis reads as.
    pub face: f64,
    /// An axis the font asks not to be shown, kept for the animators.
    pub hidden: bool,
}

/// An axis tag as text, spaces at its end trimmed.
fn tag_text(tag: [u8; 4]) -> String {
    String::from_utf8_lossy(&tag).trim_end().to_owned()
}

/// The axes of the face a layer names: empty for a font that doesn't vary.
#[must_use]
pub fn axes(family: &str, face_name: &str) -> Vec<FontAxis> {
    let face = face(family, face_name);
    let Some(font) = face.font() else {
        return Vec::new();
    };
    font.axes()
        .iter()
        .map(|axis| {
            let tag = axis.tag().to_be_bytes();
            let name = name(&font, axis.name_id()).unwrap_or_else(|| registered_name(tag));
            FontAxis {
                tag: tag_text(tag),
                name,
                min: f64::from(axis.min_value()),
                max: f64::from(axis.max_value()),
                default: f64::from(axis.default_value()),
                face: f64::from(face.place_on(tag).unwrap_or_else(|| axis.default_value())),
                hidden: axis.is_hidden(),
            }
        })
        .collect()
}

/// What OpenType calls a registered axis, for a font that doesn't name it.
fn registered_name(tag: [u8; 4]) -> String {
    match &tag {
        b"wght" => "Weight".to_owned(),
        b"wdth" => "Width".to_owned(),
        b"slnt" => "Slant".to_owned(),
        b"ital" => "Italic".to_owned(),
        b"opsz" => "Optical size".to_owned(),
        _ => tag_text(tag),
    }
}

/// Whether the family has a font with axes of its own to move. Read from
/// the lists the system and the registered files keep, so no font file is
/// opened.
#[must_use]
pub fn is_variable(family: &str) -> bool {
    fonts_named(family).iter().any(|f| !f.axes().is_empty())
}

/// The installed families that vary, for the Text panel's filter.
#[must_use]
pub fn variable_families() -> Vec<String> {
    families().into_iter().filter(|f| is_variable(f)).collect()
}

/// The built-in face, the one every layer drew with before there was a choice.
pub(crate) fn builtin() -> Arc<Face> {
    static INTER: OnceLock<Arc<Face>> = OnceLock::new();
    INTER
        .get_or_init(|| {
            let bytes: Bytes = Arc::new(crate::INTER);
            #[allow(clippy::expect_used)] // compile-time asset; failure = broken build
            Arc::new(Face::new(bytes, 0, None).expect("embedded Inter font parses"))
        })
        .clone()
}

/// The built-in family, and the faces of it carried in the program.
const INTER: &str = "Inter";
const BUNDLED: [(&str, &[u8]); 2] = [
    (
        "Bold",
        include_bytes!("../../../assets/fonts/Inter-Bold.otf"),
    ),
    (
        "Black",
        include_bytes!("../../../assets/fonts/Inter-Black.otf"),
    ),
];

/// A face of Inter the program carries, when that is what `family` and `face`
/// name. Regular is the built-in face itself.
fn bundled(family: &str, face: &str) -> Option<Arc<Face>> {
    if !family.eq_ignore_ascii_case(INTER) {
        return None;
    }
    if face.is_empty() || face.eq_ignore_ascii_case("Regular") {
        return Some(builtin());
    }
    let (_, bytes) = BUNDLED
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(face))?;
    let bytes: Bytes = Arc::new(*bytes);
    Face::new(bytes, 0, None).map(Arc::new)
}

struct Registry {
    system: fontique::Collection,
    /// The fonts of the files handed to [`register_file`].
    registered: fontique::Collection,
    /// Those files, so one is never added twice.
    files: Vec<PathBuf>,
    loaded: HashMap<(String, String), Arc<Face>>,
    /// The families the library's fonts made, each with what its files hold,
    /// as a frame key reads it. Only looked up, so the map's order decides
    /// nothing.
    library: HashMap<fontique::FamilyId, [u8; 32]>,
    /// Moved on by every reload of the library, so a face read from the
    /// collection a reload replaced is used once and never kept.
    generation: u64,
    /// The file each library font was read from, by the id of the bytes it
    /// was read into, so a template can pack a library font.
    library_files: HashMap<u64, PathBuf>,
}

/// The system's fonts and the faces loaded so far.
///
/// One lock for both, taken for a lookup and let go of before a face's file
/// is read: the interface measures text under the same lock, and must not
/// wait on a disk a render is reading. Every later draw with that face is a
/// map lookup.
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            system: fontique::Collection::new(fontique::CollectionOptions {
                shared: false,
                system_fonts: true,
            }),
            registered: fontique::Collection::new(fontique::CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            files: Vec::new(),
            loaded: HashMap::new(),
            library: HashMap::new(),
            generation: 0,
            library_files: HashMap::new(),
        })
    })
}

type Found = Option<(String, String)>;

/// The answers [`by_postscript_name`] has given, forgotten when the library
/// changes. Grows by one entry per name a project asks for.
fn postscript_seen() -> &'static Mutex<HashMap<String, Found>> {
    static SEEN: OnceLock<Mutex<HashMap<String, Found>>> = OnceLock::new();
    SEEN.get_or_init(Mutex::default)
}

/// Reads the library's fonts again from `folder`, every `.ttf`, `.otf`,
/// `.ttc` or `.otc` in it or in a folder inside it, and draws with them from
/// now on. `None`, or a folder that isn't there, leaves only the system's.
/// Said when the library home changes and whenever its fonts folder does.
///
/// The files are read with no lock held. The registry then drops the last
/// library's fonts and every face loaded so far, so the next draw looks for
/// its font again. Clearing rather than making a new collection keeps the
/// system's fonts as they were scanned.
pub fn use_library(folder: Option<&Path>) {
    let mut paths = Vec::new();
    if let Some(folder) = folder {
        font_files(folder, &mut paths);
    }
    paths.sort();
    let files: Vec<(PathBuf, Vec<u8>)> = paths
        .into_iter()
        .filter_map(|path| std::fs::read(&path).ok().map(|bytes| (path, bytes)))
        .collect();
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    registry.system.clear();
    registry.library_files.clear();
    let mut library: HashMap<fontique::FamilyId, blake3::Hasher> = HashMap::new();
    for (path, bytes) in files {
        let digest = blake3::hash(&bytes);
        let blob: fontique::Blob<u8> = bytes.into();
        registry.library_files.insert(blob.id(), path);
        for (family, _) in registry.system.register_fonts(blob, None) {
            library.entry(family).or_default().update(digest.as_bytes());
        }
    }
    registry.library = library
        .into_iter()
        .map(|(family, h)| (family, *h.finalize().as_bytes()))
        .collect();
    let ids: Vec<fontique::FamilyId> = registry.library.keys().copied().collect();
    let mut names: Vec<String> = ids
        .into_iter()
        .filter_map(|id| registry.system.family_name(id).map(str::to_owned))
        .collect();
    registry.loaded.clear();
    registry.generation = registry.generation.wrapping_add(1);
    drop(registry);
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    *library_names()
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = names;
    postscript_seen()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// Every font file in `folder` and the folders inside it, leaving out links
/// and names that start with a dot.
fn font_files(folder: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            font_files(&path, out);
        } else if kind.is_file() && is_font_file(&path) {
            out.push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["ttf", "otf", "ttc", "otc"]
            .iter()
            .any(|f| e.eq_ignore_ascii_case(f))
    })
}

/// The families the last [`use_library`] found, kept apart from the registry
/// so asking never waits on the system's fonts being scanned.
fn library_names() -> &'static Mutex<Vec<String>> {
    static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    &NAMES
}

/// The families that come from the library's fonts, sorted the way a menu
/// reads.
#[must_use]
pub fn library_families() -> Vec<String> {
    library_names()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// What the library's files of `family` hold, when that family is drawn from
/// the library, for a frame key to fold in. A text layer drawn before its font
/// arrived, or after it left, is then named apart from one drawn with it.
/// `None` for a family the system has or nobody has, which is what every
/// frame key made before the library carries.
#[must_use]
pub fn library_identity(family: &str) -> Option<[u8; 32]> {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if registry.library.is_empty() || family.is_empty() {
        return None;
    }
    let id = registry.system.family_id(family)?;
    registry.library.get(&id).copied()
}

/// The family and faces a font file holds, read from its bytes, or `None`
/// when it isn't a font this engine draws with.
#[must_use]
pub fn file_info(bytes: &[u8]) -> Option<(String, Vec<String>)> {
    let file = skrifa::raw::FileRef::new(bytes).ok()?;
    let mut family: Option<String> = None;
    let mut faces: Vec<String> = Vec::new();
    for font in file.fonts().flatten() {
        if family.is_none() {
            family = name(&font, StringId::TYPOGRAPHIC_FAMILY_NAME)
                .or_else(|| name(&font, StringId::FAMILY_NAME));
        }
        for (face, _) in faces_in(&font) {
            if !faces.contains(&face) {
                faces.push(face);
            }
        }
    }
    Some((family?, faces))
}

fn name(font: &FontRef<'_>, id: StringId) -> Option<String> {
    let name: String = font
        .localized_strings(id)
        .english_or_first()?
        .chars()
        .collect();
    (!name.is_empty()).then_some(name)
}

/// What a face is called inside its family: "Bold Italic", "Condensed".
fn face_name(font: &FontRef<'_>) -> String {
    name(font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME)
        .or_else(|| name(font, StringId::SUBFAMILY_NAME))
        .unwrap_or_else(|| "Regular".to_owned())
}

/// Every face one font file holds: the named places of a variable font, or the
/// one face of a fixed font.
fn faces_in(font: &FontRef<'_>) -> Vec<(String, Option<usize>)> {
    let instances: Vec<(String, Option<usize>)> = font
        .named_instances()
        .iter()
        .enumerate()
        .filter_map(|(i, instance)| Some((name(font, instance.subfamily_name_id())?, Some(i))))
        .collect();
    if instances.is_empty() {
        vec![(face_name(font), None)]
    } else {
        instances
    }
}

/// The font families installed on this machine, in the library or handed
/// over as files, sorted the way a menu reads.
#[must_use]
pub fn families() -> Vec<String> {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let mut names: Vec<String> = registry.system.family_names().map(str::to_owned).collect();
    names.extend(registry.registered.family_names().map(str::to_owned));
    names.push(INTER.to_owned());
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// How many fonts each registered family holds.
fn registered_counts(registered: &mut fontique::Collection) -> HashMap<String, usize> {
    let names: Vec<String> = registered.family_names().map(str::to_owned).collect();
    names
        .into_iter()
        .map(|name| {
            let fonts = registered.family_by_name(&name);
            let count = fonts.map_or(0, |info| info.fonts().len());
            (name, count)
        })
        .collect()
}

/// Add the fonts in the file at `path` to the fonts Lumit knows, so a
/// template's packed font draws without being installed. Answers the families
/// it added. A face that had been substituted for one of those families is
/// forgotten, so the next draw finds the real one.
///
/// A file that is not a font, or one already added, adds nothing.
pub fn register_file(path: &Path) -> Vec<String> {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if !path.is_file() || registry.files.iter().any(|had| had == path) {
        return Vec::new();
    }
    // Only the file's names are read under the lock, once a file. Its
    // outlines are read when a face is first drawn, with the lock let go of.
    let before = registered_counts(&mut registry.registered);
    registry.registered.load_fonts_from_paths([path]);
    let mut added: Vec<String> = registered_counts(&mut registry.registered)
        .into_iter()
        .filter(|(name, count)| before.get(name).is_none_or(|had| had < count))
        .map(|(name, _)| name)
        .collect();
    if added.is_empty() {
        return added;
    }
    added.sort();
    registry.files.push(path.to_path_buf());
    let named = |family: &str| added.iter().any(|a| a.eq_ignore_ascii_case(family));
    registry.loaded.retain(|(family, _), _| !named(family));
    added
}

/// Register every font file in `dir` (ttf, otf, ttc). What start-up calls for
/// the templates' fonts folder.
pub fn register_dir(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let extension = path.extension().and_then(|e| e.to_str());
            extension.is_some_and(|e| {
                ["ttf", "otf", "ttc"]
                    .iter()
                    .any(|font| e.eq_ignore_ascii_case(font))
            })
        })
        .collect();
    files.sort();
    let mut added: Vec<String> = Vec::new();
    for family in files.iter().flat_map(|file| register_file(file)) {
        if !added.contains(&family) {
            added.push(family);
        }
    }
    added
}

/// Whether `font` holds a face called `wanted`. An empty `wanted` is any face.
fn holds(font: &fontique::FontInfo, wanted: &str) -> bool {
    let Some(blob) = font.load(None) else {
        return false;
    };
    FontRef::from_index(blob.as_ref(), font.index()).is_ok_and(|parsed| {
        faces_in(&parsed)
            .iter()
            .any(|(name, _)| wanted.is_empty() || name.eq_ignore_ascii_case(wanted))
    })
}

/// The file a face is read from, for packing it, a library font's included.
/// `None` for a bundled face or one that is not installed.
///
/// An empty `face` is the family's regular, as a layer reads it. A face that
/// would be drawn with a stand-in has no file.
#[must_use]
pub fn file_of(family: &str, face: &str) -> Option<PathBuf> {
    let fonts = fonts_named(family);
    let passes: &[&str] = if face.is_empty() {
        &["Regular", ""]
    } else {
        &[face]
    };
    passes.iter().find_map(|wanted| {
        fonts.iter().find_map(|font| {
            let path = match font.source().kind() {
                fontique::SourceKind::Path(path) => path.to_path_buf(),
                fontique::SourceKind::Memory(blob) => registry()
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .library_files
                    .get(&blob.id())?
                    .clone(),
            };
            holds(font, wanted).then_some(path)
        })
    })
}

/// Whether `family` (with `face`, or any face when `face` is empty) is known:
/// installed, registered or bundled.
#[must_use]
pub fn installed(family: &str, face: &str) -> bool {
    if family.is_empty() || bundled(family, face).is_some() {
        return true;
    }
    let fonts = fonts_named(family);
    !fonts.is_empty() && (face.is_empty() || has_face(&fonts, face))
}

/// The faces of one family, in the order the family lists them, with the
/// faces of Inter the program carries. Empty when the family is neither
/// installed nor in the library.
#[must_use]
pub fn faces(family: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if family.eq_ignore_ascii_case(INTER) {
        out.push("Regular".to_owned());
        out.extend(BUNDLED.iter().map(|(name, _)| (*name).to_owned()));
    }
    for font in &fonts_named(family) {
        let Some(blob) = font.load(None) else {
            continue;
        };
        let Ok(parsed) = FontRef::from_index(blob.as_ref(), font.index()) else {
            continue;
        };
        for (face, _) in faces_in(&parsed) {
            if !out.contains(&face) {
                out.push(face);
            }
        }
    }
    out
}

/// The family and face of the installed font with this PostScript name, which
/// is how an After Effects project names a font. `None` when it isn't
/// installed here.
///
/// ponytail: reads the name of every installed font, once per name asked for.
/// Keep an index if a project with many fonts opens slowly.
#[must_use]
pub fn by_postscript_name(postscript: &str) -> Option<(String, String)> {
    let seen = postscript_seen();
    if let Some(found) = seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(postscript)
    {
        return found.clone();
    }
    let found = families().into_iter().find_map(|family| {
        fonts_named(&family).iter().find_map(|font| {
            let blob = font.load(None)?;
            let parsed = FontRef::from_index(blob.as_ref(), font.index()).ok()?;
            (name(&parsed, StringId::POSTSCRIPT_NAME)? == postscript)
                .then(|| (family.clone(), face_name(&parsed)))
        })
    });
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(postscript.to_owned(), found.clone());
    found
}

/// The face a layer names, or the nearest thing this machine has: the family's
/// regular when the face is unknown, the built-in Inter when the family is. A
/// face of Inter the system has wins over the one the program carries.
pub(crate) fn face(family: &str, face: &str) -> Arc<Face> {
    if family.is_empty() {
        return builtin();
    }
    let key = (family.to_owned(), face.to_owned());
    let known = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = known.loaded.get(&key) {
        return found.clone();
    }
    let generation = known.generation;
    drop(known);
    // Read with the lock let go of. Two threads after the same new face both
    // read it, and the first one in is the one kept.
    let fonts = fonts_named(family);
    let found = bundled(family, face)
        .filter(|_| !has_face(&fonts, face))
        .or_else(|| load(&fonts, face))
        .unwrap_or_else(builtin);
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if registry.generation != generation {
        return found;
    }
    registry.loaded.entry(key).or_insert(found).clone()
}

/// The fonts of the family of this name, the system's first and then the
/// registered files', looked up under the lock and handed back to read from
/// without it.
fn fonts_named(family: &str) -> Vec<fontique::FontInfo> {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let system = registry.system.family_by_name(family);
    let registered = registry.registered.family_by_name(family);
    [system, registered]
        .iter()
        .flatten()
        .flat_map(|info| info.fonts().iter().cloned())
        .collect()
}

/// Whether the family holds a face of exactly this name.
fn has_face(fonts: &[fontique::FontInfo], face: &str) -> bool {
    !face.is_empty() && fonts.iter().any(|font| holds(font, face))
}

fn load(fonts: &[fontique::FontInfo], face: &str) -> Option<Arc<Face>> {
    // The first pass looks for the face by name. The second takes the family's
    // regular, and the last whatever the family holds.
    for wanted in [face, "Regular", ""] {
        for font in fonts {
            let Some(blob) = font.load(None) else {
                continue;
            };
            let Ok(parsed) = FontRef::from_index(blob.as_ref(), font.index()) else {
                continue;
            };
            let Some((_, instance)) = faces_in(&parsed)
                .into_iter()
                .find(|(name, _)| wanted.is_empty() || name.eq_ignore_ascii_case(wanted))
            else {
                continue;
            };
            let bytes: Bytes = Arc::new(blob);
            if let Some(loaded) = Face::new(bytes, font.index(), instance) {
                return Some(Arc::new(loaded));
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::TextBlock;
    use lumit_core::model::LinearColour;
    use lumit_core::text::{ParagraphStyle, TextStyle};

    /// **A font in the library is listed and drawn with, and once it is gone
    /// drawing falls back to Inter without a fault.** Paper Mono, which the
    /// interface ships, stands in for a font the person keeps; it is read
    /// from a temporary library folder, never a real one.
    #[test]
    fn a_library_font_is_used_and_falls_back_once_deleted() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../flutter_ui/assets/fonts/PaperMono-Regular.otf");
        let bytes = std::fs::read(&source).unwrap();
        let (family, faces) = file_info(&bytes).expect("Paper Mono is a font");
        assert!(!faces.is_empty());
        assert!(file_info(b"not a font").is_none());
        let installed = families().contains(&family);

        let folder = std::env::temp_dir().join(format!("lumit-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("Mono")).unwrap();
        let kept = folder.join("Mono").join("PaperMono-Regular.otf");
        std::fs::write(&kept, &bytes).unwrap();

        use_library(Some(&folder));
        assert_eq!(library_families(), vec![family.clone()]);
        assert!(families().contains(&family));
        assert!(library_identity(&family).is_some());
        assert!(
            !Arc::ptr_eq(&face(&family, ""), &builtin()),
            "the library font is drawn with"
        );
        let style = TextStyle {
            family: family.clone(),
            ..TextStyle::default()
        };
        let paragraph = ParagraphStyle::default();
        let block = TextBlock {
            text: "Library",
            size: 32.0,
            fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
            style: &style,
            paragraph: &paragraph,
            boxed: false,
        };
        let with = crate::rasterise(&block, &[]);

        std::fs::remove_dir_all(&folder).unwrap();
        use_library(Some(&folder));
        assert!(library_families().is_empty());
        assert!(library_identity(&family).is_none());
        if !installed {
            assert!(
                Arc::ptr_eq(&face(&family, ""), &builtin()),
                "falls back to Inter"
            );
        }
        let without = crate::rasterise(&block, &[]);
        assert!(without.width > 0 && without.height > 0);
        if !installed {
            assert_ne!(with.rgba, without.rgba, "drawn in a different font");
        }
        use_library(None);
    }
}
