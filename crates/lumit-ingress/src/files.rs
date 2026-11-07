//! Files, through one door.
//!
//! On a desktop each of these is the system's own call and nothing more. A
//! page has no file system, so the browser build keeps its files in memory
//! here instead, in one store the engine and the interface share: a file
//! picked in the page is put here, and a saved project is read back from here
//! to hand to the person. Nothing leaves the machine.
//!
//! Only the calls the browser build reaches come through here.

use std::io;
use std::path::Path;
use std::time::SystemTime;

/// A file open for reading.
#[cfg(not(target_arch = "wasm32"))]
pub type Reader = std::fs::File;

/// A file open for writing.
#[cfg(not(target_arch = "wasm32"))]
pub type Writer = std::fs::File;

#[cfg(not(target_arch = "wasm32"))]
pub fn open(path: &Path) -> io::Result<Reader> {
    std::fs::File::open(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn create(path: &Path) -> io::Result<Writer> {
    std::fs::File::create(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    std::fs::write(path, bytes)
}

/// Add `bytes` to the end of a file, making it if it isn't there, and wait
/// until they are on the disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_data()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn remove_file(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn remove_dir_all(path: &Path) -> io::Result<()> {
    std::fs::remove_dir_all(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn is_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn is_dir(path: &Path) -> bool {
    path.is_dir()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn len(path: &Path) -> io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn modified(path: &Path) -> io::Result<SystemTime> {
    std::fs::metadata(path)?.modified()
}

/// What is in the folder `path`, by name, each with whether it is a folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn list(path: &Path) -> io::Result<Vec<(String, bool)>> {
    std::fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            Ok((name, entry.file_type()?.is_dir()))
        })
        .collect()
}

/// This process's id, which a temporary file's name carries.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn process_id() -> u32 {
    std::process::id()
}

// ---------------------------------------------------------------------------
// The browser build's store
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[cfg(target_arch = "wasm32")]
struct Store {
    /// Every file, by its tidied path.
    files: BTreeMap<String, (Arc<[u8]>, SystemTime)>,
    /// Every folder made, and every folder a file has been written into.
    dirs: BTreeSet<String>,
    /// Every file written or deleted since [`take_changed`] was last asked.
    changed: BTreeSet<String>,
    /// Video and sound the page holds itself ([`hold`]).
    held: BTreeSet<String>,
}

#[cfg(target_arch = "wasm32")]
static STORE: Mutex<Store> = Mutex::new(Store {
    files: BTreeMap::new(),
    dirs: BTreeSet::new(),
    changed: BTreeSet::new(),
    held: BTreeSet::new(),
});

/// The store. The page has one thread, so nothing ever waits here.
#[cfg(target_arch = "wasm32")]
fn store() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `path` as the store names it: from the root, either slash, with `.` and
/// `..` worked out.
#[cfg(target_arch = "wasm32")]
fn key(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The folders above `key`, nearest first, down to the root.
#[cfg(target_arch = "wasm32")]
fn parents(key: &str) -> impl Iterator<Item = &str> {
    key.match_indices('/')
        .rev()
        .map(move |(at, _)| key.get(..at).filter(|dir| !dir.is_empty()).unwrap_or("/"))
}

/// Whether `key` is inside the folder `dir`, at any depth.
#[cfg(target_arch = "wasm32")]
fn inside(key: &str, dir: &str) -> bool {
    dir == "/" && key != "/"
        || key
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(target_arch = "wasm32")]
fn now() -> SystemTime {
    let since = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default();
    std::time::UNIX_EPOCH + since
}

#[cfg(target_arch = "wasm32")]
fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

#[cfg(target_arch = "wasm32")]
fn put(key: String, bytes: Vec<u8>) {
    let mut store = store();
    for dir in parents(&key) {
        if !store.dirs.insert(dir.to_owned()) {
            break;
        }
    }
    store.changed.insert(key.clone());
    store.files.insert(key, (bytes.into(), now()));
}

/// Every file written or deleted since this was last asked, which the page
/// keeps in the browser's own storage so it outlasts the page.
#[cfg(target_arch = "wasm32")]
pub fn take_changed() -> Vec<String> {
    std::mem::take(&mut store().changed).into_iter().collect()
}

/// A file open for reading.
#[cfg(target_arch = "wasm32")]
pub type Reader = io::Cursor<Arc<[u8]>>;

/// A file open for writing. What is written lands in the store at
/// [`Writer::sync_all`], or when it is dropped.
#[cfg(target_arch = "wasm32")]
pub struct Writer {
    key: String,
    bytes: io::Cursor<Vec<u8>>,
    unsaved: std::cell::Cell<bool>,
}

#[cfg(target_arch = "wasm32")]
impl Writer {
    pub fn sync_all(&self) -> io::Result<()> {
        put(self.key.clone(), self.bytes.get_ref().clone());
        self.unsaved.set(false);
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.unsaved.set(true);
        self.bytes.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
impl io::Seek for Writer {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.bytes.seek(pos)
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for Writer {
    fn drop(&mut self) {
        if self.unsaved.get() {
            put(self.key.clone(), std::mem::take(self.bytes.get_mut()));
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn open(path: &Path) -> io::Result<Reader> {
    let bytes = store()
        .files
        .get(&key(path))
        .map(|(bytes, _)| bytes.clone());
    bytes.map(io::Cursor::new).ok_or_else(not_found)
}

#[cfg(target_arch = "wasm32")]
pub fn create(path: &Path) -> io::Result<Writer> {
    let key = key(path);
    put(key.clone(), Vec::new());
    Ok(Writer {
        key,
        bytes: io::Cursor::new(Vec::new()),
        unsaved: std::cell::Cell::new(false),
    })
}

#[cfg(target_arch = "wasm32")]
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    Ok(open(path)?.into_inner().to_vec())
}

#[cfg(target_arch = "wasm32")]
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    put(key(path), bytes.to_vec());
    Ok(())
}

// ponytail: the whole file is copied on each append, which a journal of a
// few megabytes bears. A list of pieces per file if one ever grows past that.
#[cfg(target_arch = "wasm32")]
pub fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let key = key(path);
    let had = store().files.get(&key).map(|(had, _)| had.clone());
    let mut all = had.map(|had| had.to_vec()).unwrap_or_default();
    all.extend_from_slice(bytes);
    put(key, all);
    Ok(())
}

/// Moves a file, or a folder and everything in it.
#[cfg(target_arch = "wasm32")]
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (key(from), key(to));
    let held = store();
    let moving: Vec<String> = held
        .files
        .keys()
        .filter(|key| **key == from || inside(key, &from))
        .cloned()
        .collect();
    if moving.is_empty() && !held.dirs.contains(&from) {
        return Err(not_found());
    }
    let dirs: Vec<String> = held
        .dirs
        .iter()
        .filter(|dir| **dir == from || inside(dir, &from))
        .cloned()
        .collect();
    drop(held);
    for dir in dirs {
        store().dirs.remove(&dir);
        create_dir_all(Path::new(&format!(
            "{to}{}",
            dir.get(from.len()..).unwrap_or("")
        )))?;
    }
    for key in moving {
        let file = store().files.remove(&key);
        store().changed.insert(key.clone());
        if let Some((bytes, _)) = file {
            put(
                format!("{to}{}", key.get(from.len()..).unwrap_or("")),
                bytes.to_vec(),
            );
        }
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn remove_file(path: &Path) -> io::Result<()> {
    let key = key(path);
    let mut store = store();
    store.files.remove(&key).ok_or_else(not_found)?;
    store.changed.insert(key);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn remove_dir_all(path: &Path) -> io::Result<()> {
    let dir = key(path);
    let mut store = store();
    let gone: Vec<String> = store
        .files
        .keys()
        .filter(|key| inside(key, &dir))
        .cloned()
        .collect();
    for key in gone {
        store.files.remove(&key);
        store.changed.insert(key);
    }
    store.dirs.retain(|key| *key != dir && !inside(key, &dir));
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    let dir = key(path);
    let mut store = store();
    for parent in parents(&dir) {
        store.dirs.insert(parent.to_owned());
    }
    store.dirs.insert(dir);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn is_file(path: &Path) -> bool {
    let (key, store) = (key(path), store());
    store.files.contains_key(&key) || store.held.contains(&key)
}

/// Say a file is at `path` that the page holds itself, as it does video and
/// sound, which its own decoders read. Its bytes are never in here, but a
/// project finds it like any other file.
#[cfg(target_arch = "wasm32")]
pub fn hold(path: &Path) {
    store().held.insert(key(path));
}

#[cfg(target_arch = "wasm32")]
pub fn is_dir(path: &Path) -> bool {
    let dir = key(path);
    dir == "/" || store().dirs.contains(&dir)
}

#[cfg(target_arch = "wasm32")]
pub fn exists(path: &Path) -> bool {
    is_file(path) || is_dir(path)
}

#[cfg(target_arch = "wasm32")]
pub fn len(path: &Path) -> io::Result<u64> {
    let len = store().files.get(&key(path)).map(|(bytes, _)| bytes.len());
    len.map(|len| len as u64).ok_or_else(not_found)
}

#[cfg(target_arch = "wasm32")]
pub fn modified(path: &Path) -> io::Result<SystemTime> {
    let when = store().files.get(&key(path)).map(|(_, when)| *when);
    when.ok_or_else(not_found)
}

/// What is in the folder `path`, by name, each with whether it is a folder.
#[cfg(target_arch = "wasm32")]
pub fn list(path: &Path) -> io::Result<Vec<(String, bool)>> {
    let dir = key(path);
    let store = store();
    let child = |key: &String| {
        let rest = key.strip_prefix(&dir)?.trim_start_matches('/');
        (inside(key, &dir) && !rest.contains('/')).then(|| rest.to_owned())
    };
    let dirs = store.dirs.iter().filter_map(child).map(|name| (name, true));
    let files = store
        .files
        .keys()
        .filter_map(child)
        .map(|name| (name, false));
    Ok(dirs.chain(files).collect())
}

/// A page has no process of its own to name.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn process_id() -> u32 {
    0
}
