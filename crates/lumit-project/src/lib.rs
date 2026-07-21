//! The `.lum` project container, autosave, and the crash-recovery journal —
//! docs/10-FILE-FORMAT.md, Phase 0 scope (no thumbnails yet).

pub mod fixtures;

use lumit_core::model::{Fingerprint, MediaRef};
use lumit_core::ops::Op;
use lumit_core::Document;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

pub const FORMAT: &str = "lumit-project";
pub const SCHEMA_VERSION: &str = "0.1.0";
pub const MIN_READER: &str = "0.1.0";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Lumit project")]
    NotALumitProject,
    #[error("project needs Lumit {min_reader} or newer (file is schema {schema_version})")]
    TooNew {
        schema_version: String,
        min_reader: String,
    },
}

/// manifest.json — MUST be the archive's first entry and parse standalone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub schema_version: String,
    pub written_by: String,
    pub min_reader: String,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Manifest {
    fn current() -> Self {
        Self {
            format: FORMAT.into(),
            schema_version: SCHEMA_VERSION.into(),
            written_by: format!("lumit {}", env!("CARGO_PKG_VERSION")),
            min_reader: MIN_READER.into(),
            extra: serde_json::Map::new(),
        }
    }
}

fn semver_triple(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split('.').map(|p| p.parse::<u64>().ok());
    match (it.next(), it.next(), it.next()) {
        (Some(Some(a)), Some(Some(b)), Some(Some(c))) => Some((a, b, c)),
        _ => None,
    }
}

/// One schema migration (docs/10-FILE-FORMAT.md §1): an in-place transform of
/// the raw `project.json` value that upgrades a document from one schema version
/// to the next. Migrations operate on `serde_json::Value` — before the typed
/// `Document` exists — precisely so a shape that no longer deserialises can be
/// reshaped first.
struct Migration {
    /// The schema version this migration reads.
    from: &'static str,
    /// The schema version it produces.
    to: &'static str,
    /// The in-place transform.
    apply: fn(&mut serde_json::Value),
}

/// The ordered migration chain. Empty today: `0.1.0` is the first schema, so no
/// older document exists to upgrade. Each future schema bump appends one
/// `Migration` here (from the previous version to the new one); [`run_migrations`]
/// then walks a file up the chain to the current schema on open.
static MIGRATIONS: &[Migration] = &[];

/// Walk `value` (raw `project.json` at schema `version`) up `chain` to the
/// current schema, applying each migration whose `from` matches the running
/// version. Bounded by `chain.len()` steps and stops if a migration fails to
/// advance the version, so a malformed chain can never loop. Pure — the real
/// chain is [`MIGRATIONS`]; tests pass a synthetic one.
fn run_migrations(
    chain: &[Migration],
    mut value: serde_json::Value,
    mut version: (u64, u64, u64),
) -> serde_json::Value {
    for _ in 0..chain.len() {
        let Some(m) = chain
            .iter()
            .find(|m| semver_triple(m.from) == Some(version))
        else {
            break;
        };
        (m.apply)(&mut value);
        match semver_triple(m.to) {
            Some(next) if next != version => version = next,
            _ => break, // no forward progress — stop rather than spin
        }
    }
    value
}

/// Atomic save: temp file in the destination directory, fsync, rename over
/// the target (docs/10-FILE-FORMAT.md §4).
pub fn save(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        stem.unwrap_or_else(|| "project.lum".into()),
        std::process::id()
    ));

    let result = (|| -> Result<(), ProjectError> {
        let file = File::create(&tmp)?;
        let mut zip = ZipWriter::new(file);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        // Manifest MUST be the first entry.
        zip.start_file("manifest.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(&Manifest::current())?.as_bytes())?;
        zip.start_file("project.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(doc)?.as_bytes())?;
        let file = zip.finish()?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp); // best effort; the target is untouched
    }
    result
}

/// Open a `.lum` file. Unknown JSON fields survive via the model's `extra` maps.
pub fn open(path: &Path) -> Result<(Document, Manifest), ProjectError> {
    let mut zip = ZipArchive::new(File::open(path)?)?;

    let manifest: Manifest = {
        let mut entry = zip
            .by_name("manifest.json")
            .map_err(|_| ProjectError::NotALumitProject)?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    if manifest.format != FORMAT {
        return Err(ProjectError::NotALumitProject);
    }
    if let (Some(ours), Some(needs)) = (
        semver_triple(SCHEMA_VERSION),
        semver_triple(&manifest.min_reader),
    ) {
        if ours < needs {
            return Err(ProjectError::TooNew {
                schema_version: manifest.schema_version.clone(),
                min_reader: manifest.min_reader.clone(),
            });
        }
    }

    let doc: Document = {
        let mut entry = zip
            .by_name("project.json")
            .map_err(|_| ProjectError::NotALumitProject)?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        // A file at an older schema is migrated up before it is typed (docs/10
        // §1). A current-schema file takes the direct path unchanged, so nothing
        // routes through `Value` needlessly.
        match semver_triple(&manifest.schema_version) {
            Some(v) if manifest.schema_version != SCHEMA_VERSION && !MIGRATIONS.is_empty() => {
                let value = run_migrations(MIGRATIONS, serde_json::from_str(&s)?, v);
                serde_json::from_value(value)?
            }
            _ => serde_json::from_str(&s)?,
        }
    };
    Ok((doc, manifest))
}

/// Rotating autosaves beside the project: `<stem>.autosave-1.lum` is newest.
pub fn autosave(doc: &Document, project_path: &Path, keep: usize) -> Result<PathBuf, ProjectError> {
    let dir = project_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("autosaves");
    fs::create_dir_all(&dir)?;
    let stem = project_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let slot = |k: usize| dir.join(format!("{stem}.autosave-{k}.lum"));

    // Shift older copies up; the oldest falls off the end.
    let _ = fs::remove_file(slot(keep));
    for k in (1..keep).rev() {
        let _ = fs::rename(slot(k), slot(k + 1));
    }
    let newest = slot(1);
    save(doc, &newest)?;
    Ok(newest)
}

/// The newest autosave beside `project_path`, if any exists — the crash-recovery
/// dialogue's third option (docs/10-FILE-FORMAT.md §4: last save + journal, last
/// save, or an autosave). [`autosave`] rotates so slot 1 is always the newest, so
/// that is the one offered. `None` when no autosave has been written yet.
#[must_use]
pub fn latest_autosave(project_path: &Path) -> Option<PathBuf> {
    let dir = project_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("autosaves");
    let stem = project_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let slot1 = dir.join(format!("{stem}.autosave-1.lum"));
    slot1.is_file().then_some(slot1)
}

/// Where a document's sidecar journal lives (docs/10-FILE-FORMAT.md §3–4).
pub fn journal_path(doc_id: Uuid) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(
        dirs.cache_dir()
            .join(doc_id.to_string())
            .join("journal")
            .join("ops.jsonl"),
    )
}

/// Media frame-index cache directory (docs/10-FILE-FORMAT.md §3) — global,
/// keyed by content fingerprint, so shared across projects and machines-safe.
pub fn media_index_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.cache_dir().join("media-index"))
}

/// The user's effect-preset library directory (docs/07-UI-SPEC.md §7) — where
/// `.lumfx` presets saved from a layer's effect stack live, so the Effects &
/// Presets browser can list and apply them. Global (shared across projects),
/// in the platform's roaming app-data area beside the config. `None` only when
/// the platform has no home directory; callers create it lazily.
pub fn presets_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_dir().join("presets"))
}

/// Bytes hashed from each of the head and tail of a file for its fingerprint.
/// 64 KiB catches format headers, codec tables and trailing indexes cheaply;
/// files smaller than two samples are hashed whole (the windows would overlap).
const FINGERPRINT_SAMPLE: usize = 64 * 1024;

/// Compute a [`Fingerprint`] for the file at `path` (docs/10 §2): its size,
/// last-modified time, and a blake3 hash of `size ++ head ++ tail`. Reads at
/// most two [`FINGERPRINT_SAMPLE`] windows regardless of file size, so it stays
/// cheap for multi-gigabyte footage — the relink resolver (step 3) calls it to
/// recognise a moved file by content rather than path.
pub fn fingerprint_path(path: &Path) -> std::io::Result<Fingerprint> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    let size = meta.len();
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0);

    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let sample = FINGERPRINT_SAMPLE as u64;
    if size <= sample * 2 {
        // Small file: hash all of it (head and tail would overlap).
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        hasher.update(&buf);
    } else {
        let mut head = vec![0u8; FINGERPRINT_SAMPLE];
        file.read_exact(&mut head)?;
        hasher.update(&head);
        file.seek(SeekFrom::End(-(FINGERPRINT_SAMPLE as i64)))?;
        let mut tail = vec![0u8; FINGERPRINT_SAMPLE];
        file.read_exact(&mut tail)?;
        hasher.update(&tail);
    }
    Ok(Fingerprint {
        size,
        mtime_secs,
        head_tail_hash: hasher.finalize().to_hex().to_string(),
    })
}

/// Which step of the relink resolver found a media file (docs/10 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveStep {
    /// The project-relative path still points at the file (step 1, preferred).
    RelativePath,
    /// The last-known absolute path still points at the file (step 2).
    AbsolutePath,
    /// A content search by fingerprint found it at a new location (step 3).
    FingerprintSearch,
}

/// The outcome of resolving a [`MediaRef`] to a file on disk (docs/10 §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Found on disk: `path` is where, `how` is which step succeeded.
    Found { path: PathBuf, how: ResolveStep },
    /// No automatic step found it — the relink dialogue takes over. Never a
    /// blocking error (docs/10 §2 step 4).
    Missing,
}

/// Resolve a media reference to a file on disk (docs/10 §2): try the
/// project-relative path, then the last absolute path, then — if a fingerprint
/// is stored — a content search across `search_roots` and the project tree;
/// otherwise report [`Resolved::Missing`] for the relink dialogue to handle.
///
/// Steps 1 and 2 trust the path (a file being there is enough); step 3 matches
/// by content, so it recognises a file that was moved or renamed.
pub fn resolve_media(media: &MediaRef, project_dir: &Path, search_roots: &[PathBuf]) -> Resolved {
    let rel = project_dir.join(&media.relative_path);
    if rel.is_file() {
        return Resolved::Found {
            path: rel,
            how: ResolveStep::RelativePath,
        };
    }
    let abs = Path::new(&media.absolute_path);
    if abs.is_file() {
        return Resolved::Found {
            path: abs.to_path_buf(),
            how: ResolveStep::AbsolutePath,
        };
    }
    if let Some(fp) = &media.fingerprint {
        for root in search_roots
            .iter()
            .map(PathBuf::as_path)
            .chain([project_dir])
        {
            if let Some(hit) = search_by_fingerprint(root, fp) {
                return Resolved::Found {
                    path: hit,
                    how: ResolveStep::FingerprintSearch,
                };
            }
        }
    }
    Resolved::Missing
}

/// Walk `root` (files only, symlinks not followed, so no cycles) for a file
/// whose content fingerprint matches `fp`. Size is checked from cheap metadata
/// before any file is hashed. Returns the first match, or None.
fn search_by_fingerprint(root: &Path, fp: &Fingerprint) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                // Cheap size filter before the hash.
                if entry.metadata().map(|m| m.len()).ok() != Some(fp.size) {
                    continue;
                }
                let path = entry.path();
                if fingerprint_path(&path)
                    .map(|c| c.likely_same_content(fp))
                    .unwrap_or(false)
                {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// The directory remapping implied by one file moving from `old` to `new`,
/// used to relink siblings that moved the same way (docs/10 §2). Defined only
/// for a pure relocation — same file name, different directory; None for a
/// rename (a changed name cannot generalise to siblings) or a non-move.
#[must_use]
pub fn path_mapping(old: &Path, new: &Path) -> Option<(PathBuf, PathBuf)> {
    if old.file_name()? != new.file_name()? {
        return None;
    }
    let (old_dir, new_dir) = (old.parent()?, new.parent()?);
    if old_dir == new_dir {
        return None;
    }
    Some((old_dir.to_path_buf(), new_dir.to_path_buf()))
}

/// Apply a [`path_mapping`] to a sibling's old path: if it lived under the
/// mapping's old directory, return where it now lives under the new one. None
/// when the sibling was elsewhere (the mapping does not apply to it).
#[must_use]
pub fn apply_mapping(mapping: &(PathBuf, PathBuf), sibling_old: &Path) -> Option<PathBuf> {
    let (from, to) = mapping;
    sibling_old
        .strip_prefix(from)
        .ok()
        .map(|rest| to.join(rest))
}

/// The result of [`collect_for_sharing`].
pub struct Collected {
    /// The document with every located reference rewritten to the collected
    /// copy under `media/`. The caller saves this into the destination folder.
    pub doc: Document,
    /// Names of footage items whose media could not be located, left referenced
    /// as-is so the shared project still opens (they show the relink slate).
    pub missing: Vec<String>,
}

/// Pick a name not already in `used`, appending `-1`, `-2`, … before the
/// extension on a collision. Records the chosen name in `used`.
fn unique_name(base: &str, used: &mut std::collections::HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let p = Path::new(base);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p.extension().map(|e| e.to_string_lossy().into_owned());
    let mut i = 1u32;
    loop {
        let cand = match &ext {
            Some(e) => format!("{stem}-{i}.{e}"),
            None => format!("{stem}-{i}"),
        };
        if used.insert(cand.clone()) {
            return cand;
        }
        i += 1;
    }
}

/// Copy the project's referenced media into `dest_dir/media/` and return a
/// document whose references point there, project-relative — the mechanism
/// behind sharing a project (K-065, docs/10 §2). `source_dir` is the current
/// project folder, used to locate each file with the same resolver `open` uses.
///
/// Nothing machine-specific survives: both the relative and the former absolute
/// path of each reference become the collected `media/<name>` path, and colliding
/// file names are disambiguated. Files that cannot be located are left as-is and
/// listed in [`Collected::missing`], so a partial collect still opens. The
/// existing fingerprint is preserved (a copy has the same content). The caller
/// writes the returned document into `dest_dir`.
pub fn collect_for_sharing(
    doc: &Document,
    source_dir: &Path,
    dest_dir: &Path,
) -> Result<Collected, ProjectError> {
    let media_dir = dest_dir.join("media");
    fs::create_dir_all(&media_dir)?;
    let mut out = doc.clone();
    let mut used = std::collections::HashSet::new();
    let mut missing = Vec::new();
    for item in &mut out.items {
        let lumit_core::model::ProjectItem::Footage(f) = item else {
            continue;
        };
        match resolve_media(&f.media, source_dir, &[]) {
            Resolved::Found { path, .. } => {
                let base = Path::new(&f.media.relative_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("{}.bin", f.id));
                let name = unique_name(&base, &mut used);
                fs::copy(&path, media_dir.join(&name))?;
                let rel = format!("media/{name}");
                f.media.absolute_path.clone_from(&rel);
                f.media.relative_path = rel;
            }
            Resolved::Missing => missing.push(f.name.clone()),
        }
    }
    Ok(Collected { doc: out, missing })
}

/// Append-only op log between saves; truncated on successful save.
pub struct JournalFile {
    path: PathBuf,
}

impl JournalFile {
    pub fn for_document(doc_id: Uuid) -> Option<Self> {
        journal_path(doc_id).map(|path| Self { path })
    }

    pub fn at_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn append(&self, op: &Op) -> Result<(), ProjectError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_string(op)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;
        f.sync_data()?;
        Ok(())
    }

    /// Read every replayable op. A torn final line (crash mid-append) is
    /// tolerated and dropped; a malformed line mid-file stops the replay there
    /// (later ops may depend on the lost one).
    pub fn read(&self) -> Result<Vec<Op>, ProjectError> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut ops = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(op) => ops.push(op),
                Err(_) => break,
            }
        }
        Ok(ops)
    }

    pub fn clear(&self) -> Result<(), ProjectError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::{FootageItem, MediaRef, ProjectItem};
    use lumit_core::ops::apply;

    fn footage(name: &str) -> FootageItem {
        FootageItem {
            id: Uuid::now_v7(),
            name: name.into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: format!("footage/{name}"),
                absolute_path: format!("/tmp/{name}"),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
        }
    }

    fn doc_with_item() -> Document {
        let mut doc = Document::new();
        let op = Op::AddItem {
            index: 0,
            item: Box::new(ProjectItem::Footage(footage("capture.mp4"))),
        };
        apply(&mut doc, &op).unwrap();
        doc
    }

    /// docs/10 §2: the fingerprint is stable, matches a byte-identical copy by
    /// content (mtime aside), and detects a change in either sampled window or a
    /// size change — the properties relink step 3 depends on.
    #[test]
    fn fingerprint_is_stable_and_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        // Larger than two sample windows, to exercise the head+tail path.
        let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let a = dir.path().join("a.bin");
        fs::write(&a, &data).unwrap();

        let f1 = fingerprint_path(&a).unwrap();
        let f2 = fingerprint_path(&a).unwrap();
        assert_eq!(f1.head_tail_hash, f2.head_tail_hash, "stable across calls");
        assert_eq!(f1.size, data.len() as u64);

        // A byte-identical copy at a new path matches by content.
        let moved = dir.path().join("moved.bin");
        fs::write(&moved, &data).unwrap();
        assert!(f1.likely_same_content(&fingerprint_path(&moved).unwrap()));

        // A change in the head window is detected.
        let mut head_changed = data.clone();
        head_changed[0] ^= 0xFF;
        let c = dir.path().join("head.bin");
        fs::write(&c, &head_changed).unwrap();
        assert!(!f1.likely_same_content(&fingerprint_path(&c).unwrap()));

        // A change in the tail window is detected.
        let mut tail_changed = data.clone();
        *tail_changed.last_mut().unwrap() ^= 0xFF;
        let d = dir.path().join("tail.bin");
        fs::write(&d, &tail_changed).unwrap();
        assert!(!f1.likely_same_content(&fingerprint_path(&d).unwrap()));

        // A different size never matches.
        let e = dir.path().join("short.bin");
        fs::write(&e, &data[..data.len() - 1]).unwrap();
        assert!(!f1.likely_same_content(&fingerprint_path(&e).unwrap()));
    }

    /// Files smaller than two sample windows are hashed whole and still compare
    /// by content.
    #[test]
    fn fingerprint_handles_small_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tiny.bin");
        fs::write(&p, b"hello").unwrap();
        let f = fingerprint_path(&p).unwrap();
        assert_eq!(f.size, 5);

        let same = dir.path().join("tiny2.bin");
        fs::write(&same, b"hello").unwrap();
        assert!(f.likely_same_content(&fingerprint_path(&same).unwrap()));

        let diff = dir.path().join("tiny3.bin");
        fs::write(&diff, b"world").unwrap();
        assert!(!f.likely_same_content(&fingerprint_path(&diff).unwrap()));
    }

    fn media_ref(rel: &str, abs: &str, fp: Option<Fingerprint>) -> lumit_core::model::MediaRef {
        lumit_core::model::MediaRef {
            relative_path: rel.into(),
            absolute_path: abs.into(),
            fingerprint: fp,
            extra: serde_json::Map::new(),
        }
    }

    /// docs/10 §2 step 1: the project-relative path wins when it still resolves.
    #[test]
    fn resolve_prefers_the_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        fs::create_dir_all(project.join("footage")).unwrap();
        let file = project.join("footage/clip.bin");
        fs::write(&file, b"data").unwrap();
        let m = media_ref("footage/clip.bin", "/nope/clip.bin", None);
        assert_eq!(
            resolve_media(&m, &project, &[]),
            Resolved::Found {
                path: file,
                how: ResolveStep::RelativePath
            }
        );
    }

    /// docs/10 §2 step 2: fall back to the last absolute path.
    #[test]
    fn resolve_falls_back_to_the_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        let file = dir.path().join("kept.bin");
        fs::write(&file, b"data").unwrap();
        let m = media_ref("footage/missing.bin", file.to_str().unwrap(), None);
        assert_eq!(
            resolve_media(&m, &project, &[]),
            Resolved::Found {
                path: file,
                how: ResolveStep::AbsolutePath
            }
        );
    }

    /// docs/10 §2 step 3: neither path resolves, but a fingerprint search finds
    /// the file — moved and renamed — under a search root.
    #[test]
    fn resolve_finds_a_moved_file_by_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        let elsewhere = dir.path().join("elsewhere/deep");
        fs::create_dir_all(&elsewhere).unwrap();
        let data: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
        let moved = elsewhere.join("renamed.bin");
        fs::write(&moved, &data).unwrap();
        let fp = fingerprint_path(&moved).unwrap();
        let m = media_ref("footage/clip.bin", "/nope/clip.bin", Some(fp));
        assert_eq!(
            resolve_media(&m, &project, &[dir.path().join("elsewhere")]),
            Resolved::Found {
                path: moved,
                how: ResolveStep::FingerprintSearch
            }
        );
    }

    /// docs/10 §2 step 4: nothing matches → Missing (never an error).
    #[test]
    fn resolve_reports_missing_when_nothing_matches() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        // Fingerprint of some content, but no matching file anywhere searched.
        let orphan = dir.path().join("orphan.bin");
        fs::write(&orphan, b"only here, not under a search root").unwrap();
        let fp = fingerprint_path(&orphan).unwrap();
        fs::remove_file(&orphan).unwrap();
        let m = media_ref("footage/x.bin", "/nope/x.bin", Some(fp));
        assert_eq!(
            resolve_media(&m, &project, std::slice::from_ref(&project)),
            Resolved::Missing
        );
    }

    /// docs/10 §2 sibling relink: a pure directory move yields a mapping that
    /// relocates siblings; a rename or non-move yields none.
    #[test]
    fn path_mapping_relinks_siblings_under_the_same_move() {
        let old = Path::new("/a/b/clip.mp4");
        let new = Path::new("/x/y/clip.mp4");
        let mapping = path_mapping(old, new).expect("a pure move maps");
        assert_eq!(
            apply_mapping(&mapping, Path::new("/a/b/other.wav")),
            Some(PathBuf::from("/x/y/other.wav")),
            "a sibling in the same folder relinks"
        );
        assert_eq!(
            apply_mapping(&mapping, Path::new("/a/b/sub/deep.mov")),
            Some(PathBuf::from("/x/y/sub/deep.mov")),
            "a sibling in a subfolder relinks under the mapping"
        );
        assert_eq!(
            apply_mapping(&mapping, Path::new("/z/elsewhere.mp4")),
            None,
            "a sibling outside the moved folder does not relink"
        );
        // A rename (different file name) does not generalise to siblings.
        assert_eq!(
            path_mapping(Path::new("/a/b/clip.mp4"), Path::new("/x/y/renamed.mp4")),
            None
        );
        // No move (same directory) yields no mapping.
        assert_eq!(
            path_mapping(Path::new("/a/b/clip.mp4"), Path::new("/a/b/clip.mp4")),
            None
        );
    }

    fn footage_item(name: &str, rel: &str, abs: &str) -> lumit_core::model::ProjectItem {
        lumit_core::model::ProjectItem::Footage(lumit_core::model::FootageItem {
            id: Uuid::now_v7(),
            name: name.into(),
            media: media_ref(rel, abs, None),
            extra: serde_json::Map::new(),
        })
    }

    fn media_of(item: &lumit_core::model::ProjectItem) -> &MediaRef {
        match item {
            lumit_core::model::ProjectItem::Footage(f) => &f.media,
            _ => panic!("expected footage"),
        }
    }

    /// docs/10 §2 / K-065: collect copies referenced media into `dest/media/`
    /// and rewrites the reference project-relative, with nothing machine-specific.
    #[test]
    fn collect_copies_media_and_rewrites_refs() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let real = dir.path().join("assets/clip.mp4");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, b"video-bytes").unwrap();

        let mut doc = Document::new();
        doc.items.push(footage_item(
            "Clip",
            "footage/clip.mp4",
            real.to_str().unwrap(),
        ));
        let dest = dir.path().join("share");
        let collected = collect_for_sharing(&doc, &src, &dest).unwrap();

        assert!(collected.missing.is_empty());
        let copied = dest.join("media/clip.mp4");
        assert!(copied.is_file(), "media copied into the share folder");
        assert_eq!(fs::read(&copied).unwrap(), b"video-bytes");
        let m = media_of(&collected.doc.items[0]);
        assert_eq!(m.relative_path, "media/clip.mp4");
        assert_eq!(
            m.absolute_path, "media/clip.mp4",
            "no machine-specific absolute path is written"
        );
    }

    /// Two references to files that share a basename get distinct collected
    /// names, so neither overwrites the other.
    #[test]
    fn collect_dedupes_colliding_names() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let a = dir.path().join("d1/clip.mp4");
        let b = dir.path().join("d2/clip.mp4");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::write(&a, b"AAA").unwrap();
        fs::write(&b, b"BBB").unwrap();

        let mut doc = Document::new();
        doc.items
            .push(footage_item("One", "footage/clip.mp4", a.to_str().unwrap()));
        doc.items
            .push(footage_item("Two", "footage/clip.mp4", b.to_str().unwrap()));
        let dest = dir.path().join("share");
        let collected = collect_for_sharing(&doc, &src, &dest).unwrap();

        assert_eq!(
            media_of(&collected.doc.items[0]).relative_path,
            "media/clip.mp4"
        );
        assert_eq!(
            media_of(&collected.doc.items[1]).relative_path,
            "media/clip-1.mp4"
        );
        assert_eq!(fs::read(dest.join("media/clip.mp4")).unwrap(), b"AAA");
        assert_eq!(fs::read(dest.join("media/clip-1.mp4")).unwrap(), b"BBB");
    }

    /// A reference that resolves nowhere is reported and left untouched, so the
    /// shared project still opens (missing media shows the relink slate).
    #[test]
    fn collect_reports_missing_media() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let mut doc = Document::new();
        doc.items.push(footage_item(
            "Ghost",
            "footage/ghost.mp4",
            "/nope/ghost.mp4",
        ));
        let dest = dir.path().join("share");
        let collected = collect_for_sharing(&doc, &src, &dest).unwrap();

        assert_eq!(collected.missing, vec!["Ghost".to_string()]);
        assert_eq!(
            media_of(&collected.doc.items[0]).relative_path,
            "footage/ghost.mp4",
            "an unlocatable reference is left unchanged"
        );
    }

    fn add_a(v: &mut serde_json::Value) {
        if let Some(o) = v.as_object_mut() {
            o.insert("a".into(), serde_json::json!(1));
        }
    }
    fn add_b(v: &mut serde_json::Value) {
        if let Some(o) = v.as_object_mut() {
            o.insert("b".into(), serde_json::json!(2));
        }
    }
    fn bump_n(v: &mut serde_json::Value) {
        if let Some(o) = v.as_object_mut() {
            let n = o.get("n").and_then(serde_json::Value::as_i64).unwrap_or(0);
            o.insert("n".into(), serde_json::json!(n + 1));
        }
    }

    /// An empty chain (today's real [`MIGRATIONS`]) is a no-op.
    #[test]
    fn no_migrations_leaves_json_unchanged() {
        let v = serde_json::json!({ "x": 5 });
        assert_eq!(run_migrations(&[], v.clone(), (0, 1, 0)), v);
        assert_eq!(run_migrations(MIGRATIONS, v.clone(), (0, 1, 0)), v);
    }

    /// docs/10 §1: a file is walked up the chain from its own version — earlier
    /// migrations are skipped, and every step from the file version onward runs
    /// in order.
    #[test]
    fn migrations_apply_in_order_from_the_file_version() {
        let chain = [
            Migration {
                from: "0.1.0",
                to: "0.2.0",
                apply: add_a,
            },
            Migration {
                from: "0.2.0",
                to: "0.3.0",
                apply: add_b,
            },
        ];
        // From the oldest version: both steps run.
        assert_eq!(
            run_migrations(&chain, serde_json::json!({}), (0, 1, 0)),
            serde_json::json!({ "a": 1, "b": 2 })
        );
        // From the middle version: only the later step runs.
        assert_eq!(
            run_migrations(&chain, serde_json::json!({}), (0, 2, 0)),
            serde_json::json!({ "b": 2 })
        );
    }

    /// A malformed chain whose migration does not advance the version applies
    /// once and stops, rather than looping forever.
    #[test]
    fn a_non_advancing_migration_does_not_loop() {
        let chain = [Migration {
            from: "0.1.0",
            to: "0.1.0",
            apply: bump_n,
        }];
        assert_eq!(
            run_migrations(&chain, serde_json::json!({}), (0, 1, 0)),
            serde_json::json!({ "n": 1 }),
            "applied exactly once, then stopped"
        );
    }

    /// A MediaRef with no fingerprint serialises without the field, so projects
    /// saved before fingerprints round-trip byte-for-byte (docs/10 §1.1).
    #[test]
    fn absent_fingerprint_is_not_serialised() {
        let m = lumit_core::model::MediaRef {
            relative_path: "footage/x.mp4".into(),
            absolute_path: "/tmp/x.mp4".into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("fingerprint"),
            "unset fingerprint must not appear in the file: {json}"
        );
        let back: lumit_core::model::MediaRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn save_open_round_trip_and_no_temp_litter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.lum");
        let doc = doc_with_item();
        save(&doc, &path).unwrap();
        let (loaded, manifest) = open(&path).unwrap();
        assert_eq!(loaded, doc);
        assert_eq!(manifest.format, FORMAT);
        save(&doc, &path).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn manifest_is_first_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.lum");
        save(&doc_with_item(), &path).unwrap();
        let mut zip = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.by_index(0).unwrap().name(), "manifest.json");
    }

    #[test]
    fn unknown_fields_survive_open_save_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.lum");
        save(&doc_with_item(), &path).unwrap();

        // A "newer Lumit" adds fields this version knows nothing about.
        let (mut doc, _) = open(&path).unwrap();
        doc.extra
            .insert("from_the_future".into(), serde_json::json!({"keep": true}));
        if let ProjectItem::Footage(f) = &mut doc.items[0] {
            f.extra
                .insert("colour_tag".into(), serde_json::json!("rec709"));
        }
        let path2 = dir.path().join("edit2.lum");
        save(&doc, &path2).unwrap();

        let (again, _) = open(&path2).unwrap();
        assert_eq!(
            again.extra["from_the_future"]["keep"],
            serde_json::json!(true)
        );
        match &again.items[0] {
            ProjectItem::Footage(f) => {
                assert_eq!(f.extra["colour_tag"], serde_json::json!("rec709"));
            }
            other => panic!("footage item expected, got {other:?}"),
        }
    }

    /// Reads one entry's bytes out of a `.lum` container.
    fn entry_bytes(path: &Path, name: &str) -> Vec<u8> {
        let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut entry = zip.by_name(name).unwrap();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        buf
    }

    #[test]
    fn two_saves_of_the_same_doc_are_byte_identical(/* docs/10 §1 */) {
        // Insert several out-of-order unknown keys: the serialised order must be
        // stable (serde_json::Map is a sorted BTreeMap without preserve_order),
        // so re-saving the same document reproduces the same project.json bytes.
        let mut doc = doc_with_item();
        doc.extra.insert("zebra".into(), serde_json::json!(1));
        doc.extra.insert("alpha".into(), serde_json::json!(2));
        doc.extra.insert("mike".into(), serde_json::json!(3));

        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.lum");
        let b = dir.path().join("b.lum");
        save(&doc, &a).unwrap();
        save(&doc, &b).unwrap();

        let ja = entry_bytes(&a, "project.json");
        let jb = entry_bytes(&b, "project.json");
        assert_eq!(
            ja, jb,
            "two saves of the same document must be byte-identical"
        );

        // And a round-trip (open then save) reproduces those exact bytes, so
        // unknown-field preservation is deterministic too.
        let (reopened, _) = open(&a).unwrap();
        let c = dir.path().join("c.lum");
        save(&reopened, &c).unwrap();
        assert_eq!(
            ja,
            entry_bytes(&c, "project.json"),
            "open+save must be stable"
        );
    }

    #[test]
    fn too_new_projects_are_refused_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.lum");
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.json", opts).unwrap();
        zip.write_all(
            br#"{"format":"lumit-project","schema_version":"9.0.0","written_by":"lumit 9","min_reader":"9.0.0"}"#,
        )
        .unwrap();
        zip.start_file("project.json", opts).unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
        match open(&path) {
            Err(ProjectError::TooNew { min_reader, .. }) => {
                assert_eq!(min_reader, "9.0.0");
            }
            other => panic!("expected TooNew, got {other:?}"),
        }
    }

    #[test]
    fn autosave_rotates_and_keeps_n() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("edit.lum");
        for i in 0..7u32 {
            let mut doc = Document::new();
            doc.extra.insert("gen".into(), serde_json::json!(i));
            autosave(&doc, &project, 5).unwrap();
        }
        let autos = dir.path().join("autosaves");
        assert_eq!(fs::read_dir(&autos).unwrap().count(), 5);
        let (newest, _) = open(&autos.join("edit.autosave-1.lum")).unwrap();
        assert_eq!(newest.extra["gen"], serde_json::json!(6));
        let (oldest, _) = open(&autos.join("edit.autosave-5.lum")).unwrap();
        assert_eq!(oldest.extra["gen"], serde_json::json!(2));
    }

    #[test]
    fn latest_autosave_finds_the_newest_or_none() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("edit.lum");
        // Nothing written yet.
        assert!(latest_autosave(&project).is_none());
        // After an autosave, slot 1 (the newest) is offered.
        let mut doc = Document::new();
        doc.extra.insert("gen".into(), serde_json::json!(42));
        autosave(&doc, &project, 5).unwrap();
        let found = latest_autosave(&project).expect("an autosave now exists");
        assert_eq!(
            found,
            dir.path().join("autosaves").join("edit.autosave-1.lum")
        );
        let (loaded, _) = open(&found).unwrap();
        assert_eq!(loaded.extra["gen"], serde_json::json!(42));
    }

    #[test]
    fn journal_appends_reads_and_tolerates_torn_tail() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JournalFile::at_path(dir.path().join("ops.jsonl"));
        let mut doc = Document::new();
        let doc0 = doc.clone();

        let item = ProjectItem::Footage(footage("a.mp4"));
        let ops = vec![
            Op::AddItem {
                index: 0,
                item: Box::new(item.clone()),
            },
            Op::RenameItem {
                id: item.id(),
                name: "hero".into(),
            },
        ];
        for op in &ops {
            apply(&mut doc, op).unwrap();
            journal.append(op).unwrap();
        }
        // simulate a crash mid-append
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.path().join("ops.jsonl"))
            .unwrap();
        f.write_all(b"{\"RenameItem\":{\"id\":\"trunc").unwrap();

        let mut replayed = doc0;
        for op in journal.read().unwrap() {
            apply(&mut replayed, &op).unwrap();
        }
        assert_eq!(
            serde_json::to_string(&replayed).unwrap(),
            serde_json::to_string(&doc).unwrap()
        );
        journal.clear().unwrap();
        assert!(journal.read().unwrap().is_empty());
    }
}
