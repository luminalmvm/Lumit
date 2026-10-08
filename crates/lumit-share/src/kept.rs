//! What a shared project keeps on disk, so closing Lumit loses nobody's work.
//!
//! A host keeps every edit since its project was last saved. A host that
//! closes without saving, or crashes, has them back when it shares the
//! project again, where its guests would otherwise find a document older
//! than the one they were working on.
//!
//! A guest that has lost its host keeps the last document both had and each
//! edit made since. Its own copy of the project, opened again, carries on
//! from there and merges when the host is found.
//!
//! Both are a line of JSON per edit, written as the edit is made and not
//! synced, so they survive Lumit going down and not the machine. A line that
//! does not read is passed over. Written from the share threads and, for a
//! host, from the store's tap.

use crate::host::VERSION;
use lumit_core::{Document, Op};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// An edit and what it replaced where it was made.
pub(crate) type Pair = (Op, Op);

fn file(kind: &str, id: Uuid) -> Option<PathBuf> {
    Some(lumit_project::shared_dir()?.join(format!("{kind}-{id}.jsonl")))
}

fn line(value: &impl Serialize) -> Option<String> {
    let mut line = serde_json::to_string(value).ok()?;
    line.push('\n');
    Some(line)
}

/// Open `path` to add lines to. A last line cut short when Lumit went down
/// is closed first, so the next one is not joined to it.
fn appending(path: &Path) -> Option<File> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .ok()?;
    if file.seek(SeekFrom::End(0)).ok()? > 0 {
        let mut last = [0u8];
        file.seek(SeekFrom::End(-1)).ok()?;
        file.read_exact(&mut last).ok()?;
        if last
            != *b"
"
        {
            file.write_all(
                b"
",
            )
            .ok()?;
        }
    }
    Some(file)
}

/// The edits in `lines`. One that does not read is passed over.
fn pairs(lines: impl Iterator<Item = std::io::Result<String>>) -> Vec<Pair> {
    let lines = lines.map_while(Result::ok);
    lines
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// A host's edits since its project was last saved.
///
/// Bounded by the edits between two saves, as the crash journal is.
pub(crate) struct HostLog {
    path: PathBuf,
    file: Option<File>,
    /// How many edits the file holds.
    count: usize,
}

impl HostLog {
    /// The log for the project `id`, and the edits it already holds. `fresh`
    /// is a host starting on a new invite, which nobody is coming back to, so
    /// whatever was kept under the last one goes.
    pub(crate) fn open(id: Uuid, fresh: bool) -> Option<(Self, Vec<Pair>)> {
        let path = file("host", id)?;
        if fresh {
            let _ = fs::remove_file(&path);
        }
        let held = File::open(&path)
            .map(|file| pairs(BufReader::new(file).lines()))
            .unwrap_or_default();
        let log = HostLog {
            path,
            file: None,
            count: held.len(),
        };
        Some((log, held))
    }

    pub(crate) fn append(&mut self, op: &Op, was: &Op) {
        if self.file.is_none() {
            if let Some(dir) = self.path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            self.file = appending(&self.path);
        }
        let written = self
            .file
            .as_mut()
            .zip(line(&(op, was)))
            .is_some_and(|(file, line)| file.write_all(line.as_bytes()).is_ok());
        if written {
            self.count += 1;
        }
    }

    /// How many edits are held, to hand back to [`Self::saved`].
    pub(crate) fn mark(&self) -> usize {
        self.count
    }

    /// The project was saved with the first `mark` edits in it, so they go.
    pub(crate) fn saved(&mut self, mark: usize) {
        self.file = None;
        let rest: Vec<String> = File::open(&self.path)
            .map(|file| {
                let lines = BufReader::new(file).lines().map_while(Result::ok);
                lines.skip(mark).collect()
            })
            .unwrap_or_default();
        if rest.is_empty() {
            let _ = fs::remove_file(&self.path);
        } else {
            let _ = fs::write(&self.path, rest.join("\n") + "\n");
        }
        self.count = rest.len();
    }

    /// Sharing has stopped for good, so there is nobody to keep them for.
    pub(crate) fn remove(self) {
        drop(self.file);
        let _ = fs::remove_file(&self.path);
    }
}

/// Let go of what a host kept for the project `id`. For a project saved
/// while nobody is sharing it: the file has moved on, and edits kept from
/// before would land on top of work they were never made against.
pub fn forget(id: Uuid) {
    if let Some(path) = file("host", id) {
        let _ = fs::remove_file(path);
    }
}

/// How a guest finds its host again, kept with what it has to merge.
#[derive(Serialize, Deserialize)]
pub(crate) struct Finding {
    /// Edits are the wire format, so a file another version wrote is not read.
    version: String,
    /// The host's own id for the project.
    pub(crate) project: Uuid,
    pub(crate) invite: String,
    pub(crate) name: String,
    pub(crate) root: Option<PathBuf>,
}

impl Finding {
    pub(crate) fn new(project: Uuid, invite: String, name: String, root: Option<PathBuf>) -> Self {
        Finding {
            version: VERSION.to_owned(),
            project,
            invite,
            name,
            root,
        }
    }
}

/// What a guest without its host has kept: the file's first line is how to
/// find the host, its second the last document both had, and the rest the
/// edits made since.
///
/// It grows by an edit at a time while the host is away, and goes when the
/// merge is made or the guest leaves.
pub(crate) struct Kept {
    path: PathBuf,
    file: File,
    /// The document the file's edits follow, to tell when the store has
    /// moved on to another and the file has to be written again.
    pub(crate) base: Arc<Document>,
    /// The file's first line is out of date: the host is looked for by a
    /// new invite. Written again at the next chance.
    pub(crate) stale: bool,
    /// How many edits the file holds.
    pub(crate) written: usize,
}

impl Kept {
    /// Start keeping for the guest's copy `base` is, over whatever was there.
    /// Written beside the file and moved over it, so a file already there is
    /// whole until this one is.
    pub(crate) fn begin(finding: &Finding, base: &Arc<Document>, since: &[Pair]) -> Option<Self> {
        let path = file("guest", base.id)?;
        fs::create_dir_all(path.parent()?).ok()?;
        let fresh = path.with_extension("new");
        fs::write(&fresh, line(finding)? + line(&**base)?.as_str()).ok()?;
        let mut kept = Kept {
            file: appending(&fresh)?,
            path: fresh.clone(),
            base: base.clone(),
            stale: false,
            written: 0,
        };
        kept.append(since);
        fs::rename(&fresh, &path).ok()?;
        kept.path = path;
        Some(kept)
    }

    pub(crate) fn append(&mut self, since: &[Pair]) {
        for pair in since {
            let written = line(pair).is_some_and(|l| self.file.write_all(l.as_bytes()).is_ok());
            if !written {
                return;
            }
            self.written += 1;
        }
    }

    /// What was kept for the guest's copy `copy`, if anything was: how to
    /// find the host, the last document both had, and the edits made since.
    pub(crate) fn read(copy: Uuid) -> Option<(Finding, Document, Vec<Pair>)> {
        let path = file("guest", copy)?;
        let mut lines = BufReader::new(File::open(path).ok()?).lines();
        let finding: Finding = serde_json::from_str(&lines.next()?.ok()?).ok()?;
        if finding.version != VERSION {
            return None;
        }
        let base = serde_json::from_str(&lines.next()?.ok()?).ok()?;
        Some((finding, base, pairs(lines)))
    }

    /// The merge is made, or the guest has left.
    pub(crate) fn remove(self) {
        drop(self.file);
        let _ = fs::remove_file(&self.path);
    }
}
