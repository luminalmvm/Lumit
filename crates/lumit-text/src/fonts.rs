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
pub(crate) struct Face {
    bytes: Bytes,
    index: u32,
    /// The font where this face sits in a variable font's design space, which
    /// is what makes Bahnschrift's Bold a bold.
    pub(crate) shaper: harfrust::Font,
}

impl Face {
    fn new(bytes: Bytes, index: u32, instance: Option<usize>) -> Option<Self> {
        let font = harfrust::Font::new(bytes.clone(), index)?;
        let shaper = match instance {
            Some(i) => font.instance_builder().named_instance(i).build(),
            None => font,
        };
        Some(Self {
            bytes,
            index,
            shaper,
        })
    }

    /// The same font as the outlines and metrics read it.
    pub(crate) fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index((*self.bytes).as_ref(), self.index).ok()
    }
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
