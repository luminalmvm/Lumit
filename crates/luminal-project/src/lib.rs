//! The `.lum` project container, autosave, and the crash-recovery journal —
//! docs/10-FILE-FORMAT.md, Phase 0 scope (no thumbnails yet).

use luminal_core::ops::Op;
use luminal_core::Document;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

pub const FORMAT: &str = "luminal-project";
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
    #[error("not a Luminal project")]
    NotALuminalProject,
    #[error("project needs Luminal {min_reader} or newer (file is schema {schema_version})")]
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
            written_by: format!("luminal {}", env!("CARGO_PKG_VERSION")),
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
            .map_err(|_| ProjectError::NotALuminalProject)?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    if manifest.format != FORMAT {
        return Err(ProjectError::NotALuminalProject);
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
            .map_err(|_| ProjectError::NotALuminalProject)?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
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

/// Where a document's sidecar journal lives (docs/10-FILE-FORMAT.md §3–4).
pub fn journal_path(doc_id: Uuid) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Luminal", "Luminal")?;
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
    let dirs = directories::ProjectDirs::from("dev", "Luminal", "Luminal")?;
    Some(dirs.cache_dir().join("media-index"))
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
    use luminal_core::model::{FootageItem, MediaRef, ProjectItem};
    use luminal_core::ops::apply;

    fn footage(name: &str) -> FootageItem {
        FootageItem {
            id: Uuid::now_v7(),
            name: name.into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: format!("footage/{name}"),
                absolute_path: format!("/tmp/{name}"),
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

        // A "newer Luminal" adds fields this version knows nothing about.
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

    #[test]
    fn too_new_projects_are_refused_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.lum");
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.json", opts).unwrap();
        zip.write_all(
            br#"{"format":"luminal-project","schema_version":"9.0.0","written_by":"luminal 9","min_reader":"9.0.0"}"#,
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
