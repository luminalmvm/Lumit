//! The `.lum` project container, autosave, and the crash-recovery journal —
//! docs/10-FILE-FORMAT.md, Phase 0 scope (no thumbnails yet).

pub mod fixtures;

use lumit_core::model::{Fingerprint, MediaRef, ProjectItem};
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
pub const SCHEMA_VERSION: &str = "0.2.0";
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

/// The ordered migration chain. Each schema bump appends one `Migration` here
/// (from the previous version to the new one); [`run_migrations`] then walks a
/// file up the chain to the current schema on open.
static MIGRATIONS: &[Migration] = &[Migration {
    from: "0.1.0",
    to: "0.2.0",
    apply: retime_onto_the_layer,
}];

/// `0.1.0` → `0.2.0` (K-249): a Footage layer's own retime segment store moves
/// onto the layer as the Retime **property**, and the frame-interpolation
/// policy moves out beside it.
///
/// Until K-249 a layer could be retimed two ways — the keyframable property on
/// the layer, and a rival segment store inside `kind.Footage`. One had to go,
/// and the property won, so a document written by the old build has its segment
/// store converted here, before it is ever typed: the store's own exact reader
/// turns it into the identical keyframes, which is why this is lossless for
/// every curve the old rows could actually author.
///
/// **The property wins if both are present.** A layer carrying each was already
/// evaluating the property alone (`source_time_at` preferred it), so keeping it
/// is what makes the file open looking the way it last rendered.
/// Lift the old segment store out of `owner["retime"]`, leaving nothing
/// readable behind.
///
/// `take` puts null in its place, which serde reads as the absent field the
/// new shape expects — and makes the store unreachable whatever happens next,
/// so it can never be read twice.
fn take_retime_store(owner: &mut serde_json::Value) -> Option<lumit_core::retime::Retime> {
    let old = owner.get_mut("retime").map(serde_json::Value::take)?;
    serde_json::from_value(old).ok() // unreadable: opens un-retimed, not wrong
}

/// Write `store` onto `dest` as the Retime **property**, with its
/// interpolation policy beside it.
///
/// **The property wins if there is already one.** A layer carrying each was
/// evaluating the property alone (`source_time_at` preferred it), so keeping
/// it is what makes the file open looking the way it last rendered. A clip
/// never had two, so the rule costs it nothing.
fn write_retime_property(
    dest: &mut serde_json::Map<String, serde_json::Value>,
    store: lumit_core::retime::Retime,
) {
    // The policy is not part of the map (docs/04 §10) and now lives beside it.
    // Carried across whether or not the map is; `or_insert` leaves a clip's
    // own policy, which it always had, exactly as written.
    if let Ok(policy) = serde_json::to_value(&store.interpolation) {
        dest.entry("interpolation").or_insert(policy);
    }
    if dest.get("retime").is_some_and(|r| !r.is_null()) {
        return;
    }
    // Built as a real `Property` and serialised, rather than as hand-written
    // JSON: the shape then follows the type, and a later change to either
    // cannot silently make this write a document the same build refuses to
    // read.
    let property = lumit_core::anim::Property {
        animation: lumit_core::anim::Animation::Keyframed(store.source_keyframes()),
        extra: serde_json::Map::new(),
    };
    if let Ok(v) = serde_json::to_value(property) {
        dest.insert("retime".into(), v);
    }
}

fn retime_onto_the_layer(value: &mut serde_json::Value) {
    let Some(comps) = value.get_mut("comps").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for comp in comps {
        let Some(layers) = comp.get_mut("layers").and_then(|l| l.as_array_mut()) else {
            continue;
        };
        for layer in layers {
            // A Sequence layer's clips carried the same segment store, and
            // move to the same property shape (K-249's second half).
            if let Some(clips) = layer
                .pointer_mut("/kind/Sequence/clips")
                .and_then(|c| c.as_array_mut())
            {
                for clip in clips {
                    let Some(store) = take_retime_store(clip) else {
                        continue;
                    };
                    if let Some(fields) = clip.as_object_mut() {
                        write_retime_property(fields, store);
                    }
                }
            }
            // Taken out of the layer's *kind* and written onto the layer
            // itself — the one place the two owners differ. Sequenced so the
            // take lands before the object is reached for, or writing the
            // property would put the old store back.
            let store = match layer.pointer_mut("/kind/Footage") {
                Some(footage) => take_retime_store(footage),
                None => None,
            };
            let Some(store) = store else {
                continue;
            };
            if let Some(fields) = layer.as_object_mut() {
                write_retime_property(fields, store);
            }
        }
    }
}

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
    let mut doc = doc;
    // Forward-migrate effect stacks (K-258): a built-in whose schema grew
    // since this file was saved gains the new parameters at their defaults,
    // so the panel has values to draw and edits have ids to write.
    for item in &mut doc.items {
        if let lumit_core::model::ProjectItem::Composition(comp) = item {
            for layer in &mut comp.layers {
                lumit_core::fx::backfill_builtin_params(&mut layer.effects);
            }
        }
    }

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

/// Where a document's parked frames live when the disk frame cache is kept in
/// the application's own data area rather than beside the project file
/// (docs/06-RENDER-PIPELINE.md §5.4, Settings → Performance → Cache).
///
/// In plain terms: the frames the cache parks have to go *somewhere*, and beside
/// the project file only works once the project HAS a file. Keyed by the
/// document's own id, which is written into the `.lum` and survives every save
/// and reopen, so a project caches from the moment it is created and still finds
/// its frames tomorrow.
///
/// The platform's own cache directory, through the same `ProjectDirs` call the
/// journal and the media index make, so there is one Lumit folder rather than
/// three:
///
/// | | |
/// |---|---|
/// | Windows | `%LOCALAPPDATA%\Lumit\Lumit\cache\frames\<id>\` |
/// | macOS | `~/Library/Caches/dev.Lumit.Lumit/frames/<id>/` |
/// | Linux | `$XDG_CACHE_HOME/lumit/frames/<id>/` (default `~/.cache/lumit`) |
///
/// On Windows that is **local** app data, never roaming: a roaming profile would
/// try to copy the cache to a network share at logoff, and this one can be tens
/// of gigabytes.
///
/// The *cache* directory, not the temp directory — temp is emptied on reboot, so
/// every project would come back cold. These survive a reboot, and the operating
/// system may reclaim them under disk pressure, which is exactly right for a
/// folder deletable at any time with no correctness effect.
///
/// `None` only when the platform has no home directory; the caller then runs
/// with no disk tier rather than failing.
pub fn frame_cache_dir(doc_id: Uuid) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.cache_dir().join("frames").join(doc_id.to_string()))
}

/// Camera-solve sidecar directory (docs/10-FILE-FORMAT.md §3, K-417) — where a
/// tracked clip's solve is parked so the next session does not re-track it.
///
/// Global and keyed by (media fingerprint, analysis settings), for the reason
/// [`media_index_dir`] is: the solve describes the *file* and the settings it
/// was analysed under, not the project that happened to ask. Two projects
/// cutting the same rushes share one analysis, and a copy of a project finds
/// its solves already there. Rebuildable and deletable at any time, like every
/// tier under this root.
pub fn track_cache_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.cache_dir().join("track"))
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

/// The path of `target` relative to `base` (both taken as-is, no filesystem
/// access): the shared prefix is stripped and each remaining `base` component
/// becomes a `..`. None when no relative path exists at all — different
/// Windows drives — where the caller keeps the bare file name instead (the
/// footage-beside-the-project convention, and the fingerprint search covers
/// the rest). Always forward slashes, so a project saved on Windows resolves
/// on Linux and macOS unchanged.
#[must_use]
pub fn relative_between(base: &Path, target: &Path) -> Option<String> {
    use std::path::Component;
    let mut b: Vec<Component> = base.components().collect();
    let mut t: Vec<Component> = target.components().collect();
    // Cross-drive on Windows: no relative path exists.
    if let (Some(Component::Prefix(pb)), Some(Component::Prefix(pt))) = (b.first(), t.first()) {
        if pb.as_os_str() != pt.as_os_str() {
            return None;
        }
    }
    let common = b.iter().zip(t.iter()).take_while(|(x, y)| x == y).count();
    b.drain(..common);
    t.drain(..common);
    let mut parts: Vec<String> = b.iter().map(|_| "..".to_string()).collect();
    parts.extend(
        t.iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    Some(parts.join("/"))
}

/// A saved project carries relative paths and fingerprints, nothing
/// machine-specific (docs/10 §2, K-173): clone `doc` for writing with every
/// located media reference rebased against `project_dir` — the relative path
/// recomputed from the session's absolute path (or, failing that, wherever
/// the current relative path resolves) — and a fingerprint stamped where one
/// is missing, so the saved file can be found again by content after any
/// move. References whose file cannot be found right now are left exactly
/// as they are: saving must never lose the information a later relink needs.
/// The in-memory document is untouched (no ops, no dirty, no undo entries);
/// `absolute_path` never reaches the file regardless (it is serde-skipped).
#[must_use]
pub fn rebase_for_save(doc: &Document, project_dir: &Path) -> Document {
    let mut doc = doc.clone();
    for item in &mut doc.items {
        let ProjectItem::Footage(f) = item else {
            continue;
        };
        // Where is the file, right now? The session's absolute path first,
        // else wherever the stored relative path points.
        let abs = Path::new(&f.media.absolute_path);
        let located: Option<PathBuf> = if !f.media.absolute_path.is_empty() && abs.is_file() {
            Some(abs.to_path_buf())
        } else {
            let rel = project_dir.join(&f.media.relative_path);
            rel.is_file().then_some(rel)
        };
        let Some(located) = located else {
            continue; // missing: keep the reference untouched for relinking
        };
        if let Some(rel) = relative_between(project_dir, &located) {
            f.media.relative_path = rel;
        } else if let Some(name) = located.file_name() {
            // No relative path exists (another drive): the bare name — the
            // footage-beside-the-project convention — plus the fingerprint.
            f.media.relative_path = name.to_string_lossy().into_owned();
        }
        if f.media.fingerprint.is_none() {
            f.media.fingerprint = fingerprint_path(&located).ok();
        }
    }
    doc
}

/// Wire the docs/10 §2 resolver over a whole opened document: every footage
/// reference is resolved against the project's directory (relative → legacy
/// absolute → fingerprint search), the session `absolute_path` is pointed at
/// whatever was found, and the count of references that moved (found
/// somewhere other than their stored relative path) is returned alongside
/// the names of those still missing. The caller probes the updated paths;
/// missing items keep their reference untouched for the relink dialogue.
pub fn resolve_all_media(
    doc: &mut Document,
    project_dir: &Path,
    search_roots: &[PathBuf],
) -> (usize, Vec<String>) {
    let mut relinked = 0;
    let mut missing = Vec::new();
    for item in &mut doc.items {
        let ProjectItem::Footage(f) = item else {
            continue;
        };
        match resolve_media(&f.media, project_dir, search_roots) {
            Resolved::Found { path, how } => {
                if how != ResolveStep::RelativePath {
                    relinked += 1;
                }
                f.media.absolute_path = path.to_string_lossy().into_owned();
            }
            Resolved::Missing => missing.push(f.name.clone()),
        }
    }
    (relinked, missing)
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
///
/// `Clone` because the bridge shares one handle between the change observer and
/// the state that arms it: it is a path, not an open file, so a copy addresses
/// the same journal rather than competing for a descriptor.
#[derive(Clone)]
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

    /// **Where a document's parked frames go, pinned per platform.**
    ///
    /// Not a restatement of `directories`' documentation: what is checked is that
    /// the frame cache lands in the *same* Lumit folder as the journal and the
    /// media index (one folder, not three), under the platform's **cache**
    /// directory rather than its data or config directory, and keyed by the
    /// document id. Each of those is a one-word edit away from being wrong — and
    /// two of the ways it can be wrong are quiet: parking tens of gigabytes under
    /// Windows' *roaming* app data makes a work machine copy the lot to a network
    /// share at logoff, and parking them under the temp directory makes every
    /// project come back cold after a reboot, which reads as "the cache is
    /// broken" rather than as a wrong path.
    #[test]
    fn the_frame_cache_sits_in_lumits_own_cache_folder() {
        let id = Uuid::now_v7();
        let (Some(frames), Some(index), Some(journal), Some(presets)) = (
            frame_cache_dir(id),
            media_index_dir(),
            journal_path(id),
            presets_dir(),
        ) else {
            // No home directory at all (a bare container): the disk tier is off
            // rather than misplaced, which is the documented answer.
            eprintln!("skipping: this platform has no home directory");
            return;
        };

        // One Lumit folder: the frame cache shares the cache root with the media
        // index and the journal, both of which pre-date it.
        let cache_root = index.parent().expect("media-index has a parent");
        assert!(
            frames.starts_with(cache_root),
            "frames at {frames:?} left the cache root {cache_root:?}"
        );
        assert!(journal.starts_with(cache_root));
        assert!(
            frames.to_string_lossy().contains(&id.to_string()),
            "the document id is what makes a project find its frames again"
        );

        // The cache directory, not the data or config one: presets and settings
        // are kilobytes that should be backed up, and this is gigabytes that
        // should not.
        assert!(
            !frames.starts_with(presets.parent().expect("presets has a parent")),
            "the frame cache must not sit with the presets in app data"
        );

        // Never roaming (Windows), and never temp (all three).
        let shown = frames.to_string_lossy().to_lowercase();
        assert!(
            !shown.contains("roaming"),
            "a cache this size must not follow a roaming profile over the \
             network: {frames:?}"
        );
        assert!(
            !frames.starts_with(std::env::temp_dir()),
            "temp is emptied on reboot, so every project would come back cold: \
             {frames:?}"
        );
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

    /// TF-36 / K-173: what a saved project carries. The written clone's
    /// references are rebased relative to the project's folder and stamped
    /// with fingerprints; the serialized JSON contains no `absolute_path`
    /// key at all (it would embed the local username — the thing docs/10 §2
    /// promises the file never holds); and a legacy file that DOES carry one
    /// still loads it, so old saves keep their step-2 fallback.
    #[test]
    fn saved_projects_carry_relative_paths_and_no_absolute_ones() {
        let dir = tempfile::tempdir().unwrap();
        let media_dir = dir.path().join("media");
        fs::create_dir_all(&media_dir).unwrap();
        let file = media_dir.join("clip.bin");
        fs::write(&file, vec![7u8; 100_000]).unwrap();

        let mut doc = Document::new();
        let mut item = footage("clip.bin");
        item.media.relative_path = "stale/nonsense.bin".into(); // rebased below
        item.media.absolute_path = file.to_string_lossy().into_owned();
        apply(
            &mut doc,
            &Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(item)),
            },
        )
        .unwrap();

        let rebased = rebase_for_save(&doc, dir.path());
        let ProjectItem::Footage(f) = &rebased.items[0] else {
            panic!("footage survives the rebase");
        };
        assert_eq!(
            f.media.relative_path, "media/clip.bin",
            "rebased, / slashes"
        );
        assert!(f.media.fingerprint.is_some(), "fingerprint stamped on save");
        // The in-memory document is untouched.
        let ProjectItem::Footage(orig) = &doc.items[0] else {
            unreachable!()
        };
        assert_eq!(orig.media.relative_path, "stale/nonsense.bin");
        assert!(orig.media.fingerprint.is_none());

        // The file itself: no absolute path anywhere in the JSON.
        let json = serde_json::to_string(&rebased).unwrap();
        assert!(
            !json.contains("absolute_path"),
            "an absolute path embeds the username — never serialized (K-173)"
        );
        // A legacy file that carries one still loads it (step-2 fallback).
        let legacy: MediaRef = serde_json::from_str(
            r#"{"relative_path":"a.mp4","absolute_path":"/home/Full Name/a.mp4"}"#,
        )
        .unwrap();
        assert_eq!(legacy.absolute_path, "/home/Full Name/a.mp4");

        // A missing file keeps its reference untouched — saving must never
        // destroy the information a later relink needs.
        let mut doc2 = Document::new();
        let mut gone = footage("gone.bin");
        gone.media.relative_path = "somewhere/gone.bin".into();
        gone.media.absolute_path = "/nowhere/gone.bin".into();
        apply(
            &mut doc2,
            &Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(gone)),
            },
        )
        .unwrap();
        let rebased2 = rebase_for_save(&doc2, dir.path());
        let ProjectItem::Footage(f2) = &rebased2.items[0] else {
            unreachable!()
        };
        assert_eq!(f2.media.relative_path, "somewhere/gone.bin");
        assert!(f2.media.fingerprint.is_none());
    }

    /// TF-36: opening resolves every reference — the relative path first, and
    /// when it has gone stale, the fingerprint search finds the moved file
    /// (docs/10 §2 steps 1–3, previously built but never wired). The session
    /// absolute path points at whatever was found; missing files are named.
    #[test]
    fn open_resolution_relinks_moved_media_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();

        // One file where its relative path says; one moved elsewhere in the
        // project tree (found by fingerprint); one truly missing.
        let here = dir.path().join("here.bin");
        fs::write(&here, &data).unwrap();
        let moved_dir = dir.path().join("moved");
        fs::create_dir_all(&moved_dir).unwrap();
        let moved = moved_dir.join("renamed.bin");
        let mut other = data.clone();
        other[0] ^= 0xAA;
        fs::write(&moved, &other).unwrap();

        let mut doc = Document::new();
        let mut a = footage("here.bin");
        a.media.relative_path = "here.bin".into();
        a.media.absolute_path = String::new();
        let mut b = footage("was-elsewhere.bin");
        b.media.relative_path = "old/was-elsewhere.bin".into();
        b.media.absolute_path = String::new();
        b.media.fingerprint = Some(fingerprint_path(&moved).unwrap());
        let mut c = footage("gone.bin");
        c.media.relative_path = "gone.bin".into();
        c.media.absolute_path = String::new();
        c.media.fingerprint = None;
        for (i, item) in [a, b, c].into_iter().enumerate() {
            apply(
                &mut doc,
                &Op::AddItem {
                    index: i,
                    item: Box::new(ProjectItem::Footage(item)),
                },
            )
            .unwrap();
        }

        let (relinked, missing) = resolve_all_media(&mut doc, dir.path(), &[]);
        assert_eq!(relinked, 1, "only the moved file counts as relinked");
        assert_eq!(missing, vec!["gone.bin".to_string()]);
        let abs = |i: usize| match &doc.items[i] {
            ProjectItem::Footage(f) => f.media.absolute_path.clone(),
            _ => unreachable!(),
        };
        assert_eq!(abs(0), here.to_string_lossy());
        assert_eq!(abs(1), moved.to_string_lossy(), "found by content");
    }

    /// The pure relative-path arithmetic behind the rebase.
    #[test]
    fn relative_between_walks_up_and_down() {
        use std::path::Path;
        let base = Path::new("/projects/film");
        assert_eq!(
            relative_between(base, Path::new("/projects/film/media/a.mp4")).as_deref(),
            Some("media/a.mp4")
        );
        assert_eq!(
            relative_between(base, Path::new("/projects/other/b.mp4")).as_deref(),
            Some("../other/b.mp4")
        );
        assert_eq!(
            relative_between(base, Path::new("/projects/film/c.mp4")).as_deref(),
            Some("c.mp4")
        );
        #[cfg(windows)]
        assert_eq!(
            relative_between(Path::new("C:\\p"), Path::new("D:\\m\\a.mp4")),
            None,
            "cross-drive: no relative path exists"
        );
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

    /// An empty chain is a no-op, and the real chain leaves a document with
    /// nothing to migrate alone.
    #[test]
    fn no_migrations_leaves_json_unchanged() {
        let v = serde_json::json!({ "x": 5 });
        assert_eq!(run_migrations(&[], v.clone(), (0, 1, 0)), v);
        assert_eq!(run_migrations(MIGRATIONS, v.clone(), (0, 1, 0)), v);
    }

    /// A `0.1.0` document whose Footage layer carries the old segment store
    /// opens with that retiming on the layer's Retime **property** (K-249),
    /// and reads the same source moments it always did.
    ///
    /// Half speed is the case worth pinning: at four seconds of layer time the
    /// layer shows two seconds of source, before and after.
    #[test]
    fn the_old_segment_store_becomes_the_retime_property() {
        use lumit_core::retime::Retime;
        use lumit_core::time::Rational;

        let store = Retime::constant_speed(
            Rational::new(10, 1).unwrap(),
            Rational::ZERO,
            Rational::new(1, 2).unwrap(),
        );
        assert!(
            (store.evaluate(4.0) - 2.0).abs() < 1e-9,
            "the fixture is half speed"
        );

        let doc = serde_json::json!({
            "comps": [{
                "layers": [{
                    "kind": { "Footage": {
                        "item": Uuid::now_v7(),
                        "retime": serde_json::to_value(&store).unwrap(),
                    }}
                }]
            }]
        });
        let doc = run_migrations(MIGRATIONS, doc, (0, 1, 0));

        let layer = &doc["comps"][0]["layers"][0];
        assert!(
            layer["kind"]["Footage"]["retime"].is_null(),
            "the old store is emptied, so it can never be read a second time"
        );
        let property: lumit_core::anim::Property =
            serde_json::from_value(layer["retime"].clone()).expect("a Retime property");
        assert!(
            (property.value_at(4.0) - 2.0).abs() < 1e-9,
            "and it still shows the source moment it used to"
        );
    }

    /// A Sequence layer's **clips** convert too — the second half of K-249,
    /// and the one that would otherwise have left the sequence view editing a
    /// representation nothing else spoke.
    #[test]
    fn a_clips_segment_store_becomes_the_retime_property() {
        use lumit_core::retime::Retime;
        use lumit_core::time::Rational;

        let store = Retime::constant_speed(
            Rational::new(4, 1).unwrap(),
            Rational::ZERO,
            Rational::new(2, 1).unwrap(),
        );
        let doc = serde_json::json!({
            "comps": [{
                "layers": [{
                    "kind": { "Sequence": { "clips": [{
                        "id": Uuid::now_v7(),
                        "source": { "Footage": Uuid::now_v7() },
                        "source_in": [0, 1],
                        "source_out": [8, 1],
                        "place_start": [0, 1],
                        "place_duration": [4, 1],
                        "retime": serde_json::to_value(&store).unwrap(),
                    }]}}
                }]
            }]
        });

        let out = run_migrations(MIGRATIONS, doc, (0, 1, 0));
        // It typed as a real Clip, which is the whole test: the migration has
        // to produce a document *this* build can read.
        let clip: lumit_core::sequence::Clip = serde_json::from_value(
            out["comps"][0]["layers"][0]["kind"]["Sequence"]["clips"][0].clone(),
        )
        .expect("a clip");
        assert_eq!(
            clip.constant_speed(),
            Some(2.0),
            "double speed before, double speed after"
        );
        // …and it reads the same source moments it used to.
        assert!((clip.source_time(1.0) - store.evaluate(1.0)).abs() < 1e-6);
        assert!((clip.source_time(3.0) - store.evaluate(3.0)).abs() < 1e-6);
    }

    /// The policy for making in-between frames rides across too — it was never
    /// part of the map (docs/04 §10), and it is not lost with the store.
    #[test]
    fn the_migration_carries_the_interpolation_policy_out() {
        use lumit_core::retime::{Interpolation, Retime};
        use lumit_core::time::Rational;

        let mut store = Retime::identity(Rational::new(5, 1).unwrap(), Rational::ZERO);
        store.interpolation = Interpolation::Blend;
        let doc = serde_json::json!({
            "comps": [{
                "layers": [{
                    "kind": { "Footage": {
                        "item": Uuid::now_v7(),
                        "retime": serde_json::to_value(&store).unwrap(),
                    }}
                }]
            }]
        });

        let out = run_migrations(MIGRATIONS, doc, (0, 1, 0));
        let policy: Interpolation =
            serde_json::from_value(out["comps"][0]["layers"][0]["interpolation"].clone())
                .expect("a policy");
        assert_eq!(policy, Interpolation::Blend);
    }

    /// A layer that already carried the property keeps it: both routes existed
    /// at once, and the property is the one that was actually evaluating
    /// (`source_time_at` preferred it), so keeping it is what makes the file
    /// open looking the way it last rendered.
    #[test]
    fn the_property_wins_when_a_layer_carried_both() {
        use lumit_core::retime::Retime;
        use lumit_core::time::Rational;

        let segments = Retime::constant_speed(
            Rational::new(10, 1).unwrap(),
            Rational::ZERO,
            Rational::new(1, 2).unwrap(),
        );
        // The property says "hold source zero throughout" — nothing like the
        // segment store beside it, so which one survived is unambiguous.
        let property = lumit_core::anim::Property::fixed(0.0);
        let doc = serde_json::json!({
            "comps": [{
                "layers": [{
                    "retime": serde_json::to_value(&property).unwrap(),
                    "kind": { "Footage": {
                        "item": Uuid::now_v7(),
                        "retime": serde_json::to_value(&segments).unwrap(),
                    }}
                }]
            }]
        });

        let out = run_migrations(MIGRATIONS, doc, (0, 1, 0));
        let kept: lumit_core::anim::Property =
            serde_json::from_value(out["comps"][0]["layers"][0]["retime"].clone())
                .expect("a Retime property");
        assert!((kept.value_at(4.0) - 0.0).abs() < 1e-9);
    }

    /// A document with nothing to migrate survives the walk untouched — a
    /// layer of another kind, and a footage layer that was never retimed.
    #[test]
    fn the_migration_leaves_untouched_layers_alone() {
        let doc = serde_json::json!({
            "comps": [{
                "layers": [
                    { "kind": { "Footage": { "item": Uuid::now_v7() } } },
                    { "kind": "Adjustment" },
                ]
            }]
        });
        assert_eq!(run_migrations(MIGRATIONS, doc.clone(), (0, 1, 0)), doc);
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
        // The absolute path is session-state (K-173): never serialized, so it
        // comes back empty; everything else round-trips.
        assert_eq!(back.absolute_path, "");
        assert_eq!(back.relative_path, m.relative_path);
        assert_eq!(back.fingerprint, m.fingerprint);
    }

    /// **A project's own cache location travels with it.** The whole reason it
    /// lives in the document rather than in the settings file: copy the project
    /// to another machine, or hand it to someone else, and the folder it caches
    /// to comes along. A project that has not been given one saves nothing at
    /// all — an absent field, so an older build reads the file unchanged and a
    /// project's file does not grow a line for a choice nobody made.
    #[test]
    fn a_projects_own_cache_location_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.lum");

        let mut doc = doc_with_item();
        assert!(doc.cache_location.is_none(), "no override by default");
        save(&doc, &path).unwrap();
        assert!(open(&path).unwrap().0.cache_location.is_none());

        doc.cache_location = Some(lumit_core::model::CacheLocation::Custom {
            folder: "E:/scratch".into(),
        });
        save(&doc, &path).unwrap();
        assert_eq!(
            open(&path).unwrap().0.cache_location,
            Some(lumit_core::model::CacheLocation::Custom {
                folder: "E:/scratch".into()
            })
        );

        // The other two carry no folder, and still round-trip as themselves.
        doc.cache_location = Some(lumit_core::model::CacheLocation::BesideProject);
        save(&doc, &path).unwrap();
        assert_eq!(
            open(&path).unwrap().0.cache_location,
            Some(lumit_core::model::CacheLocation::BesideProject)
        );
    }

    /// **A project's arrangement travels with it** (K-245): hand the file to
    /// someone else and it opens with the panels where its author left them.
    /// The engine stores it as the frontend's own JSON without reading inside,
    /// so it round-trips whole; a project nobody has arranged saves no field at
    /// all, and an older build reads that file unchanged.
    #[test]
    fn the_saved_arrangement_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arranged.lum");

        let mut doc = doc_with_item();
        assert!(doc.ui_state.is_none(), "nothing arranged by default");
        save(&doc, &path).unwrap();
        assert!(open(&path).unwrap().0.ui_state.is_none());
        let bare = std::fs::metadata(&path).unwrap().len();

        let arrangement = serde_json::json!({
            "dock": { "kind": "tabs", "active": 1 },
            "session": { "frame": 12, "open_comps": ["a", "b"] },
        });
        doc.ui_state = Some(arrangement.clone());
        save(&doc, &path).unwrap();
        assert_eq!(open(&path).unwrap().0.ui_state, Some(arrangement));
        assert!(
            std::fs::metadata(&path).unwrap().len() > bare,
            "it is really in the file, not only in the document"
        );
    }

    #[test]
    fn save_open_round_trip_and_no_temp_litter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.lum");
        let mut doc = doc_with_item();
        save(&doc, &path).unwrap();
        let (loaded, manifest) = open(&path).unwrap();
        // Absolute paths are session-state, never saved (K-173) — equality
        // holds once the original's is cleared to match.
        if let ProjectItem::Footage(f) = &mut doc.items[0] {
            f.media.absolute_path = String::new();
        }
        assert_eq!(loaded, doc);
        assert_eq!(manifest.format, FORMAT);
        save(&doc, &path).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// The anti-aliasing setting is a project property (K-274,
    /// docs/impl/anti-aliasing.md §5, test 7): a non-default value must survive
    /// a save and reload, and a `.lum` written before the field existed must
    /// load at the default rather than failing.
    #[test]
    fn the_anti_aliasing_setting_round_trips_and_defaults_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.lum");
        let mut doc = doc_with_item();
        doc.anti_aliasing = lumit_core::model::AntiAliasing::X8;
        save(&doc, &path).unwrap();
        let (loaded, _) = open(&path).unwrap();
        assert_eq!(loaded.anti_aliasing, lumit_core::model::AntiAliasing::X8);

        // An older file: the same project with the key removed entirely, which
        // is exactly what a `.lum` written before this field looks like.
        let older = dir.path().join("older.lum");
        strip_document_key(&path, &older, "anti_aliasing");
        let (old, _) = open(&older).unwrap();
        assert_eq!(
            old.anti_aliasing,
            lumit_core::model::AntiAliasing::default(),
            "a file with no setting must load at the default, not fail"
        );
    }

    /// Rewrite a `.lum` with one key deleted from its document JSON — how a
    /// test stands in for a file written by a build that predates a field.
    fn strip_document_key(from: &Path, to: &Path, key: &str) {
        let (_, manifest) = open(from).unwrap();
        let mut zip = ZipArchive::new(File::open(from).unwrap()).unwrap();
        let mut raw = String::new();
        {
            use std::io::Read;
            zip.by_name("project.json")
                .unwrap()
                .read_to_string(&mut raw)
                .unwrap();
        }
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(
            value.as_object_mut().unwrap().remove(key).is_some(),
            "{key} was not in the saved document, so removing it proves nothing"
        );
        let doc: Document = serde_json::from_value(value).unwrap();
        let _ = manifest;
        save(&doc, to).unwrap();
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
