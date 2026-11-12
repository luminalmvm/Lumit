//! The user's library across the seam: where each kind's folder is for the
//! profile in force, the fonts kept in it, and packing chosen items into one
//! `.lumlibrary` file and back. The frontend owns the Library window and the
//! sync; the engine owns the folders and what it draws with.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use flutter_rust_bridge::frb;

use crate::api::BridgeError;

/// Where each of the library's engine kinds lives for the profile in force.
/// An empty string when this computer has no home folder. Nothing is made on
/// disk: a caller makes a folder when it first writes one.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLibraryFolders {
    /// Effect, animation, look and material presets, and node groups.
    pub presets: String,
    /// The user's own `.lens` files.
    pub lenses: String,
    /// The user's own `.flare` files.
    pub flares: String,
    /// Fonts drawn with beside the system's.
    pub fonts: String,
    /// LUT files a row finds when its own is missing.
    pub luts: String,
    /// The one file of export presets.
    pub export_presets: String,
    /// Saved templates, groups as sub-folders.
    pub templates: String,
}

#[frb(ignore)]
fn shown(path: Option<PathBuf>) -> String {
    path.map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The library's folders for the profile in force.
#[frb(sync)]
#[must_use]
pub fn library_folders() -> BridgeLibraryFolders {
    BridgeLibraryFolders {
        presets: shown(lumit_project::presets_dir()),
        lenses: shown(lumit_project::lenses_dir()),
        flares: shown(lumit_project::flares_dir()),
        fonts: shown(lumit_project::library_home::fonts_dir()),
        luts: shown(lumit_project::library_home::luts_dir()),
        export_presets: shown(lumit_project::export_presets_path()),
        templates: shown(lumit_project::templates_dir()),
    }
}

/// Read the fonts folder of the home in force and draw with what is there,
/// with the fonts its templates carried.
/// One at a time, so a reload that started for an older home can't land
/// after a newer one.
#[frb(ignore)]
fn reload_fonts() {
    static RELOAD: Mutex<()> = Mutex::new(());
    let _one = RELOAD.lock().unwrap_or_else(PoisonError::into_inner);
    lumit_text::fonts::use_library(lumit_project::library_home::fonts_dir().as_deref());
    // The fonts that came inside this home's templates. A file met before
    // is skipped, so a switch back and forth adds nothing twice.
    if let Some(dir) = lumit_project::template_file::fonts_dir().filter(|d| d.is_dir()) {
        lumit_text::fonts::register_dir(&dir);
    }
}

/// Use the library at `home`, the folder a profile's library lives in, or
/// with `null` the first profile's folders. Said on start and on every
/// profile switch. The fonts folder moves with it, so its fonts are read
/// again, on a thread of their own so this call returns at once.
#[frb(sync)]
pub fn library_use_home(home: Option<String>) {
    lumit_project::library_home::set_library_home(home.map(PathBuf::from));
    let spawned = std::thread::Builder::new()
        .name("library-fonts".into())
        .spawn(reload_fonts);
    if spawned.is_err() {
        reload_fonts();
    }
}

/// Read the library's fonts folder again, after a font was added, replaced
/// or removed. Text layers drawn with a font that came or went are drawn
/// again.
///
/// Not sync: it reads every font file in the folder.
pub fn library_fonts_changed() {
    reload_fonts();
}

/// The font families that come from the library's fonts, sorted for a menu.
#[frb(sync)]
#[must_use]
pub fn library_font_families() -> Vec<String> {
    lumit_text::fonts::library_families()
}

/// What a font file holds.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLibraryFont {
    /// The family it belongs to, as the Text panel lists it.
    pub family: String,
    /// Its faces, such as Regular and Bold Italic.
    pub faces: Vec<String>,
}

/// The family and faces of the font file at `path`, or `null` when it isn't
/// a font Lumit draws with or won't read.
///
/// Not sync: it reads the file.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn library_font_info(path: String) -> Option<BridgeLibraryFont> {
    let bytes = std::fs::read(&path).ok()?;
    let (family, faces) = lumit_text::fonts::file_info(&bytes)?;
    Some(BridgeLibraryFont { family, faces })
}

/// One thing in the library, packed or unpacked.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLibraryPackItem {
    /// The kind's id: `presets`, `styles`, `themes`, `fonts`, `shortcuts`,
    /// `lenses`, `flares`, `exports`, `luts` or `templates`.
    pub kind: String,
    /// The file or folder on this computer.
    pub path: String,
    /// Where it sits inside its kind's folder, `/` separated, such as
    /// `Looks/Teal.lumfx` or `Soft glass`.
    pub name: String,
}

/// Pack `items` into one `.lumlibrary` file at `out`, each stored as
/// `<kind>/<name>`, a folder item with everything in it. Written beside `out`
/// as `.part` and moved into place, so a failure leaves nothing. Refuses
/// links and anything over the package caps. Answers how many items went in.
///
/// Not sync: a folder of fonts takes a moment to pack.
#[allow(clippy::needless_pass_by_value)]
pub fn library_pack(items: Vec<BridgeLibraryPackItem>, out: String) -> Result<u32, BridgeError> {
    let items: Vec<lumit_extensions::library::LibraryItem> = items
        .into_iter()
        .map(|item| lumit_extensions::library::LibraryItem {
            kind: item.kind,
            path: PathBuf::from(item.path),
            name: item.name,
        })
        .collect();
    lumit_extensions::library::pack(&items, Path::new(&out)).map_err(|_| BridgeError::WriteFailed)
}

/// Unpack the `.lumlibrary` at `file` into the folder `into`, made if missing
/// and empty if not, as `<kind>/<name>`. Judged as a style package is: no
/// links, nothing that lands outside `into`, held to the caps. Items of a
/// kind this version doesn't know are skipped. Answers the items it holds,
/// with `path` where each landed.
///
/// Not sync: a package of fonts takes a moment to unpack.
#[allow(clippy::needless_pass_by_value)]
pub fn library_unpack(
    file: String,
    into: String,
) -> Result<Vec<BridgeLibraryPackItem>, BridgeError> {
    let items = lumit_extensions::library::unpack(Path::new(&file), Path::new(&into))
        .map_err(|_| BridgeError::ReadFailed)?;
    Ok(items
        .into_iter()
        .map(|item| BridgeLibraryPackItem {
            kind: item.kind,
            path: item.path.to_string_lossy().into_owned(),
            name: item.name,
        })
        .collect())
}
