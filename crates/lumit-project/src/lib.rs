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
            let (w, h) = (f64::from(comp.width), f64::from(comp.height));
            for layer in &mut comp.layers {
                lumit_core::fx::backfill_builtin_params(&mut layer.effects);
                // And convert the share-of-the-frame values K-558 turned into
                // px@comp. Separate because it needs the composition's own
                // size, which is why it is done here rather than in the
                // backfill: a per cent is only a pixel count once the frame is
                // known.
                lumit_core::fx::migrate_percent_to_px(&mut layer.effects, w, h);
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

/// Where the user's own **export** presets live (docs/07 §11) — one small
/// JSON file beside the effect-preset library, in the same roaming app-data
/// area, because a saved export setting should follow the user between
/// projects and between machines that share a profile.
///
/// A file rather than a folder: an export preset is a few dozen fields, the
/// list is short, and one file is one thing to back up. `None` only when the
/// platform has no home directory; the library then lives for the session and
/// says so rather than failing.
pub fn export_presets_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_dir().join("export-presets.json"))
}

/// The file name of the sound an export plays when it finishes, if the
/// *When done → make a noise* hook is set (docs/07 §11).
pub const EXPORT_DONE_SOUND: &str = "export-done.wav";

/// Where that sound lives in the application's own data area — the second
/// place looked in, after a copy beside the executable, so a user can supply
/// one without touching an installed build. `None` when the platform has no
/// home directory; the hook is then simply silent.
pub fn export_done_sound_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_dir().join("sounds").join(EXPORT_DONE_SOUND))
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
        rebase_one(&mut f.media, project_dir);
    }
    // A proxy is a media reference like any other (K-501): it is written
    // relative, fingerprinted, and found again after a move by exactly the same
    // rules — otherwise reopening a project would silently lose every proxy and
    // the small pictures would come back as full-resolution ones with no
    // explanation on screen.
    for proxy in doc.proxies.values_mut() {
        rebase_one(&mut proxy.media, project_dir);
    }
    // And the project's OCIO config (K-490), for the reason it was made a
    // `MediaRef` rather than a bare path: a config that travelled with its
    // project keeps working, and one that moved elsewhere relinks by content
    // through the machinery footage already uses.
    if let Some(config) = doc.colour.config.as_mut() {
        rebase_one(config, project_dir);
    }
    doc
}

/// One media reference rebased against `project_dir`: the relative path
/// recomputed from wherever the file is right now, and a fingerprint stamped
/// where none was held. A reference whose file cannot be found is left exactly
/// as it is — saving must never lose the information a later relink needs.
fn rebase_one(media: &mut lumit_core::model::MediaRef, project_dir: &Path) {
    // Where is the file, right now? The session's absolute path first,
    // else wherever the stored relative path points.
    let abs = Path::new(&media.absolute_path);
    let located: Option<PathBuf> = if !media.absolute_path.is_empty() && abs.is_file() {
        Some(abs.to_path_buf())
    } else {
        let rel = project_dir.join(&media.relative_path);
        rel.is_file().then_some(rel)
    };
    let Some(located) = located else {
        return; // missing: keep the reference untouched for relinking
    };
    if let Some(rel) = relative_between(project_dir, &located) {
        media.relative_path = rel;
    } else if let Some(name) = located.file_name() {
        // No relative path exists (another drive): the bare name — the
        // footage-beside-the-project convention — plus the fingerprint.
        media.relative_path = name.to_string_lossy().into_owned();
    }
    if media.fingerprint.is_none() {
        media.fingerprint = fingerprint_path(&located).ok();
    }
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
    let mut unresolved = Vec::new();
    for (index, item) in doc.items.iter_mut().enumerate() {
        let ProjectItem::Footage(f) = item else {
            continue;
        };
        // An image sequence imported from After Effects arrives pointing at the
        // *folder* the run lives in, because that is what the .aep records — a
        // folder is not a file, so every step below would call it missing. One
        // look inside turns it into the run's first frame, which is what a
        // sequence item points at everywhere else (K-539).
        if f.sequence.is_some() {
            if let Some(frame) = first_numbered_file(Path::new(&f.media.absolute_path))
                .or_else(|| first_numbered_file(&project_dir.join(&f.media.relative_path)))
            {
                f.media.relative_path = relative_between(project_dir, &frame)
                    .unwrap_or_else(|| f.media.relative_path.clone());
                f.media.absolute_path = frame.to_string_lossy().into_owned();
            }
        }
        match resolve_media(&f.media, project_dir, search_roots) {
            Resolved::Found { path, how } => {
                if how != ResolveStep::RelativePath {
                    relinked += 1;
                }
                f.media.absolute_path = path.to_string_lossy().into_owned();
            }
            Resolved::Missing => unresolved.push(index),
        }
    }
    // Step 3b: whatever is still lost, looked for **by file name** under the
    // project's own folder. This is the weakest of the matches, so it runs last
    // and only for what nothing else found — and it is the one that answers the
    // ordinary case: a project that arrived beside its footage but with the
    // paths of the machine it was made on written into it, which is every
    // After Effects import on a second computer. The tree is walked once for
    // all of them rather than once each, because forty-eight lost clips would
    // otherwise be forty-eight walks of the same folder — and not at all when
    // nothing is lost, which is what the guard is for: the walk is the whole
    // cost of this step, and a project that opened clean must not pay it.
    let mut missing = Vec::new();
    if !unresolved.is_empty() {
        let beside = files_by_name(project_dir);
        for index in unresolved {
            let Some(ProjectItem::Footage(f)) = doc.items.get_mut(index) else {
                continue;
            };
            let found = file_name_of(&f.media.relative_path)
                .or_else(|| file_name_of(&f.media.absolute_path))
                .and_then(|name| beside.get(name));
            match found {
                Some(path) => {
                    relinked += 1;
                    f.media.absolute_path = path.to_string_lossy().into_owned();
                }
                None => missing.push(f.name.clone()),
            }
        }
    }
    // Proxies resolve by the same three steps, and a proxy that cannot be found
    // is **not** reported missing (K-501): a missing stand-in is not a missing
    // clip, the render falls back to the original on its own, and putting it in
    // this list would open the relink dialogue over footage that is perfectly
    // present. It also does not count as a relink, which is a count of the
    // project's real media.
    for proxy in doc.proxies.values_mut() {
        if let Resolved::Found { path, .. } = resolve_media(&proxy.media, project_dir, search_roots)
        {
            proxy.media.absolute_path = path.to_string_lossy().into_owned();
        }
    }
    // The colour config resolves by the same three steps, and like a proxy it
    // is **not** reported missing: a project whose config vanished still opens,
    // still keeps every colour space name it was given, and simply previews
    // through the built-in family until the file comes back (K-490's calm
    // degrade). Opening the relink dialogue over it would be a lie about what
    // is wrong.
    if let Some(config) = doc.colour.config.as_mut() {
        if let Resolved::Found { path, .. } = resolve_media(config, project_dir, search_roots) {
            config.absolute_path = path.to_string_lossy().into_owned();
        }
    }
    (relinked, missing)
}

/// The first numbered file directly inside `dir`, in name order — the frame an
/// image-sequence folder's run starts at.
///
/// `None` when `dir` is not a directory (the ordinary case: it is already a
/// file, or it is not there at all) or holds nothing numbered.
///
/// "Numbered" rather than "an image": a sequence folder's stray `readme.txt`,
/// `Thumbs.db` or `.DS_Store` would otherwise sort ahead of the frames and be
/// picked as the run's first file. A frame of a sequence has a number in its
/// name by definition, and those do not.
fn first_numbered_file(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.chars().any(|c| c.is_ascii_digit()))
        .collect();
    names.sort();
    names.first().map(|name| dir.join(name))
}

/// Every file under `root`, by file name, first one in walk order winning.
///
/// Symlinks are not followed, so the walk cannot cycle. Two files of the same
/// name in different subfolders are a genuine ambiguity and the first found
/// answers for both; the fingerprint search above it is what tells them apart
/// once a project has been saved.
fn files_by_name(root: &Path) -> std::collections::HashMap<String, PathBuf> {
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                out.entry(entry.file_name().to_string_lossy().into_owned())
                    .or_insert_with(|| entry.path());
            }
        }
    }
    out
}

/// The directory remapping implied by one file moving from `old` to `new`,
/// used to relink siblings that moved the same way (docs/10 §2). Defined only
/// for a pure relocation — same file name, different directory; None for a
/// rename (a changed name cannot generalise to siblings) or a non-move.
///
/// **The pair is the shallowest one the move proves.** Media does not live in
/// one flat folder: `Clips/scene 1/a.mov` moving to `Backup/scene 1/a.mov`
/// says that `Clips` became `Backup`, and mapping only the immediate parents
/// would leave every sibling in `scene 2` unfound — which is what made
/// relinking a real project a forty-clip job. So the shared trailing
/// components are peeled off both sides. Over-reaching costs nothing: a
/// mapping only ever *suggests* where to look, and the caller repoints an item
/// only when a file is actually there.
#[must_use]
pub fn path_mapping(old: &Path, new: &Path) -> Option<(PathBuf, PathBuf)> {
    if old.file_name()? != new.file_name()? {
        return None;
    }
    let (mut old_dir, mut new_dir) = (old.parent()?, new.parent()?);
    if old_dir == new_dir {
        return None;
    }
    while old_dir.file_name().is_some() && old_dir.file_name() == new_dir.file_name() {
        let (Some(up_old), Some(up_new)) = (old_dir.parent(), new_dir.parent()) else {
            break;
        };
        if up_old == up_new {
            // The two agree from here up: the move is the pair below.
            break;
        }
        (old_dir, new_dir) = (up_old, up_new);
    }
    Some((old_dir.to_path_buf(), new_dir.to_path_buf()))
}

/// The file name at the end of a stored path, split on **both** separators.
///
/// `Path::file_name` splits on the separator of the machine it is running on,
/// and a media reference is written on whichever machine made the project — a
/// Windows path handed to it on macOS comes back whole, so every name match
/// against one silently fails. Paths in a project are text until they are
/// resolved, so they are taken apart as text.
#[must_use]
pub fn file_name_of(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
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
    // Proxies do not travel (K-501). A collected project is a copy of the
    // *work*: the stand-ins are local convenience files, regenerable in one
    // action from the originals that are being copied, and carrying them would
    // double the size of the folder to ship pictures nobody delivers. Dropping
    // them here rather than leaving dangling references also means the copy
    // opens with no missing-proxy state to explain.
    out.proxies.clear();
    // The OCIO config does not travel either, and is **kept referenced** rather
    // than cleared (K-490). It is not one file: it is a text file plus whatever
    // look-up tables its own `search_path` points at, so copying the one file
    // into `media/` would produce a config that parses and then cannot find a
    // single table. Clearing it instead would silently change what the copy
    // looks like, which is worse. So the name goes with the project and the
    // copy either finds the config on the other machine or degrades calmly and
    // says so — the same answer a missing config gets anywhere else.
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
            sequence: None,
            id: Uuid::now_v7(),
            name: name.into(),
            extra: serde_json::Map::new(),
            colour_space: None,
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

    /// **An image sequence imported from After Effects opens on its first
    /// frame** (K-539).
    ///
    /// The .aep names the folder a run lives in — that is what its file alias
    /// targets — and a folder is not a file, so every resolution step would
    /// call an imported sequence missing and send the user to relink something
    /// that is sitting right there. One look inside answers it. The stray
    /// `readme.txt` in the fixture is the reason "the first numbered file"
    /// rather than "the first file": it sorts ahead of the frames.
    #[test]
    fn an_imported_sequence_folder_resolves_to_the_runs_first_frame() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Depth");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("readme.txt"), b"notes").unwrap();
        for n in 0..4u32 {
            fs::write(folder.join(format!("Depth{n:06}_depth.exr")), b"frame").unwrap();
        }

        let mut doc = Document::default();
        let item = FootageItem {
            colour_space: None,
            id: Uuid::now_v7(),
            name: "Depth".into(),
            media: MediaRef {
                relative_path: "Depth".into(),
                absolute_path: folder.to_string_lossy().into_owned(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            sequence: Some(lumit_core::model::SequenceRef::default()),
            extra: serde_json::Map::new(),
        };
        lumit_core::ops::apply(
            &mut doc,
            &Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(item)),
            },
        )
        .unwrap();

        let (_, missing) = resolve_all_media(&mut doc, dir.path(), &[]);
        assert!(missing.is_empty(), "the run is right there: {missing:?}");
        let ProjectItem::Footage(f) = &doc.items[0] else {
            unreachable!()
        };
        assert_eq!(
            f.media.relative_path, "Depth/Depth000000_depth.exr",
            "the folder became the frame the run starts at"
        );
        assert!(
            Path::new(&f.media.absolute_path).is_file(),
            "and it resolved to a real file: {}",
            f.media.absolute_path
        );
    }

    /// **An import from another machine finds its footage beside the project.**
    ///
    /// The paths written into an After Effects project are the paths of the
    /// computer it was made on, and there are no fingerprints yet — nothing
    /// was ever saved — so steps 1 to 3 all come back empty on a second
    /// machine even when every file is sitting right there in a subfolder.
    /// Step 3b looks for what is left by file name under the project's own
    /// folder, which is the one thing that is true about a project someone
    /// copied across with its media.
    #[test]
    fn what_nothing_else_found_is_looked_for_by_name_beside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let buried = dir.path().join("Clips").join("Cine1");
        fs::create_dir_all(&buried).unwrap();
        fs::write(buried.join("Depth.avi"), b"clip").unwrap();

        let mut doc = Document::new();
        // As an import leaves it: both paths are the other machine's.
        let mut found = footage("Depth.avi");
        found.media.relative_path = "D:/Elsewhere/Clips/Cine1/Depth.avi".into();
        found.media.absolute_path = found.media.relative_path.clone();
        let mut lost = footage("Missing.avi");
        lost.media.relative_path = "D:/Elsewhere/Clips/Cine1/Missing.avi".into();
        lost.media.absolute_path = lost.media.relative_path.clone();
        for (i, item) in [found, lost].into_iter().enumerate() {
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
        assert_eq!(relinked, 1);
        assert_eq!(missing, vec!["Missing.avi".to_string()]);
        match &doc.items[0] {
            ProjectItem::Footage(f) => assert_eq!(
                f.media.absolute_path,
                buried.join("Depth.avi").to_string_lossy(),
                "found by name however deep it sits"
            ),
            _ => unreachable!(),
        }
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
        // A whole tree carried to another drive maps its *root*, not the one
        // folder the relinked file happens to sit in — otherwise a clip four
        // folders down brings back only its own neighbours.
        let deep = path_mapping(
            Path::new("/old/edit/Clips/Cine1/Depth.avi"),
            Path::new("/new/place/Clips/Cine1/Depth.avi"),
        )
        .expect("a tree move maps");
        assert_eq!(
            apply_mapping(&deep, Path::new("/old/edit/Clips/Cine5/World.avi")),
            Some(PathBuf::from("/new/place/Clips/Cine5/World.avi")),
            "a sibling in a different subfolder relinks under the same move"
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

    /// **A whole media tree that moved is one mapping, not one per folder.**
    ///
    /// The regression: footage sits in `Clips/scene 1`, `Clips/scene 2`, and
    /// relinking a clip out of the first folder used to say only that
    /// `old/Clips/scene 1` had become `new/Clips/scene 1` — leaving every
    /// sibling in `scene 2` for the user to find by hand. The shared tail is
    /// the part that did *not* move, so it is peeled off.
    #[test]
    fn a_mapping_reaches_the_folder_that_actually_moved() {
        let mapping = path_mapping(
            Path::new("/old/Clips/scene 1/a.mov"),
            Path::new("/new/Clips/scene 1/a.mov"),
        )
        .expect("a pure move maps");
        assert_eq!(mapping, (PathBuf::from("/old"), PathBuf::from("/new")));
        assert_eq!(
            apply_mapping(&mapping, Path::new("/old/Clips/scene 2/b.mov")),
            Some(PathBuf::from("/new/Clips/scene 2/b.mov")),
            "a sibling in the folder next door relinks under the same move"
        );

        // The peel stops where the two paths meet: a folder moved *within* one
        // tree maps that folder, not the tree it is still inside.
        let inside = path_mapping(
            Path::new("/proj/a/scene/clip.mov"),
            Path::new("/proj/b/scene/clip.mov"),
        )
        .expect("a pure move maps");
        assert_eq!(inside, (PathBuf::from("/proj/a"), PathBuf::from("/proj/b")));
    }

    fn footage_item(name: &str, rel: &str, abs: &str) -> lumit_core::model::ProjectItem {
        lumit_core::model::ProjectItem::Footage(lumit_core::model::FootageItem {
            sequence: None,
            id: Uuid::now_v7(),
            name: name.into(),
            media: media_ref(rel, abs, None),
            extra: serde_json::Map::new(),
            colour_space: None,
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

    /// **A sequence costs a project that has none nothing at all** (K-539). The
    /// field is skipped when unset, so every project saved before image
    /// sequences existed round-trips byte-for-byte — and a project that *does*
    /// have one carries its rate back exactly, because a rate that went through
    /// a float would not (docs/14 §2).
    #[test]
    fn a_sequence_saves_only_when_there_is_one_and_keeps_its_exact_rate() {
        let plain = footage("clip.mp4");
        let json = serde_json::to_string(&plain).unwrap();
        assert!(
            !json.contains("sequence"),
            "an ordinary file must not grow a sequence field: {json}"
        );

        let mut run = footage("shot[0001-0100].exr");
        run.sequence = Some(lumit_core::model::SequenceRef {
            frame_rate: lumit_core::time::FrameRate::new(24000, 1001).unwrap(),
            extra: serde_json::Map::new(),
        });
        let back: lumit_core::model::FootageItem =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(back.sequence_fps(), Some((24000, 1001)));
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

    /// **Colour tags travel with the project** (K-451). They are organisation
    /// rather than picture, but organisation is exactly what is lost when a
    /// project is handed on, so they belong in the file. A project nobody has
    /// tagged saves no field at all — the serde-default rule docs/10 §1.1 gives
    /// every additive field — so an older build reads such a file unchanged,
    /// and a file written before tags existed opens with every item untagged
    /// rather than failing.
    #[test]
    fn item_colour_tags_survive_a_save_and_older_files_open_untagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tagged.lum");

        let mut doc = doc_with_item();
        let id = doc.items[0].id();
        assert_eq!(doc.item_label(id), 0, "untagged by default");
        save(&doc, &path).unwrap();
        assert!(
            open(&path).unwrap().0.item_labels.is_empty(),
            "a project nobody has tagged gains no field"
        );

        apply(&mut doc, &Op::SetItemLabel { id, label: 5 }).unwrap();
        save(&doc, &path).unwrap();
        assert_eq!(open(&path).unwrap().0.item_label(id), 5);

        // The shape a file written before tags existed has: no key at all.
        let mut older: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        older
            .as_object_mut()
            .expect("a document is an object")
            .remove("item_labels");
        let reopened: Document = serde_json::from_value(older).unwrap();
        assert_eq!(reopened.item_label(id), 0);
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

    /// **The colour shelf survives the file, and an empty one writes no line**
    /// (K-448, docs/10 §1.1): a project nobody has kept a colour in must be
    /// byte-identical to one written before swatches existed, and a shelf must
    /// come back in the order it was kept, names and all.
    #[test]
    fn the_colour_shelf_round_trips_and_an_empty_one_writes_no_line() {
        use lumit_core::model::{LinearColour, Swatch};

        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.lum");
        save(&doc_with_item(), &plain).unwrap();
        let json = String::from_utf8(entry_bytes(&plain, "project.json")).unwrap();
        assert!(
            !json.contains("\"swatches\""),
            "a project with no swatches must write no line for them:\n{json}"
        );

        let path = dir.path().join("edit.lum");
        let mut doc = doc_with_item();
        doc.swatches = vec![
            Swatch {
                colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
                name: Some("Brand red".into()),
            },
            Swatch {
                colour: LinearColour([0.0, 0.25, 0.5, 0.75]),
                name: None,
            },
        ];
        save(&doc, &path).unwrap();
        let (loaded, _) = open(&path).unwrap();
        assert_eq!(loaded.swatches, doc.swatches);

        // An older file: the same project with the key removed, which is what a
        // `.lum` written before the shelf existed looks like.
        let older = dir.path().join("older.lum");
        strip_document_key(&path, &older, "swatches");
        let (old, _) = open(&older).unwrap();
        assert!(
            old.swatches.is_empty(),
            "a file with no shelf must load with an empty one, not fail"
        );
    }

    /// **The colour settings survive the file, and cost an older one nothing**
    /// (K-490, docs/impl/ocio.md §3.1, §9.1). Three things, because only the
    /// first is visible and the other two are the ones a regression breaks:
    ///
    /// 1. A project that has never named a config writes **no line** for
    ///    either field, so a `.lum` saved before OCIO existed round-trips
    ///    byte-identically.
    /// 2. A named config comes back whole, and its relative path is rebased
    ///    and fingerprinted on save exactly as footage is — the whole reason
    ///    it is a `MediaRef` and not a string.
    /// 3. A per-item colour space comes back as the name it was given.
    #[test]
    fn the_colour_settings_round_trip_and_cost_an_older_file_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.lum");
        let doc = doc_with_item();
        save(&doc, &plain).unwrap();
        let json = String::from_utf8(entry_bytes(&plain, "project.json")).unwrap();
        assert!(
            !json.contains("\"colour\"") && !json.contains("colour_space"),
            "a project with no colour settings must write no line for them:\n{json}"
        );

        // A config beside the project, so rebasing has something real to find.
        let config = dir.path().join("aces/config.ocio");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "ocio_profile_version: 2\n").unwrap();

        let path = dir.path().join("edit.lum");
        let mut doc = doc_with_item();
        doc.colour.config = Some(lumit_core::model::MediaRef {
            relative_path: String::new(),
            absolute_path: config.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        });
        if let ProjectItem::Footage(f) = &mut doc.items[0] {
            f.colour_space = Some("ACEScct".into());
        }
        save(&rebase_for_save(&doc, dir.path()), &path).unwrap();

        let saved = String::from_utf8(entry_bytes(&path, "project.json")).unwrap();
        assert!(
            !saved.contains(&config.to_string_lossy().replace('\\', "\\\\")),
            "an absolute path must never reach the file (K-173):\n{saved}"
        );

        let (mut loaded, _) = open(&path).unwrap();
        let (_, missing) = resolve_all_media(&mut loaded, dir.path(), &[]);
        assert!(
            !missing.iter().any(|m| m.contains("config")),
            "the config is not reported as missing footage"
        );
        let held = loaded.colour.config.clone().expect("the config came back");
        assert_eq!(
            held.relative_path.replace('\\', "/"),
            "aces/config.ocio",
            "the config is stored relative to the project and rebased on save"
        );
        assert!(
            held.fingerprint.is_some(),
            "a located config is fingerprinted, so it relinks by content"
        );
        assert!(
            held.absolute_path.ends_with("config.ocio"),
            "opening resolves the config for this session"
        );
        let ProjectItem::Footage(f) = &loaded.items[0] else {
            panic!("expected footage");
        };
        assert_eq!(f.colour_space.as_deref(), Some("ACEScct"));

        // An older file: the same project with the keys removed, which is what
        // a `.lum` written before these fields looks like.
        let older = dir.path().join("older.lum");
        strip_document_key(&path, &older, "colour");
        let (old, _) = open(&older).unwrap();
        assert_eq!(
            old.colour,
            lumit_core::model::ColourManagement::default(),
            "a file with no colour block loads at the default, not a failure"
        );
    }

    /// **A vanished config never holds the project hostage** (K-490's calm
    /// degrade, docs/impl/ocio.md §3.3). It opens, it keeps every name it was
    /// given, and — unlike footage — it is not reported missing, because a
    /// missing config is not a missing clip and opening the relink dialogue
    /// over it would say the wrong thing.
    #[test]
    fn a_config_that_vanished_opens_quietly_and_keeps_its_names() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("aces/config.ocio");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "ocio_profile_version: 2\n").unwrap();

        let path = dir.path().join("edit.lum");
        let mut doc = doc_with_item();
        doc.colour.config = Some(lumit_core::model::MediaRef {
            relative_path: String::new(),
            absolute_path: config.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        });
        if let ProjectItem::Footage(f) = &mut doc.items[0] {
            f.colour_space = Some("ACEScct".into());
        }
        save(&rebase_for_save(&doc, dir.path()), &path).unwrap();
        fs::remove_file(&config).unwrap();

        let (loaded, _) = open(&path).unwrap();
        let held = loaded.colour.config.expect("the reference is kept");
        assert_eq!(held.relative_path.replace('\\', "/"), "aces/config.ocio");
        let ProjectItem::Footage(f) = &loaded.items[0] else {
            panic!("expected footage");
        };
        assert_eq!(
            f.colour_space.as_deref(),
            Some("ACEScct"),
            "a name is the user's statement about the file; a missing config never drops it"
        );
    }

    /// **A proxy is a media reference like any other, and it survives the
    /// file** (K-501, docs/03 §3a). Three things at once, because all three are
    /// easy to get wrong and only the first is visible:
    ///
    /// 1. A project with **no** proxies writes no line for either field, so
    ///    every `.lum` saved before proxies existed round-trips unchanged.
    /// 2. An attached proxy comes back whole — its path, its own *use proxy*
    ///    switch, and the project-wide switch — and its **relative path is
    ///    rebased and fingerprinted on save** exactly as the original's is. A
    ///    proxy that did not get that treatment would be found on the machine
    ///    that made it and nowhere else.
    /// 3. A file written before the fields existed loads with the master switch
    ///    **on**, which is the default a new project has.
    #[test]
    fn a_proxy_round_trips_and_a_project_without_one_writes_no_line_for_it() {
        use lumit_core::model::ProxyRef;

        let dir = tempfile::tempdir().unwrap();
        let media_dir = dir.path().join("media");
        fs::create_dir_all(&media_dir).unwrap();
        let original = media_dir.join("clip.bin");
        let proxy_file = media_dir.join("clip_proxy.mov");
        fs::write(&original, vec![7u8; 4096]).unwrap();
        fs::write(&proxy_file, vec![9u8; 512]).unwrap();

        // 1. Nothing attached: neither key reaches the file.
        let plain = dir.path().join("plain.lum");
        save(&doc_with_item(), &plain).unwrap();
        let raw = document_json(&plain);
        let obj = raw.as_object().unwrap();
        assert!(
            !obj.contains_key("proxies") && !obj.contains_key("use_proxies"),
            "a project with no proxies must save exactly as it did before them"
        );

        // 2. One attached, with a deliberately stale relative path to prove the
        //    save rebases it.
        let mut doc = Document::new();
        let mut item = footage("clip.bin");
        item.media.relative_path = "stale/nonsense.bin".into();
        item.media.absolute_path = original.to_string_lossy().into_owned();
        let id = item.id;
        apply(
            &mut doc,
            &Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(item)),
            },
        )
        .unwrap();
        apply(
            &mut doc,
            &Op::SetItemProxy {
                id,
                proxy: Some(Box::new(ProxyRef {
                    media: lumit_core::model::MediaRef {
                        relative_path: "also/stale.mov".into(),
                        absolute_path: proxy_file.to_string_lossy().into_owned(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    enabled: false,
                    extra: serde_json::Map::new(),
                })),
            },
        )
        .unwrap();
        doc.use_proxies = false;

        let path = dir.path().join("edit.lum");
        // The shell rebases before it writes, so the test does too.
        save(&rebase_for_save(&doc, dir.path()), &path).unwrap();
        let (loaded, _) = open(&path).unwrap();
        assert!(!loaded.use_proxies, "the master switch survives");
        let back = loaded.proxy(id).expect("the proxy survives");
        assert!(!back.enabled, "and so does the item's own switch");
        assert_eq!(
            back.media.relative_path, "media/clip_proxy.mov",
            "a proxy's relative path is rebased on save like any other reference"
        );
        assert!(
            back.media.fingerprint.is_some(),
            "and it is fingerprinted, so it can be found again after a move"
        );
        // And resolving — the step the shell runs after opening — points the
        // session path at the file that is actually there, without reporting
        // anything missing, because nothing is.
        let mut loaded = loaded;
        let (_, missing) = resolve_all_media(&mut loaded, dir.path(), &[]);
        assert!(missing.is_empty());
        let back = loaded.proxy(id).unwrap();
        assert_eq!(
            Path::new(&back.media.absolute_path).canonicalize().ok(),
            proxy_file.canonicalize().ok()
        );

        // A proxy that has gone missing is **not** a missing clip: the render
        // falls back to the original on its own, so it must never open the
        // relink dialogue over footage that is perfectly present.
        fs::remove_file(&proxy_file).unwrap();
        let (_, missing) = resolve_all_media(&mut loaded, dir.path(), &[]);
        assert!(
            missing.is_empty(),
            "a missing proxy is not reported as missing media"
        );

        // 3. The same file with the two keys removed — what a `.lum` written
        //    before proxies looks like.
        let half = dir.path().join("half.lum");
        let older = dir.path().join("older.lum");
        strip_document_key(&path, &half, "proxies");
        strip_document_key(&half, &older, "use_proxies");
        let (old, _) = open(&older).unwrap();
        assert!(old.proxies.is_empty());
        assert!(
            old.use_proxies,
            "a file with no master switch loads with it on, like a new project"
        );
    }

    /// A collected project ships the originals and **not** the proxies (K-501):
    /// the stand-ins are local convenience files, remade in one action, and a
    /// copy that carried them would be twice the size and open with references
    /// to files nobody sent.
    #[test]
    fn collecting_for_sharing_leaves_the_proxies_behind() {
        use lumit_core::model::ProxyRef;

        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let clip = src.path().join("clip.bin");
        fs::write(&clip, vec![3u8; 2048]).unwrap();

        let mut doc = Document::new();
        let mut item = footage("clip.bin");
        item.media.relative_path = "clip.bin".into();
        item.media.absolute_path = clip.to_string_lossy().into_owned();
        let id = item.id;
        doc.items.push(ProjectItem::Footage(item));
        doc.proxies.insert(
            id,
            ProxyRef {
                media: lumit_core::model::MediaRef {
                    relative_path: "clip_proxy.mov".into(),
                    absolute_path: src
                        .path()
                        .join("clip_proxy.mov")
                        .to_string_lossy()
                        .into_owned(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                enabled: true,
                extra: serde_json::Map::new(),
            },
        );

        let collected = collect_for_sharing(&doc, src.path(), dest.path()).unwrap();
        assert!(collected.missing.is_empty());
        assert!(
            collected.doc.proxies.is_empty(),
            "the copy carries no proxy references"
        );
        assert!(
            !dest.path().join("media/clip_proxy.mov").exists(),
            "and no proxy file was copied"
        );
        // The original did travel, which is the half that must not break.
        assert!(dest.path().join("media/clip.bin").is_file());
    }

    /// **A marker can carry a span, and the span survives the file**
    /// (docs/15-DESIGN.md §12A.1, docs/03-DATA-MODEL.md §11). The redesigned
    /// ruler draws a marker as a pill that runs from its frame for its
    /// duration, so the number has to be in the `.lum` and not merely in the
    /// session — and a marker that is only a moment must stay a moment,
    /// written as no span at all rather than as a zero-length one.
    ///
    /// The second half is the one that would go unnoticed: a `.lum` written
    /// before markers could span at all must open with its markers as moments,
    /// not fail to open. Every additive field owes that (docs/10 §1.1), and a
    /// marker's is easy to miss because markers arrive inside a composition
    /// rather than at the top of the document.
    #[test]
    fn a_markers_duration_round_trips_and_is_absent_when_it_is_a_moment() {
        use lumit_core::markers::{Marker, MarkerKind};
        use lumit_core::model::{Composition, LinearColour};
        use lumit_core::time::{Duration, FrameRate, Rational};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marked.lum");
        let rat = |n: i64, d: i64| Rational::new(n, d).unwrap();

        let moment = Marker::user(Uuid::now_v7(), rat(1, 1));
        let span = Marker {
            duration: Some(rat(3, 2)),
            label: "Chorus".into(),
            ..Marker::user(Uuid::now_v7(), rat(2, 1))
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Comp 1".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(rat(10, 1)),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: vec![moment.clone(), span.clone()],
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        apply(
            &mut doc,
            &Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            },
        )
        .unwrap();

        save(&doc, &path).unwrap();
        let (loaded, _) = open(&path).unwrap();
        let markers = match &loaded.items[0] {
            ProjectItem::Composition(c) => c.markers.clone(),
            other => panic!("expected the composition back, got {other:?}"),
        };
        assert_eq!(markers, vec![moment.clone(), span.clone()]);
        assert_eq!(
            markers[1].duration,
            Some(rat(3, 2)),
            "the span is the point of the test"
        );
        assert_eq!(markers[0].duration, None, "and a moment stays a moment");

        // A file written before markers could span: the same project with the
        // key removed from every marker, which is exactly what such a `.lum`
        // holds.
        let older = dir.path().join("older.lum");
        let mut value = document_json(&path);
        let items = value["items"].as_array_mut().unwrap();
        for marker in items[0]["Composition"]["markers"].as_array_mut().unwrap() {
            assert!(
                marker.as_object_mut().unwrap().remove("duration").is_some(),
                "duration was not written, so removing it proves nothing"
            );
        }
        save(&serde_json::from_value::<Document>(value).unwrap(), &older).unwrap();

        let (old, _) = open(&older).unwrap();
        let markers = match &old.items[0] {
            ProjectItem::Composition(c) => c.markers.clone(),
            other => panic!("expected the composition back, got {other:?}"),
        };
        assert!(
            markers.iter().all(|m| m.duration.is_none()),
            "an older file's markers must open as moments, not fail to open"
        );
        assert_eq!(markers[1].label, "Chorus", "and keep everything else");
        assert_eq!(markers[1].kind, MarkerKind::User);
    }

    /// The document JSON inside a `.lum`, as a value a test can pick apart —
    /// the raw file rather than a re-serialised document, so what is checked is
    /// what was actually written.
    fn document_json(path: &Path) -> serde_json::Value {
        use std::io::Read;
        let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut raw = String::new();
        zip.by_name("project.json")
            .unwrap()
            .read_to_string(&mut raw)
            .unwrap();
        serde_json::from_str(&raw).unwrap()
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

    /// **A project written before drivers existed is untouched by them**
    /// (K-471 §4, and the note's first core invariant).
    ///
    /// The `graph` field is additive with a serde default and is skipped when
    /// empty, so a pre-K-471 document opens with an empty graph, its
    /// `project.json` carries no such key, and opening and re-saving reproduces
    /// the same bytes. A wired layer then round-trips whole — drivers, wires and
    /// canvas positions.
    #[test]
    fn an_untouched_project_gains_no_graph_and_re_saves_byte_for_byte() {
        use lumit_core::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};

        let dir = tempfile::tempdir().unwrap();
        let mut doc = doc_with_item();
        let mut comp = lumit_core::model::Composition {
            id: Uuid::now_v7(),
            name: "Comp 1".into(),
            width: 1920,
            height: 1080,
            frame_rate: lumit_core::time::FrameRate::new(25, 1).unwrap(),
            duration: lumit_core::time::Duration(lumit_core::time::Rational::new(10, 1).unwrap()),
            background: lumit_core::model::LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let blur = lumit_core::fx::instantiate("blur").unwrap();
        let blur_id = blur.id;
        let mut layer = lumit_core::model::Layer {
            id: Uuid::now_v7(),
            name: "Solid".into(),
            kind: lumit_core::model::LayerKind::Solid {
                def: Uuid::now_v7(),
            },
            in_point: lumit_core::time::CompTime(lumit_core::time::Rational::ZERO),
            out_point: lumit_core::time::CompTime(lumit_core::time::Rational::new(10, 1).unwrap()),
            start_offset: lumit_core::time::CompTime(lumit_core::time::Rational::ZERO),
            transform: Default::default(),
            matte: None,
            parent: None,
            label: 0,
            markers: Vec::new(),
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: vec![blur],
            graph: LayerGraph::default(),
            switches: Default::default(),
            extra: serde_json::Map::new(),
        };
        let layer_id = layer.id;
        comp.layers.push(layer.clone());
        doc.items
            .push(lumit_core::model::ProjectItem::Composition(comp));

        // 1. The pre-K-471 shape: no `graph` key anywhere in the file.
        let a = dir.path().join("a.lum");
        save(&doc, &a).unwrap();
        let json = String::from_utf8(entry_bytes(&a, "project.json")).unwrap();
        assert!(
            !json.contains("\"graph\""),
            "an untouched project must carry no graph key"
        );

        // 2. Open and re-save: the same bytes, so nothing was invented on load.
        let (reopened, _) = open(&a).unwrap();
        assert!(reopened
            .items
            .iter()
            .filter_map(|i| match i {
                lumit_core::model::ProjectItem::Composition(c) => Some(c),
                _ => None,
            })
            .flat_map(|c| c.layers.iter())
            .all(|l| l.graph.is_empty()));
        let b = dir.path().join("b.lum");
        save(&reopened, &b).unwrap();
        assert_eq!(
            entry_bytes(&a, "project.json"),
            entry_bytes(&b, "project.json"),
            "opening and re-saving an untouched project must reproduce its bytes"
        );

        // 3. A wired layer round-trips whole.
        let mut wiggle = lumit_core::fx::instantiate("wiggle").unwrap();
        wiggle.custom_name = Some("The wobble".into());
        let wiggle_id = wiggle.id;
        layer.graph = LayerGraph {
            nodes: vec![wiggle],
            edges: vec![
                Edge {
                    from: OutputRef::Driver {
                        node: wiggle_id,
                        port: "value".into(),
                    },
                    to: InputRef::Param {
                        node: NodeRef::Effect(blur_id),
                        port: "radius".into(),
                    },
                },
                Edge {
                    from: OutputRef::SourceMatte,
                    to: InputRef::Matte { effect: blur_id },
                },
            ],
            layout: vec![
                (NodeRef::Source, [0.0, 0.0]),
                (NodeRef::Driver(wiggle_id), [120.5, -40.25]),
                (NodeRef::Out, [640.0, 0.0]),
            ],
            exposed: vec![NodeRef::Effect(blur_id)],
        };
        let wanted = layer.graph.clone();
        let mut wired = doc.clone();
        for item in &mut wired.items {
            if let lumit_core::model::ProjectItem::Composition(c) = item {
                c.layers = vec![layer.clone()];
            }
        }
        let c = dir.path().join("c.lum");
        save(&wired, &c).unwrap();
        let (back, _) = open(&c).unwrap();
        let got = back
            .items
            .iter()
            .filter_map(|i| match i {
                lumit_core::model::ProjectItem::Composition(c) => Some(c),
                _ => None,
            })
            .flat_map(|c| c.layers.iter())
            .find(|l| l.id == layer_id)
            .expect("the layer")
            .graph
            .clone();
        assert_eq!(got, wanted, "the whole graph must survive the file");

        // And a wired project re-saves byte for byte as well.
        let d = dir.path().join("d.lum");
        save(&back, &d).unwrap();
        assert_eq!(
            entry_bytes(&c, "project.json"),
            entry_bytes(&d, "project.json")
        );
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
