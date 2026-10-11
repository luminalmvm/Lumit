//! Packing a project's footage into its `.lum` (docs/01-GLOSSARY.md: Packed
//! project): which files a save offers, where an open reads them back out to,
//! and the flag that stops a long one.
//!
//! Runs on the frb worker thread that is saving or opening. No project lock is
//! held across any of it.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use lumit_core::model::{packed_id, MediaRef, ProjectItem};
use lumit_core::Document;
use lumit_project::PackSource;
use uuid::Uuid;

/// Set to stop the pack, unpack, open or import in flight. One flag for the
/// process, because one project is open at a time and these never overlap,
/// and each job clears it as it starts.
static CANCEL: AtomicBool = AtomicBool::new(false);

/// The read-out folders this process has taken, by document
/// (`lumit_project::hold_read_out`). Kept until the process ends and never let
/// go sooner: a project that has been closed may still have a file open, and
/// holding a folder costs one small handle. One entry a document opened.
static HELD: Mutex<BTreeMap<Uuid, File>> = Mutex::new(BTreeMap::new());

/// A job that can be stopped is starting: forget any earlier cancel.
pub(crate) fn begin() {
    CANCEL.store(false, Ordering::Release);
}

pub(crate) fn cancel() {
    CANCEL.store(true, Ordering::Release);
}

pub(crate) fn cancelled() -> bool {
    CANCEL.load(Ordering::Acquire)
}

/// A progress callback for the engine's copies that sends the fraction done
/// to `report` and stops the job once [`cancel`] has been called.
///
/// A copy reports every megabyte. Only a move of half a per cent is passed
/// on, so a ten-gigabyte pack sends two hundred reports and not ten thousand.
pub(crate) fn progress(mut report: impl FnMut(f64)) -> impl FnMut(u64, u64) -> bool {
    let mut sent = -1.0_f64;
    move |done, total| {
        if total > 0 {
            let fraction = done as f64 / total as f64;
            if fraction - sent >= 0.005 || done >= total {
                sent = fraction;
                report(fraction);
            }
        }
        !cancelled()
    }
}

/// Whether this machine switched on packing every save for `document` saved
/// at `path`. The file's own switch counts only when it did
/// (`lumit_project::auto_pack_note`).
pub(crate) fn auto_packs(document: Uuid, path: &Path) -> bool {
    lumit_project::auto_pack_note(document, path).is_some_and(|note| note.is_file())
}

/// Keep the note that says so, or take it away.
pub(crate) fn note_auto_pack(document: Uuid, path: &Path, on: bool) {
    let Some(note) = lumit_project::auto_pack_note(document, path) else {
        return;
    };
    if !on {
        let _ = std::fs::remove_file(note);
    } else if let Some(dir) = note.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(note, []);
    }
}

/// The files a save offers for packing: every footage item's when `all`,
/// otherwise only those of the items the project already packs. Proxies, the
/// colour config and the files effects read are offered by the same rule. An
/// item whose file is not on disk offers nothing.
pub(crate) fn sources(doc: &Document, project_dir: &Path, all: bool) -> Vec<PackSource> {
    // The file a reference names, when it is wanted and on disk.
    let file = |id: Uuid, media: &MediaRef| {
        if !all && !doc.packed.contains_key(&id) {
            return None;
        }
        let path = if media.absolute_path.is_empty() {
            project_dir.join(&media.relative_path)
        } else {
            PathBuf::from(&media.absolute_path)
        };
        path.is_file().then_some(path)
    };
    let footage = doc.items.iter().filter_map(|item| {
        let ProjectItem::Footage(f) = item else {
            return None;
        };
        Some(PackSource {
            item: f.id,
            files: run_files(file(f.id, &f.media)?, f.sequence.is_some()),
        })
    });
    let proxies = doc.proxies.iter().filter_map(|(item, proxy)| {
        let id = packed_id(*item);
        Some(PackSource {
            item: id,
            files: vec![file(id, &proxy.media)?],
        })
    });
    // A config is no use without the look-up tables it names. The ones in its
    // own folder or below go with it. One kept anywhere else has no place in
    // the pack and stays behind.
    let config = doc.colour.config.as_ref().and_then(|config| {
        let id = packed_id(doc.id);
        let path = file(id, config)?;
        let dir = path.parent()?.to_path_buf();
        let tables = lumit_render::colour::config_files(&path)
            .into_iter()
            .filter(|table| {
                table.strip_prefix(&dir).is_ok_and(|below| {
                    below
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                })
            });
        Some(PackSource {
            item: id,
            files: std::iter::once(path).chain(tables).collect(),
        })
    });
    // A file an effect reads, such as a LUT's cube, is a path and nothing else.
    let effects = doc.effect_files().filter_map(|(id, path)| {
        let wanted = all || doc.packed.contains_key(&id);
        (wanted && Path::new(path).is_file()).then(|| PackSource {
            item: id,
            files: vec![PathBuf::from(path)],
        })
    });
    footage
        .chain(proxies)
        .chain(config)
        .chain(effects)
        .collect()
}

/// `first`, followed by the rest of its numbered run when the item is an
/// image sequence. Working out a run is the decoder crate's job, so a build
/// without it packs the one file it can name.
fn run_files(first: PathBuf, sequence: bool) -> Vec<PathBuf> {
    #[cfg(feature = "media")]
    if sequence {
        if let Some(run) = lumit_media::sequence::detect(&first) {
            let rest: Vec<PathBuf> = (0..run.count as usize)
                .map(|n| run.file_at(n))
                .filter(|file| *file != first)
                .collect();
            return std::iter::once(first).chain(rest).collect();
        }
    }
    #[cfg(not(feature = "media"))]
    let _ = sequence;
    vec![first]
}

/// The archives a project's packed footage is read from: the file itself, and
/// for an autosave the project it was written beside, since an autosave
/// carries the document and not the footage.
pub(crate) fn archives(path: &Path) -> Vec<PathBuf> {
    std::iter::once(path.to_path_buf())
        .chain(lumit_project::autosave_owner(path))
        .collect()
}

/// Where a document's packed footage is read out to on this machine. The
/// temp folder stands in on a platform with no home directory.
pub(crate) fn read_out_dir(doc_id: Uuid) -> PathBuf {
    lumit_project::packed_media_dir(doc_id).unwrap_or_else(|| {
        std::env::temp_dir()
            .join("lumit-packed")
            .join(doc_id.to_string())
    })
}

/// Point the packed items of a just-opened `doc` at files to read, copying
/// out of the archive whatever is no longer on disk. `report` hears the
/// fraction copied, and only when something is.
///
/// A failure here is not a failed open: the items it could not place are
/// left for the resolver, and show the relink slate if that finds nothing.
pub(crate) fn restore(
    doc: &mut Document,
    archive: &Path,
    project_dir: &Path,
    mut report: impl FnMut(f64),
) {
    let dest = read_out_dir(doc.id);
    // Taken before anything is read out, so no other Lumit clears the folder
    // away under this one. Every open also clears the folders nobody holds
    // and nobody has opened for a week.
    if let Some(root) = dest.parent() {
        let held = lumit_project::hold_read_out(root, doc.id, !doc.packed.is_empty());
        if let (Some(held), Ok(mut all)) = (held, HELD.lock()) {
            all.insert(doc.id, held);
        }
    }
    if doc.packed.is_empty() {
        return;
    }
    let mut copied = |done: u64, total: u64| {
        if total > 0 {
            report(done as f64 / total as f64);
        }
        !cancelled()
    };
    let _ = lumit_project::restore_packed(doc, &archives(archive), project_dir, &dest, &mut copied);
}
