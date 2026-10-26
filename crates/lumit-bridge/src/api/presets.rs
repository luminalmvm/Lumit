//! Groups in the preset library.
//!
//! A group is a folder inside the library, so the folders on disk are the
//! only record of which groups there are and what is in them.

use std::path::{Component, Path, PathBuf};

use flutter_rust_bridge::frb;

use crate::api::{effect::BridgeFfxImport, BridgeError};

/// Every group in the preset library, with `/` between nested folders, sorted
/// by name. An empty folder is a group too.
#[frb(sync)]
pub fn list_preset_groups() -> Vec<String> {
    let Some(library) = lumit_project::presets_dir() else {
        return Vec::new();
    };
    let mut groups: Vec<String> = folders_under(&library)
        .iter()
        .filter_map(|folder| group_of(&library, folder))
        .filter(|group| !group.is_empty())
        .collect();
    groups.sort_by_key(|group| group.to_lowercase());
    groups
}

/// Make a group, and any group it sits in.
#[frb(sync)]
pub fn make_preset_group(group: String) -> Result<(), BridgeError> {
    std::fs::create_dir_all(group_folder(&group)?).map_err(|_| BridgeError::WriteFailed)
}

/// Give a group a new name where it is. Refused when another group beside it
/// already has that name.
#[frb(sync)]
pub fn rename_preset_group(group: String, name: String) -> Result<(), BridgeError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    if !is_plain_name(name) {
        return Err(BridgeError::InvalidPreset);
    }
    let from = group_folder(&group)?;
    move_to(&from, &from.with_file_name(name))
}

/// Delete a group with every preset and group in it.
#[frb(sync)]
pub fn delete_preset_group(group: String) -> Result<(), BridgeError> {
    std::fs::remove_dir_all(group_folder(&group)?).map_err(|_| BridgeError::WriteFailed)
}

/// Move a saved preset into `group`, or to the top of the library when
/// `group` is empty. Refused when a preset of the same file name is there.
#[frb(sync)]
pub fn move_preset(path: String, group: String) -> Result<(), BridgeError> {
    let library = lumit_project::presets_dir().ok_or(BridgeError::InvalidPreset)?;
    let from = Path::new(&path);
    // Only a file already in the library is moved.
    let inside = std::fs::canonicalize(from)
        .ok()
        .zip(std::fs::canonicalize(&library).ok())
        .is_some_and(|(file, library)| file.starts_with(library));
    let name = from.file_name().filter(|_| inside && from.is_file());
    let name = name.ok_or(BridgeError::InvalidPreset)?;
    let folder = folder_of(&library, &group).ok_or(BridgeError::InvalidPreset)?;
    std::fs::create_dir_all(&folder).map_err(|_| BridgeError::WriteFailed)?;
    move_to(from, &folder.join(name))
}

/// `root` and every folder inside it, however deep. A link to a folder is not
/// followed, so a loop of links cannot hang the walk.
#[frb(ignore)]
pub(crate) fn folders_under(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.to_path_buf()];
    let mut next = 0;
    while let Some(folder) = folders.get(next).cloned() {
        next += 1;
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        folders.extend(
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path()),
        );
    }
    folders
}

/// The group `folder` is in the library: its path from there, with `/`
/// between folders. Empty for the library itself.
#[frb(ignore)]
pub(crate) fn group_of(library: &Path, folder: &Path) -> Option<String> {
    let steps: Vec<String> = folder
        .strip_prefix(library)
        .ok()?
        .components()
        .map(|step| step.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(steps.join("/"))
}

/// The folder a group names in the library. None when a step is not a plain
/// name, so no group reaches outside the library.
fn folder_of(library: &Path, group: &str) -> Option<PathBuf> {
    let mut folder = library.to_path_buf();
    for step in group.split('/').filter(|step| !step.is_empty()) {
        if !is_plain_name(step) {
            return None;
        }
        folder.push(step);
    }
    Some(folder)
}

/// A group's folder in the user's library, never the library itself.
fn group_folder(group: &str) -> Result<PathBuf, BridgeError> {
    let library = lumit_project::presets_dir().ok_or(BridgeError::InvalidPreset)?;
    folder_of(&library, group)
        .filter(|folder| *folder != library)
        .ok_or(BridgeError::InvalidPreset)
}

/// One file or folder name: no separators, no drive and no `..`.
fn is_plain_name(name: &str) -> bool {
    let mut steps = Path::new(name).components();
    matches!(
        (steps.next(), steps.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// Move a file or folder, never onto something else already there. The same
/// thing under another spelling is not a clash, since a file system that
/// ignores case finds the old one at the new name.
fn move_to(from: &Path, to: &Path) -> Result<(), BridgeError> {
    if to.exists() && std::fs::canonicalize(to).ok() != std::fs::canonicalize(from).ok() {
        return Err(BridgeError::InvalidPreset);
    }
    std::fs::rename(from, to).map_err(|_| BridgeError::WriteFailed)
}

/// Convert every `.ffx` in `folder` and the folders inside it into the
/// library at `library`, each into the same folders it sat in under `folder`.
/// On any library folder, so it is testable without the user's own.
#[frb(ignore)]
pub(crate) fn import_ffx_into(folder: &Path, library: &Path) -> BridgeFfxImport {
    let mut done = BridgeFfxImport {
        imported: 0,
        skipped: 0,
    };
    for from in folders_under(folder) {
        let Ok(entries) = std::fs::read_dir(&from) else {
            continue;
        };
        let into = library.join(from.strip_prefix(folder).unwrap_or(Path::new("")));
        for path in entries.flatten().map(|entry| entry.path()) {
            let is_ffx = path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("ffx"));
            if !is_ffx {
                continue;
            }
            if write_ffx_as_lumfx(&path, library, &into) {
                done.imported += 1;
            } else {
                done.skipped += 1;
            }
        }
    }
    done
}

/// One `.ffx` into `into` under its own name. False when it is left out.
fn write_ffx_as_lumfx(ffx: &Path, library: &Path, into: &Path) -> bool {
    let Some(name) = ffx
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
    else {
        return false;
    };
    let file = format!("{name}.{}", lumit_core::preset::PRESET_EXTENSION);
    let to = into.join(&file);
    if to.exists() {
        return false;
    }
    let Ok((effects, _report)) = lumit_import::open_ffx(ffx) else {
        return false;
    };
    // A preset of text animators or a transform alone has nothing Lumit reads.
    if effects.is_empty() || std::fs::create_dir_all(into).is_err() {
        return false;
    }
    let Ok(text) = lumit_core::preset::to_json(&name, &effects) else {
        return false;
    };
    // An import from before folders were kept left this preset at the top of
    // the library. The same preset there moves into its folder instead.
    let top = library.join(&file);
    if into != library && same_effects(&top, &text) {
        return std::fs::rename(&top, &to).is_ok();
    }
    std::fs::write(&to, text).is_ok()
}

/// Whether the preset at `path` is `text` but for its effects' ids and random
/// seeds, which each conversion makes new. Compared as written: a number read
/// back from the file can land a hair off the one in memory.
fn same_effects(path: &Path, text: &str) -> bool {
    let plain = |text: &str| -> Option<serde_json::Value> {
        let mut preset: serde_json::Value = serde_json::from_str(text).ok()?;
        for effect in preset.get_mut("effects")?.as_array_mut()? {
            let effect = effect.as_object_mut()?;
            effect.remove("id");
            let params = effect.get_mut("params").and_then(|p| p.as_array_mut());
            for param in params.into_iter().flatten() {
                if let Some(value) = param.get_mut("value").and_then(|v| v.as_object_mut()) {
                    value.remove("Seed");
                }
            }
        }
        Some(preset)
    };
    let saved = std::fs::read_to_string(path).ok();
    saved
        .and_then(|saved| plain(&saved))
        .is_some_and(|saved| plain(text) == Some(saved))
}
