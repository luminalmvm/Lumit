//! Where this machine last saw media that a project can't point at with a relative path.
//!
//! A saved project only carries relative paths, so a file on a different drive to the
//! project has nothing to be relative to and saves as its bare name. Its real location is
//! kept here instead, in the local app data folder and keyed by its fingerprint, so the
//! project still opens with its media on this machine and the `.lum` holds no absolute paths.
//! Losing this file only costs a relink.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lumit_core::model::{MediaRef, ProjectItem};
use lumit_core::Document;

/// How many places are kept, oldest dropped first.
const MAX_PLACES: usize = 4096;

/// Saves and autosaves run on different threads, so the read and write happen one at a time.
static STORE: Mutex<()> = Mutex::new(());

/// Fingerprint hash and absolute path, oldest first.
type Places = Vec<(String, String)>;

/// Where the places are kept. `None` when the platform has no home directory.
#[must_use]
pub fn media_places_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_local_dir().join("media-places.json"))
}

/// Remember every file in the saved `doc` that its relative path won't find again.
pub fn remember(doc: &Document, project_dir: &Path) {
    if let Some(store) = media_places_path() {
        remember_in(&store, doc, project_dir);
    }
}

/// Fill in the absolute path of every reference in a just opened `doc` this machine has seen.
pub fn recall(doc: &mut Document) {
    if let Some(store) = media_places_path() {
        recall_from(&store, doc);
    }
}

/// [`remember`] with the store named, so the tests don't touch the real one.
pub fn remember_in(store: &Path, doc: &Document, project_dir: &Path) {
    let footage = doc.items.iter().filter_map(|item| match item {
        ProjectItem::Footage(f) => Some(&f.media),
        _ => None,
    });
    let lost: Places = footage
        .chain(doc.proxies.values().map(|p| &p.media))
        .chain(doc.colour.config.as_ref())
        .filter_map(|m| {
            let fp = m.fingerprint.as_ref()?;
            let found_by_relative = project_dir.join(&m.relative_path).is_file();
            (!found_by_relative
                && !m.absolute_path.is_empty()
                && Path::new(&m.absolute_path).is_file())
            .then(|| (fp.head_tail_hash.clone(), m.absolute_path.clone()))
        })
        .collect();
    if lost.is_empty() {
        return;
    }

    let _held = STORE.lock().unwrap_or_else(|held| held.into_inner());
    let mut places = load(store);
    let mut changed = false;
    for place in lost {
        if places.contains(&place) {
            continue;
        }
        places.retain(|(hash, _)| *hash != place.0);
        places.push(place);
        changed = true;
    }
    if !changed {
        return;
    }
    let excess = places.len().saturating_sub(MAX_PLACES);
    places.drain(..excess);
    // Written beside and renamed over, so a crash mid-write leaves the old list.
    let Ok(text) = serde_json::to_string(&places) else {
        return;
    };
    if let Some(dir) = store.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = store.with_extension("json.tmp");
    if std::fs::write(&temp, text).is_ok() {
        let _ = std::fs::rename(&temp, store);
    }
}

/// [`recall`] with the store named, so the tests don't touch the real one.
pub fn recall_from(store: &Path, doc: &mut Document) {
    let places = {
        let _held = STORE.lock().unwrap_or_else(|held| held.into_inner());
        load(store)
    };
    if places.is_empty() {
        return;
    }
    let mut refs: Vec<&mut MediaRef> = doc
        .items
        .iter_mut()
        .filter_map(|item| match item {
            ProjectItem::Footage(f) => Some(&mut f.media),
            _ => None,
        })
        .collect();
    refs.extend(doc.proxies.values_mut().map(|p| &mut p.media));
    refs.extend(doc.colour.config.as_mut());
    for media in refs {
        if !media.absolute_path.is_empty() {
            continue;
        }
        let Some(fp) = &media.fingerprint else {
            continue;
        };
        if let Some((_, path)) = places
            .iter()
            .rev()
            .find(|(hash, _)| *hash == fp.head_tail_hash)
        {
            media.absolute_path = path.clone();
        }
    }
}

/// An absent or damaged file reads as nothing remembered.
fn load(store: &Path) -> Places {
    std::fs::read_to_string(store)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::FootageItem;
    use uuid::Uuid;

    /// A document holding one footage item named `song.mp3` pointing at `file`.
    fn doc_with(file: &Path) -> Document {
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Footage(FootageItem {
            id: Uuid::now_v7(),
            name: "song.mp3".into(),
            media: MediaRef {
                // What a save writes when there's no relative path, as across drives.
                relative_path: "song.mp3".into(),
                absolute_path: file.to_string_lossy().into_owned(),
                fingerprint: crate::fingerprint_path(file).ok(),
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            sequence: None,
            extra: serde_json::Map::new(),
        }));
        doc
    }

    /// The same document as it comes back out of the `.lum`, with no absolute path.
    fn reopened(doc: &Document) -> Document {
        let mut doc = doc.clone();
        for item in &mut doc.items {
            if let ProjectItem::Footage(f) = item {
                f.media.absolute_path.clear();
            }
        }
        doc
    }

    #[test]
    fn a_file_with_no_relative_path_is_found_again_after_reopening() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let music = root.path().join("music");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&music).unwrap();
        let file = music.join("song.mp3");
        std::fs::write(&file, b"not really an mp3").unwrap();
        let store = root.path().join("places.json");

        let saved = doc_with(&file);
        remember_in(&store, &saved, &project);

        // Without the places the file is lost, which is the bug.
        let (_, missing) = crate::resolve_all_media(&mut reopened(&saved), &project, &[]);
        assert_eq!(missing, vec!["song.mp3".to_string()]);

        let mut doc = reopened(&saved);
        recall_from(&store, &mut doc);
        let (_, missing) = crate::resolve_all_media(&mut doc, &project, &[]);
        assert!(missing.is_empty(), "still missing: {missing:?}");
        let Some(ProjectItem::Footage(f)) = doc.items.first() else {
            panic!("the footage item went away");
        };
        assert_eq!(Path::new(&f.media.absolute_path), file);
    }

    #[test]
    fn a_file_its_relative_path_finds_is_not_remembered() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("song.mp3");
        std::fs::write(&file, b"beside the project").unwrap();
        let store = root.path().join("places.json");

        remember_in(&store, &doc_with(&file), root.path());
        assert!(!store.exists());
    }
}
