//! Finding the fonts the system has, and loading the one a layer names.
//!
//! A layer names its font by family and face, the two strings the Text panel
//! shows. A font that isn't installed here falls back to the built-in Inter, so
//! a project opened on another machine still draws its words.
//!
//! ponytail: a missing glyph draws the font's own empty box, there is no
//! fallback to a second font for a script the first doesn't cover.

use std::collections::HashMap;
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
/// the list the system keeps, so no font file is opened.
#[must_use]
pub fn is_variable(family: &str) -> bool {
    family_named(family).is_some_and(|info| info.fonts().iter().any(|f| !f.axes().is_empty()))
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

struct Registry {
    system: fontique::Collection,
    loaded: HashMap<(String, String), Arc<Face>>,
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
            loaded: HashMap::new(),
        })
    })
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

/// The font families installed on this machine, sorted the way a menu reads.
#[must_use]
pub fn families() -> Vec<String> {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let mut names: Vec<String> = registry.system.family_names().map(str::to_owned).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// The faces of one family, in the order the family lists them. Empty when the
/// family isn't installed.
#[must_use]
pub fn faces(family: &str) -> Vec<String> {
    let Some(info) = family_named(family) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for font in info.fonts() {
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
    type Found = Option<(String, String)>;
    static SEEN: OnceLock<Mutex<HashMap<String, Found>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Mutex::default);
    if let Some(found) = seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(postscript)
    {
        return found.clone();
    }
    let found = families().into_iter().find_map(|family| {
        let info = family_named(&family)?;
        info.fonts().iter().find_map(|font| {
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
/// regular when the face is unknown, the built-in Inter when the family is.
pub(crate) fn face(family: &str, face: &str) -> Arc<Face> {
    if family.is_empty() {
        return builtin();
    }
    let key = (family.to_owned(), face.to_owned());
    let known = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = known.loaded.get(&key) {
        return found.clone();
    }
    drop(known);
    // Read with the lock let go of. Two threads after the same new face both
    // read it, and the first one in is the one kept.
    let found = family_named(family)
        .and_then(|info| load(&info, face))
        .unwrap_or_else(builtin);
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    registry.loaded.entry(key).or_insert(found).clone()
}

/// The family of this name among the system's, looked up under the lock and
/// handed back to read from without it.
fn family_named(family: &str) -> Option<fontique::FamilyInfo> {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    registry.system.family_by_name(family)
}

fn load(info: &fontique::FamilyInfo, face: &str) -> Option<Arc<Face>> {
    // The first pass looks for the face by name. The second takes the family's
    // regular, and the last whatever the family holds.
    for wanted in [face, "Regular", ""] {
        for font in info.fonts() {
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
