//! Where the page keeps Lumit's files past a reload (flutter_ui/web/files.js).
//! The files live in memory (`lumit_ingress::files`), and the ones worth
//! keeping are copied into the browser's own storage on this machine a moment
//! after they change, and put back as the next page starts.

use std::path::Path;

use lumit_ingress::files;
use wasm_bindgen::prelude::wasm_bindgen;

/// Settings, Lumit's own folder, the files the person opened, and the
/// journals of saved projects, which hold the edits made since. Not the
/// cache, nor the journal of a project never saved, which nothing reads back.
fn kept(path: &str) -> bool {
    ["/home/.config/", "/home/.local/share/lumit/", "/web/"]
        .iter()
        .any(|folder| path.starts_with(folder))
        || path.starts_with("/home/.cache/lumit/") && path.contains("/journal/ops-")
}

/// The kept files written or deleted since this was last asked.
#[wasm_bindgen]
pub fn lumit_web_changed() -> Vec<String> {
    let mut changed = files::take_changed();
    changed.retain(|path| kept(path));
    changed
}

/// A file's bytes, or nothing when it has gone.
#[wasm_bindgen]
pub fn lumit_web_file(path: String) -> Option<Vec<u8>> {
    files::read(Path::new(&path)).ok()
}

/// The `/web/<id>/` folders the projects at `projects` use: each one's own,
/// which holds its autosaves, and every one their footage, their autosaves'
/// or an open project's is in. A file opened into any other folder is used by
/// none of them.
#[wasm_bindgen]
pub fn lumit_web_used(projects: Vec<String>) -> Vec<String> {
    let mut docs = Vec::new();
    let mut used: Vec<String> = projects.iter().filter_map(|p| folder(p)).collect();
    for project in projects.iter().map(Path::new) {
        let autosaves = project.with_file_name("autosaves");
        let saved = files::list(&autosaves).unwrap_or_default();
        let saved = saved.into_iter().map(|(name, _)| autosaves.join(name));
        for lum in std::iter::once(project.to_path_buf()).chain(saved) {
            if let Ok((doc, _)) = lumit_project::open(&lum) {
                docs.push(std::sync::Arc::new(doc));
            }
        }
    }
    if let Ok(open) = crate::api::state::PROJECTS.read() {
        let open = open
            .values()
            .filter_map(|p| Some(p.read().ok()?.store.snapshot()));
        docs.extend(open);
    }
    for doc in &docs {
        for item in &doc.items {
            if let lumit_core::model::ProjectItem::Footage(f) = item {
                used.extend(folder(&f.media.absolute_path));
                used.extend(folder(&f.media.relative_path));
            }
        }
    }
    used.sort_unstable();
    used.dedup();
    used
}

/// The `/web/<id>/` folder `path` is in, if it is in one.
fn folder(path: &str) -> Option<String> {
    let id = path.strip_prefix("/web/")?.split('/').next()?;
    Some(format!("/web/{id}/"))
}

/// Put back a file kept from an earlier page.
#[wasm_bindgen]
pub fn lumit_web_restore(path: String, bytes: Vec<u8>) {
    let path = Path::new(&path);
    if let Some(dir) = path.parent() {
        let _ = files::create_dir_all(dir);
    }
    let _ = files::write(path, &bytes);
}
