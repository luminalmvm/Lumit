//! lumit-bridge: the seam the Flutter frontend calls the Rust engine through
//! (docs/flutter-port/03-ARCHITECTURE.md).
//!
//! # In plain terms
//!
//! The Flutter application is written in Dart and cannot call Rust functions
//! directly. This crate is the translator between them. It compiles to a single
//! shared library (a `.dll` on Windows) that Dart loads at start-up; Dart then
//! calls the plain C functions this crate exports — `lumit_bridge_new_project`,
//! `lumit_bridge_snapshot`, and so on — passing text in and getting text back.
//!
//! The text is JSON (docs call this "bridge v0", a hand-rolled JSON-over-C-ABI
//! seam we keep until the API stabilises and `flutter_rust_bridge` generates
//! this layer instead). Every call returns a UTF-8, NUL-terminated string that
//! Rust allocated; the caller reads it and then hands it back to
//! [`lumit_bridge_free_string`] so Rust can release the memory. Every reply is
//! either `{"ok":true, …}` or `{"ok":false,"error":"…"}` — the error is a
//! calm sentence for the status line, never a crash.
//!
//! The engine-side document and its undo history live behind one process-wide
//! lock (there is exactly one Flutter window driving it, so a single client is
//! assumed). No exported function crosses the C boundary with a panic: each
//! body runs inside [`std::panic::catch_unwind`] and turns a panic into an
//! ordinary error reply.
//!
//! Engine crates never depend on this crate, and nothing depends on it — it is
//! a leaf over `lumit-core` and `lumit-project` (docs/05-ARCHITECTURE.md).

use lumit_core::model::{
    Composition, Document, Folder, FootageItem, LinearColour, MediaRef, ProjectItem,
};
use lumit_core::ops::{AutoFolderKind, Op};
use lumit_core::store::DocumentStore;
use lumit_core::time::{Duration, FrameRate, Rational};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// The C ABI generation. Bumped only when the exported function set or the JSON
/// shapes change incompatibly, so Dart can refuse a mismatched library.
const ABI_VERSION: u32 = 1;

/// The engine-side state one Flutter client drives: the document store (which
/// owns the document and its undo/redo history, exactly as `lumit-ui`'s
/// `AppState` does) plus the path the project was loaded from or last saved to.
struct Bridge {
    store: DocumentStore,
    path: Option<PathBuf>,
}

impl Bridge {
    fn new() -> Self {
        Self {
            store: DocumentStore::new(Document::new()),
            path: None,
        }
    }
}

/// The single process-wide client state (single-client assumption, above). The
/// lock is only ever held for the duration of one pure state transition below —
/// never across anything that could re-enter the bridge — so it cannot deadlock.
static BRIDGE: OnceLock<Mutex<Bridge>> = OnceLock::new();

/// Run `f` against the shared bridge state. A previous panic that poisoned the
/// lock is recovered from (the caught panic already produced an error reply;
/// the state itself is a valid `Document`), so one bad call never wedges the
/// bridge for the rest of the session.
fn with_bridge<R>(f: impl FnOnce(&mut Bridge) -> R) -> R {
    let mutex = BRIDGE.get_or_init(|| Mutex::new(Bridge::new()));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    f(&mut guard)
}

// ---------------------------------------------------------------------------
// The pure state transitions. These take a `&mut Bridge` (or nothing) and
// return the reply JSON as a `String`; they hold no lock and touch no global,
// so the tests drive their own `Bridge` in full isolation and the exported C
// functions route the shared one through them.
// ---------------------------------------------------------------------------

/// `{"ok":true,"version":"…","abi":1}` — stateless.
fn version() -> String {
    json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "abi": ABI_VERSION,
    })
    .to_string()
}

/// The document tree as the Project panel reads it. The document stores items
/// flat (`Document::items`) with folders referencing children by id; this walks
/// [`Document::root_items`] and nests each folder's children, so the JSON mirrors
/// the panel's real nesting rather than the flat storage. A malformed folder
/// cycle is broken by the `seen` set, never looped.
fn snapshot_value(bridge: &Bridge) -> Value {
    let doc = bridge.store.snapshot();
    let mut seen = HashSet::new();
    let items: Vec<Value> = doc
        .root_items()
        .into_iter()
        .filter_map(|id| item_value(&doc, id, &mut seen))
        .collect();
    json!({
        "ok": true,
        "items": items,
        "can_undo": bridge.store.can_undo(),
        "can_redo": bridge.store.can_redo(),
        "path": bridge.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
    })
}

fn snapshot(bridge: &Bridge) -> String {
    snapshot_value(bridge).to_string()
}

/// One item as `{id, name, kind, children}`. `children` is populated only for
/// folders (recursively); every other kind carries an empty list. Returns
/// `None` for an id already visited (cycle guard) or absent from the document.
fn item_value(doc: &Document, id: Uuid, seen: &mut HashSet<Uuid>) -> Option<Value> {
    if !seen.insert(id) {
        return None;
    }
    let item = doc.item(id)?;
    let children: Vec<Value> = match item {
        ProjectItem::Folder(f) => f
            .children
            .iter()
            .filter_map(|child| item_value(doc, *child, seen))
            .collect(),
        _ => Vec::new(),
    };
    Some(json!({
        "id": id.to_string(),
        "name": item.name(),
        "kind": item_kind(item),
        "children": children,
    }))
}

fn item_kind(item: &ProjectItem) -> &'static str {
    match item {
        ProjectItem::Footage(_) => "footage",
        ProjectItem::Folder(_) => "folder",
        ProjectItem::Composition(_) => "composition",
        ProjectItem::Solid(_) => "solid",
    }
}

fn new_project(bridge: &mut Bridge) -> String {
    bridge.store = DocumentStore::new(Document::new());
    bridge.path = None;
    snapshot(bridge)
}

fn open_project(bridge: &mut Bridge, path: &str) -> String {
    let path = PathBuf::from(path);
    match lumit_project::open(&path) {
        Ok((doc, _manifest)) => {
            bridge.store = DocumentStore::new(doc);
            bridge.path = Some(path);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("open project: {e}")),
    }
}

/// Save to `path`; an empty `path` means "save to the loaded path" and errors
/// when there is none yet (bridge v0 has no file dialog to ask for one). The
/// written document is rebased against its folder exactly as `lumit-ui`'s Save
/// does, so no machine-specific path reaches the file (K-173).
fn save_project(bridge: &mut Bridge, path: &str) -> String {
    let target = if path.is_empty() {
        match &bridge.path {
            Some(p) => p.clone(),
            None => {
                return err_json(
                    "save project: no path yet — this project has never been saved, so a path is required",
                );
            }
        }
    } else {
        PathBuf::from(path)
    };
    let dir = target.parent().unwrap_or_else(|| std::path::Path::new(""));
    let doc = lumit_project::rebase_for_save(&bridge.store.snapshot(), dir);
    match lumit_project::save(&doc, &target) {
        Ok(()) => {
            bridge.path = Some(target);
            snapshot(bridge)
        }
        Err(e) => err_json(format!("save project: {e}")),
    }
}

/// Create a composition and file it in the "Compositions" auto-folder, as one
/// undo step — the same op path `lumit-ui`'s `confirm_comp_dialog` commits, so
/// undo removes it cleanly. An empty `name` becomes "Comp N" (N counting the
/// compositions already present).
fn new_composition(bridge: &mut Bridge, name: &str) -> String {
    let doc = bridge.store.snapshot();
    let name = if name.trim().is_empty() {
        let existing = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Composition(_)))
            .count();
        format!("Comp {}", existing + 1)
    } else {
        name.to_owned()
    };

    let (frame_rate, duration) = match (FrameRate::new(60, 1), Rational::new(30, 1)) {
        (Ok(fr), Ok(dur)) => (fr, Duration(dur)),
        _ => return err_json("new composition: could not build the default frame rate"),
    };

    // Ensure the Compositions auto-folder exists (tracked by id, like the egui
    // frontend: renaming or nesting it keeps it the Compositions folder).
    let mut ops: Vec<Op> = Vec::new();
    let folder_id = match doc
        .auto_folders
        .compositions
        .filter(|id| doc.folder(*id).is_some())
    {
        Some(id) => id,
        None => {
            let id = Uuid::now_v7();
            ops.push(Op::AddItem {
                index: doc.items.len(),
                item: Box::new(ProjectItem::Folder(Folder {
                    id,
                    name: "Compositions".into(),
                    children: Vec::new(),
                    extra: serde_json::Map::new(),
                })),
            });
            ops.push(Op::SetAutoFolder {
                kind: AutoFolderKind::Compositions,
                folder: Some(id),
            });
            id
        }
    };

    let comp = Composition {
        id: Uuid::now_v7(),
        name,
        width: 1920,
        height: 1080,
        frame_rate,
        duration,
        background: LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;

    // The comp's index accounts for any AddItem ops queued ahead of it.
    let added = ops
        .iter()
        .filter(|o| matches!(o, Op::AddItem { .. }))
        .count();
    ops.push(Op::AddItem {
        index: doc.items.len() + added,
        item: Box::new(ProjectItem::Composition(comp)),
    });

    // File it into the folder. The folder may have been created earlier in this
    // same batch (so it is absent from `doc`), in which case its children start
    // empty — matching `lumit-ui`'s `file_into_folder_op`.
    let mut children = doc
        .folder(folder_id)
        .map(|f| f.children.clone())
        .unwrap_or_default();
    children.push(comp_id);
    ops.push(Op::SetFolderChildren {
        folder: folder_id,
        children,
    });

    match bridge.store.commit(Op::Batch { ops }) {
        Ok(_) => snapshot(bridge),
        Err(e) => err_json(format!("new composition: {e}")),
    }
}

/// Add a footage item for `path` as one undo step — the same op `lumit-ui`'s
/// `import_paths` commits for each picked file. Footage has no auto-folder (only
/// solids and comps do), so this mirrors the egui frontend exactly: a single
/// [`Op::AddItem`] appended to the flat item list, and undo removes it.
///
/// # In plain terms
///
/// This records that a media file belongs to the project. It does not open,
/// decode, or thumbnail the file — that probing needs FFmpeg and arrives in a
/// later phase (F2). The item simply carries the file's path (its name, and the
/// on-disk location for this session), exactly as the egui frontend stores a
/// media reference the moment you import, before any probe has run.
fn import_footage(bridge: &mut Bridge, path: &str) -> String {
    if path.trim().is_empty() {
        return err_json("import footage: no path given");
    }
    let file = PathBuf::from(path);
    // The item's name is the file's own name, exactly as `import_paths` derives
    // it (falling back to "footage" for a path with no final component).
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "footage".into());
    let item = FootageItem {
        id: Uuid::now_v7(),
        name: name.clone(),
        media: MediaRef {
            // Import stores the bare name as the relative path; Save rebases it
            // against the project folder (K-173), just as the egui frontend does.
            relative_path: name,
            absolute_path: file.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    };
    let index = bridge.store.snapshot().items.len();
    match bridge.store.commit(Op::AddItem {
        index,
        item: Box::new(ProjectItem::Footage(item)),
    }) {
        Ok(_) => snapshot(bridge),
        Err(e) => err_json(format!("import footage: {e}")),
    }
}

fn undo(bridge: &mut Bridge) -> String {
    match bridge.store.undo() {
        Ok(_) => snapshot(bridge),
        Err(e) => err_json(format!("undo: {e}")),
    }
}

fn redo(bridge: &mut Bridge) -> String {
    match bridge.store.redo() {
        Ok(_) => snapshot(bridge),
        Err(e) => err_json(format!("redo: {e}")),
    }
}

/// `{"ok":false,"error":"…"}`. serde escapes any control character, so the
/// resulting string never carries an interior NUL and always makes a `CString`.
fn err_json(message: impl AsRef<str>) -> String {
    json!({ "ok": false, "error": message.as_ref() }).to_string()
}

// ---------------------------------------------------------------------------
// The C ABI. Every body runs inside `catch_unwind` and returns Rust-owned,
// NUL-terminated UTF-8 the caller frees with `lumit_bridge_free_string`.
// ---------------------------------------------------------------------------

/// Turn a reply string into a Rust-owned C string. serde JSON never contains an
/// interior NUL, so the fallback is only a belt-and-braces guard.
fn to_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => match CString::new(err_json("internal: reply contained a NUL byte")) {
            Ok(c) => c.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
    }
}

/// Run a reply-producing closure, converting any panic into an error reply so
/// nothing unwinds across the C boundary (docs/14-ENGINEERING-RULES: no panics
/// across FFI).
fn guard(f: impl FnOnce() -> String + std::panic::UnwindSafe) -> *mut c_char {
    let reply = std::panic::catch_unwind(f)
        .unwrap_or_else(|_| err_json("internal error: a panic was caught at the bridge boundary"));
    to_c_string(reply)
}

/// Decode a caller-supplied C string to an owned `String`. `None` when the
/// pointer is null or the bytes are not valid UTF-8.
///
/// # Safety
/// `ptr` must be null or a valid NUL-terminated C string that stays alive for
/// the duration of the call.
unsafe fn c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(str::to_owned)
}

/// `{"ok":true,"version":"…","abi":1}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_version() -> *mut c_char {
    guard(version)
}

/// Discard the current document and start an empty one. Returns a fresh
/// snapshot.
#[no_mangle]
pub extern "C" fn lumit_bridge_new_project() -> *mut c_char {
    guard(|| with_bridge(new_project))
}

/// Open a `.lum` project from `path`. Returns the loaded snapshot, or an error
/// reply if the file is missing, not a Lumit project, or too new.
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_open_project(path: *const c_char) -> *mut c_char {
    let path = match c_str_to_string(path) {
        Some(p) => p,
        None => {
            return to_c_string(err_json(
                "open project: the path was null or not valid UTF-8",
            ))
        }
    };
    guard(move || with_bridge(|b| open_project(b, &path)))
}

/// Save the project. An empty `path` saves to the loaded path (error if none).
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_save_project(path: *const c_char) -> *mut c_char {
    // A null pointer is treated as "save to the loaded path", the same as "".
    let path = c_str_to_string(path).unwrap_or_default();
    guard(move || with_bridge(|b| save_project(b, &path)))
}

/// The current document as `{"ok":true,"items":[…],"can_undo":…,"can_redo":…,"path":…}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_snapshot() -> *mut c_char {
    guard(|| with_bridge(|b| snapshot(b)))
}

/// Create a composition (filed in the Compositions folder, one undo step).
/// An empty `name` becomes "Comp N".
///
/// # Safety
/// `name` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_new_composition(name: *const c_char) -> *mut c_char {
    let name = c_str_to_string(name).unwrap_or_default();
    guard(move || with_bridge(|b| new_composition(b, &name)))
}

/// Add a footage item referencing the media file at `path` (one undo step). No
/// probing happens here — the item just carries the path (F2 adds probing and
/// thumbnails). An empty path returns a calm error reply.
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_import_footage(path: *const c_char) -> *mut c_char {
    let path = match c_str_to_string(path) {
        Some(p) => p,
        None => {
            return to_c_string(err_json(
                "import footage: the path was null or not valid UTF-8",
            ))
        }
    };
    guard(move || with_bridge(|b| import_footage(b, &path)))
}

/// Undo the last committed operation. A no-op (nothing to undo) still returns a
/// valid snapshot.
#[no_mangle]
pub extern "C" fn lumit_bridge_undo() -> *mut c_char {
    guard(|| with_bridge(undo))
}

/// Redo the last undone operation. A no-op still returns a valid snapshot.
#[no_mangle]
pub extern "C" fn lumit_bridge_redo() -> *mut c_char {
    guard(|| with_bridge(redo))
}

/// Free a string returned by any of the functions above. Passing null is safe
/// and does nothing; passing the same pointer twice is undefined, exactly as
/// with C's `free`.
///
/// # Safety
/// `s` must be null or a pointer returned by one of this crate's functions and
/// not yet freed.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_free_string(s: *mut c_char) {
    // Freeing cannot meaningfully panic, but the no-panic-across-FFI rule holds
    // uniformly, so this body is guarded too.
    let _ = std::panic::catch_unwind(|| {
        if !s.is_null() {
            drop(CString::from_raw(s));
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Parse a reply string, asserting it is well-formed JSON.
    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    #[test]
    fn version_parses_and_reports_the_abi() {
        let v = parse(&version());
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["abi"], json!(ABI_VERSION));
        assert_eq!(v["version"], json!(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn new_project_shows_an_empty_document() {
        let mut b = Bridge::new();
        let snap = parse(&new_project(&mut b));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(snap["items"], json!([]));
        assert_eq!(snap["can_undo"], json!(false));
        assert_eq!(snap["can_redo"], json!(false));
        assert_eq!(snap["path"], Value::Null);
    }

    #[test]
    fn new_composition_lists_it_nested_and_enables_undo() {
        let mut b = Bridge::new();
        let snap = parse(&new_composition(&mut b, "Intro"));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(snap["can_undo"], json!(true));
        // One root item: the Compositions folder, with the comp nested inside.
        let items = snap["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "the comp is filed under one root folder");
        assert_eq!(items[0]["kind"], json!("folder"));
        assert_eq!(items[0]["name"], json!("Compositions"));
        let children = items[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["kind"], json!("composition"));
        assert_eq!(children[0]["name"], json!("Intro"));
        assert!(
            children[0]["children"].as_array().unwrap().is_empty(),
            "a composition has no children"
        );
    }

    #[test]
    fn empty_name_defaults_to_numbered_comp() {
        let mut b = Bridge::new();
        new_composition(&mut b, "");
        let snap = parse(&new_composition(&mut b, ""));
        let folder = &snap["items"][0];
        let children = folder["children"].as_array().unwrap();
        let names: Vec<&str> = children
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"Comp 1"));
        assert!(names.contains(&"Comp 2"));
    }

    #[test]
    fn undo_removes_the_composition() {
        let mut b = Bridge::new();
        new_composition(&mut b, "Shot");
        let after_undo = parse(&undo(&mut b));
        assert_eq!(after_undo["items"], json!([]), "undo clears the batch");
        assert_eq!(after_undo["can_undo"], json!(false));
        assert_eq!(after_undo["can_redo"], json!(true));
        // Redo brings it back.
        let after_redo = parse(&redo(&mut b));
        assert_eq!(after_redo["items"].as_array().unwrap().len(), 1);
        assert_eq!(after_redo["can_undo"], json!(true));
    }

    #[test]
    fn save_then_open_round_trips_through_a_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge-round-trip.lum");
        let path_str = path.to_string_lossy().into_owned();

        let mut writer = Bridge::new();
        new_composition(&mut writer, "Saved comp");
        let saved = parse(&save_project(&mut writer, &path_str));
        assert_eq!(saved["ok"], json!(true));
        assert_eq!(saved["path"], json!(path_str));

        // A fresh bridge opens it and sees the same tree.
        let mut reader = Bridge::new();
        let opened = parse(&open_project(&mut reader, &path_str));
        assert_eq!(opened["ok"], json!(true));
        let folder = &opened["items"][0];
        assert_eq!(folder["children"][0]["name"], json!("Saved comp"));
        assert_eq!(opened["path"], json!(path_str));
    }

    #[test]
    fn save_without_a_path_is_a_calm_error() {
        let mut b = Bridge::new();
        new_composition(&mut b, "Unsaved");
        let reply = parse(&save_project(&mut b, ""));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("no path"));
    }

    #[test]
    fn opening_a_missing_file_returns_ok_false_without_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-here.lum");
        let mut b = Bridge::new();
        let reply = parse(&open_project(&mut b, &missing.to_string_lossy()));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .starts_with("open project:"));
        // The bridge is still usable afterwards.
        assert_eq!(parse(&snapshot(&b))["ok"], json!(true));
    }

    #[test]
    fn import_footage_lists_the_item_as_footage() {
        let mut b = Bridge::new();
        let snap = parse(&import_footage(&mut b, "/media/clips/shot.mp4"));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(snap["can_undo"], json!(true));
        // Footage has no auto-folder: it sits at the root, unnested.
        let items = snap["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "the footage sits at the document root");
        assert_eq!(items[0]["kind"], json!("footage"));
        assert_eq!(items[0]["name"], json!("shot.mp4"));
        assert!(
            items[0]["children"].as_array().unwrap().is_empty(),
            "footage has no children"
        );
    }

    #[test]
    fn undo_removes_the_imported_footage() {
        let mut b = Bridge::new();
        import_footage(&mut b, "/media/clips/shot.mp4");
        let after_undo = parse(&undo(&mut b));
        assert_eq!(after_undo["items"], json!([]), "undo clears the import");
        assert_eq!(after_undo["can_undo"], json!(false));
        assert_eq!(after_undo["can_redo"], json!(true));
    }

    #[test]
    fn import_footage_with_an_empty_path_is_a_calm_error() {
        let mut b = Bridge::new();
        let reply = parse(&import_footage(&mut b, "   "));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("no path"));
        // The bridge is still usable and unchanged afterwards.
        assert_eq!(parse(&snapshot(&b))["items"], json!([]));
    }

    #[test]
    fn exported_functions_return_freeable_strings() {
        // Drive the real C ABI: call, copy, free. A double-free or use of a
        // freed pointer would be caught by miri (not in CI) — this at least
        // exercises the alloc/free contract end to end.
        let ptr = lumit_bridge_version();
        assert!(!ptr.is_null());
        let copied = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        assert_eq!(parse(&copied)["ok"], json!(true));
        unsafe { lumit_bridge_free_string(ptr) };

        // The snapshot goes through the shared static state; it must be a valid
        // reply and free cleanly.
        let snap_ptr = lumit_bridge_snapshot();
        assert!(!snap_ptr.is_null());
        let snap = unsafe { CStr::from_ptr(snap_ptr) }
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(parse(&snap)["ok"], json!(true));
        unsafe { lumit_bridge_free_string(snap_ptr) };

        // Freeing null is a no-op.
        unsafe { lumit_bridge_free_string(std::ptr::null_mut()) };
    }
}
