//! Packing a project's footage into its `.lum` (docs/01-GLOSSARY.md: Packed
//! project): which files a save offers, where an open reads them back out to,
//! and the flag that stops a long one.
//!
//! Runs on the frb worker thread that is saving or opening. No project lock is
//! held across any of it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use lumit_core::model::ProjectItem;
use lumit_core::Document;
use lumit_project::PackSource;
use uuid::Uuid;

/// Set to stop the pack or unpack in flight. One flag for the process, because
/// one project is open at a time, and each job clears it as it starts.
static CANCEL: AtomicBool = AtomicBool::new(false);

/// A pack or an unpack is starting: forget any earlier cancel.
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

/// The files a save offers for packing: every footage item's when `all`,
/// otherwise only those of the items the project already packs. An item whose
/// file is not on disk offers nothing.
pub(crate) fn sources(doc: &Document, project_dir: &Path, all: bool) -> Vec<PackSource> {
    doc.items
        .iter()
        .filter_map(|item| {
            let ProjectItem::Footage(f) = item else {
                return None;
            };
            if !all && !doc.packed.contains_key(&f.id) {
                return None;
            }
            let path = if f.media.absolute_path.is_empty() {
                project_dir.join(&f.media.relative_path)
            } else {
                PathBuf::from(&f.media.absolute_path)
            };
            path.is_file().then(|| PackSource {
                item: f.id,
                files: run_files(path, f.sequence.is_some()),
            })
        })
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
    if doc.packed.is_empty() {
        return;
    }
    let dest = read_out_dir(doc.id);
    // An open is not cancelled from here, so the flag is not read.
    let mut copied = |done: u64, total: u64| {
        if total > 0 {
            report(done as f64 / total as f64);
        }
        true
    };
    let _ = lumit_project::restore_packed(doc, &archives(archive), project_dir, &dest, &mut copied);
}
