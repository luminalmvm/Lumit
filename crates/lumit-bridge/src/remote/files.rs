//! A remote export's files: which ones a composition needs, what each is by
//! the hash of what is in it, and the worker's store of them.
//!
//! A file is known by its whole blake3 hash, never by its name or where it
//! sits, so a file the worker already has is never sent, whoever named it
//! what. Hashing a long file takes a while, so each is hashed once and the
//! answer kept by where it is, how big and when it last changed.
//!
//! Runs on the remote IO thread, and nothing here is asked of the UI thread.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use lumit_core::model::{Document, ProjectItem};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a file is to the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Kind {
    Footage,
    Proxy,
    /// The colour config and its tables.
    Config,
    /// A file an effect reads, such as a LUT.
    Effect,
    /// An analysis the export reads, filed under `id`.
    Analysis,
    /// A simulation baked for the effect `id`.
    Bake,
}

/// One file of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Entry {
    pub hash: String,
    pub size: u64,
    pub kind: Kind,
    /// The reference it belongs to: the id the project packs it under, the
    /// analysis, or the baked effect.
    pub id: Uuid,
    /// Where it sits from its reference's first file's folder, with `/`
    /// between folders. The first file is listed first.
    pub path: String,
    /// Where it is on the sender.
    pub from: String,
}

/// Every file a job needs, besides the document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub files: Vec<Entry>,
}

/// The most a manifest may be: a few hundred thousand files.
pub(super) const MANIFEST_MOST: u64 = 256 << 20;

/// The most a document may be, as `lumit_project` reads one.
pub(super) const DOCUMENT_MOST: u64 = 512 << 20;

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(super) fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// What a file with this hash is asked for as: the first half of the hash.
pub(super) fn item(hash: &str) -> Uuid {
    let half = hash
        .get(..32)
        .and_then(|half| u128::from_str_radix(half, 16).ok());
    Uuid::from_u128(half.unwrap_or(0))
}

/// Whether `hash` is a hash at all, which one from another computer has to
/// be before it names a folder here.
pub(super) fn is_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A path another computer gave, as one that stays inside the folder it is
/// joined to. `None` for anything that would leave it.
pub(super) fn inside(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return None;
        }
        out.push(part);
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// A path another computer gave, as a plain absolute path on one of this
/// computer's own disks. `None` for anything else: relative, a share on
/// another machine, which Windows would sign in to just to look, or a
/// device path.
pub(super) fn local_path(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path);
    let mut parts = path.components();
    #[cfg(windows)]
    {
        let Some(Component::Prefix(prefix)) = parts.next() else {
            return None;
        };
        if !matches!(prefix.kind(), std::path::Prefix::Disk(_)) {
            return None;
        }
    }
    let plain = path.is_absolute() && !parts.any(|part| part == Component::ParentDir);
    plain.then_some(path)
}

/// The file name of `path`, as a file in the store is called.
pub(super) fn name_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    if inside(name).is_some() {
        name.to_owned()
    } else {
        "file".to_owned()
    }
}

/// Hash the file at `path` whole, saying how far it has got. `None` when it
/// can't be read, or `stop` says to give up.
pub(super) fn hash_file(
    path: &Path,
    stop: &dyn Fn() -> bool,
    done: &mut dyn FnMut(u64),
) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut piece = vec![0u8; 1 << 20];
    loop {
        if stop() {
            return None;
        }
        let n = file.read(&mut piece).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&piece[..n]);
        done(n as u64);
    }
    Some(hex(hasher.finalize().as_bytes()))
}

/// Hashes already worked out, by where the file is.
///
/// Bounded: past [`HASHES_MOST`] entries the oldest go as the next are added.
pub(super) struct Hashes {
    file: PathBuf,
    known: Mutex<(HashMap<String, Known>, u64, bool)>,
}

/// A file as it was when it was hashed: its size, when it last changed, its
/// hash, and when it was hashed or asked about, in the order of asking.
#[derive(Clone, Serialize, Deserialize)]
struct Known {
    size: u64,
    changed: u64,
    hash: String,
    asked: u64,
}

const HASHES_MOST: usize = 200_000;

/// When a file last changed, in nanoseconds, or nought when it can't say.
fn changed(meta: &std::fs::Metadata) -> u64 {
    let at = meta.modified().ok();
    let since = at.and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok());
    since.map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

impl Hashes {
    pub(super) fn open(file: PathBuf) -> Hashes {
        let read = std::fs::read(&file).ok();
        let known: HashMap<String, Known> = read
            .and_then(|text| serde_json::from_slice(&text).ok())
            .unwrap_or_default();
        let next = known.values().map(|k| k.asked).max().unwrap_or(0) + 1;
        Hashes {
            file,
            known: Mutex::new((known, next, false)),
        }
    }

    /// The hash of the file at `path`, hashing it only when it changed since.
    pub(super) fn of(
        &self,
        path: &Path,
        stop: &dyn Fn() -> bool,
        done: &mut dyn FnMut(u64),
    ) -> Option<String> {
        let meta = std::fs::metadata(path)
            .ok()
            .filter(std::fs::Metadata::is_file)?;
        let (size, at) = (meta.len(), changed(&meta));
        let key = path.to_string_lossy().into_owned();
        if let Ok(mut known) = self.known.lock() {
            let next = known.1;
            if let Some(kept) = known
                .0
                .get_mut(&key)
                .filter(|k| k.size == size && k.changed == at)
            {
                kept.asked = next;
                let hash = kept.hash.clone();
                known.1 += 1;
                done(size);
                return Some(hash);
            }
        }
        let hash = hash_file(path, stop, done)?;
        if let Ok(mut known) = self.known.lock() {
            let asked = known.1;
            known.1 += 1;
            if known.0.len() >= HASHES_MOST {
                let oldest = known
                    .0
                    .iter()
                    .min_by_key(|(_, k)| k.asked)
                    .map(|(p, _)| p.clone());
                if let Some(oldest) = oldest {
                    known.0.remove(&oldest);
                }
            }
            let kept = Known {
                size,
                changed: at,
                hash: hash.clone(),
                asked,
            };
            known.0.insert(key, kept);
            known.2 = true;
        }
        Some(hash)
    }

    /// Write what was learnt since the last time.
    pub(super) fn keep(&self) {
        let text = {
            let Ok(mut known) = self.known.lock() else {
                return;
            };
            if !known.2 {
                return;
            }
            known.2 = false;
            serde_json::to_vec(&known.0)
        };
        if let (Ok(text), Some(dir)) = (text, self.file.parent()) {
            let _ = std::fs::create_dir_all(dir);
            let making = self.file.with_extension("new");
            if std::fs::write(&making, text).is_ok() {
                let _ = std::fs::rename(&making, &self.file);
            }
        }
    }
}

/// A file of the set before it is hashed.
pub(super) struct Found {
    pub kind: Kind,
    pub id: Uuid,
    pub path: String,
    pub from: PathBuf,
}

/// The compositions `comp` shows, itself included.
pub(super) fn reached(doc: &Document, comp: Uuid) -> BTreeSet<Uuid> {
    let mut reached = BTreeSet::new();
    let mut next = vec![comp];
    while let Some(id) = next.pop() {
        let Some(found) = doc.comp(id) else {
            continue;
        };
        if reached.insert(id) {
            next.extend(crate::edits::nested_comps(doc, found));
        }
    }
    reached
}

/// `doc` with only what an export of `comp` reaches: its compositions and
/// the footage they show. The rest of the project is never read by it.
pub(super) fn trimmed(doc: &Document, comp: Uuid) -> Document {
    let reached = reached(doc, comp);
    let used: HashSet<Uuid> = doc
        .comp(comp)
        .map(|c| lumit_core::model::comp_footage_items(doc, c))
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut trimmed = doc.clone();
    trimmed.items.retain(|item| match item {
        ProjectItem::Footage(f) => used.contains(&f.id),
        ProjectItem::Composition(c) => reached.contains(&c.id),
        _ => true,
    });
    trimmed.proxies.retain(|item, _| used.contains(item));
    trimmed
}

/// Every file an export of `comp` reads: its footage with the rest of each
/// image sequence and each model's own files, proxies, the colour config and
/// its tables, the files its effects read, the analyses it reads and the
/// simulations baked for it under `bakes`. Files not on this computer are
/// left out, and the export does without them here too.
pub(super) fn needed(
    doc: &Document,
    comp: Uuid,
    project_dir: &Path,
    bakes: Option<&Path>,
) -> Vec<Found> {
    let doc = trimmed(doc, comp);
    let config = lumit_core::model::packed_id(doc.id);
    let mut found = Vec::new();
    for source in crate::packing::sources(&doc, project_dir, true) {
        let kind = match doc.item(source.item) {
            Some(ProjectItem::Footage(_)) => Kind::Footage,
            _ if source.item == config => Kind::Config,
            _ if doc.packed_ref(source.item).is_some() => Kind::Proxy,
            _ => Kind::Effect,
        };
        let Some(base) = source
            .files
            .first()
            .and_then(|f| f.parent())
            .map(Path::to_path_buf)
        else {
            continue;
        };
        for file in source.files {
            found.push(Found {
                kind,
                id: source.item,
                path: below(&base, &file),
                from: file,
            });
        }
    }
    for (id, path) in lumit_render::sent::files(&doc) {
        if path.is_file() {
            found.push(Found {
                kind: Kind::Analysis,
                id,
                path: name_of(&path.to_string_lossy()),
                from: path,
            });
        }
    }
    if let Some(bakes) = bakes {
        let comps = doc.items.iter().filter_map(|item| match item {
            ProjectItem::Composition(c) => Some(c),
            _ => None,
        });
        let effects = comps.flat_map(|c| c.effects()).map(|fx| fx.id);
        for effect in effects.collect::<BTreeSet<Uuid>>() {
            let dir = bakes.join(effect.to_string());
            for file in walk(&dir, 2) {
                found.push(Found {
                    kind: Kind::Bake,
                    id: effect,
                    path: below(&dir, &file),
                    from: file,
                });
            }
        }
    }
    found
}

/// `file` from `base`, with `/` between folders, or its name alone when it
/// is not below it.
fn below(base: &Path, file: &Path) -> String {
    let parts: Option<Vec<String>> = file.strip_prefix(base).ok().map(|rest| {
        rest.components()
            .filter_map(|part| match part {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    });
    match parts.filter(|parts| !parts.is_empty()) {
        Some(parts) => parts.join("/"),
        None => name_of(&file.to_string_lossy()),
    }
}

/// Every file in `dir` and up to `depth` folders below it.
pub(super) fn walk(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            out.push(entry.path());
        } else if kind.is_dir() && depth > 0 {
            out.extend(walk(&entry.path(), depth - 1));
        }
    }
    out
}

/// Where the store keeps the file `hash`, called `name`.
pub(super) fn stored(root: &Path, hash: &str, name: &str) -> PathBuf {
    root.join("files").join(hash).join(name_of(name))
}

/// The file the store has for `hash`, if it has it whole at `size`.
pub(super) fn in_store(root: &Path, hash: &str, size: u64) -> Option<PathBuf> {
    let dir = root.join("files").join(hash);
    let files = std::fs::read_dir(dir).ok()?;
    files.flatten().map(|f| f.path()).find(|path| {
        let whole = path.extension().is_none_or(|ext| ext != "part");
        whole && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == size)
    })
}

/// Put the checked file at `path`, the file `hash` of `size` bytes, in the
/// store as `name`. One already there, which another job took meanwhile, is
/// kept instead. False when it could not be put there.
pub(super) fn keep(root: &Path, hash: &str, size: u64, name: &str, path: &Path) -> bool {
    if let Some(there) = in_store(root, hash, size) {
        let _ = std::fs::remove_file(path);
        touch(&there);
        return true;
    }
    let to = stored(root, hash, name);
    let moved = to
        .parent()
        .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
        && std::fs::rename(path, &to).is_ok();
    if moved {
        touch(&to);
    }
    moved
}

/// Mark a stored file as just used, so trimming takes it last.
pub(super) fn touch(path: &Path) {
    let opened = std::fs::OpenOptions::new().append(true).open(path);
    if let Ok(file) = opened {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// Put `from` at `to` as well: a second name for it where the disk allows,
/// a copy where not.
pub(super) fn place(from: &Path, to: &Path) -> bool {
    if let Some(dir) = to.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(to);
    std::fs::hard_link(from, to).is_ok() || std::fs::copy(from, to).is_ok()
}

/// The most the store keeps: the lesser of this and a quarter of the room it
/// has, which is what it holds and what is free on its disk.
const STORE_MOST: u64 = 100 << 30;

/// What the store holds, in bytes.
pub(super) fn store_bytes(root: &Path) -> u64 {
    walk(&root.join("files"), 1)
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|m| m.len())
        .sum()
}

/// Delete the store's files, least recently used first, until it holds no
/// more than it may, or with `all` everything. Never what `keep` names.
pub(super) fn trim(root: &Path, keep: &HashSet<String>, all: bool) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = walk(&root.join("files"), 1)
        .into_iter()
        .filter_map(|file| {
            let meta = std::fs::metadata(&file).ok()?;
            Some((meta.modified().ok()?, meta.len(), file))
        })
        .collect();
    let mut held: u64 = files.iter().map(|f| f.1).sum();
    let free = crate::framecache::disk::free_bytes(root);
    let disk = free.map_or(u64::MAX, |free| held.saturating_add(free) / 4);
    let most = if all { 0 } else { STORE_MOST.min(disk) };
    files.sort_by_key(|f| f.0);
    for (_, size, file) in files {
        if held <= most {
            break;
        }
        let Some(dir) = file.parent() else {
            continue;
        };
        let hash = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        if hash.is_some_and(|hash| keep.contains(&hash)) {
            continue;
        }
        if std::fs::remove_file(&file).is_ok() {
            held = held.saturating_sub(size);
            let _ = std::fs::remove_dir(dir);
        }
    }
}
