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

/// Put back a file kept from an earlier page.
#[wasm_bindgen]
pub fn lumit_web_restore(path: String, bytes: Vec<u8>) {
    let path = Path::new(&path);
    if let Some(dir) = path.parent() {
        let _ = files::create_dir_all(dir);
    }
    let _ = files::write(path, &bytes);
}
