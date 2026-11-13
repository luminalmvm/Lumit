//! Analyses made on another machine, for a project shared with it: depth and
//! mattes, a Roto brush's mattes, and tracks.
//!
//! Each is kept in this machine's own sidecar folder under the name this
//! machine gives it, so a warm pass reads it as if it had been made here and
//! nobody runs it again. A depth or matte file is named for the pack and the
//! card that made it, which another machine may not share, so it is filed
//! again under this one's name rather than copied.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use lumit_core::model::Document;
use uuid::Uuid;

use crate::{planes, roto, sidecar, track};

/// Every analysis `doc` could be holding: what it is filed under, and where
/// this machine keeps its file, there or not.
#[must_use]
pub fn files(doc: &Document) -> Vec<(Uuid, PathBuf)> {
    let mut out = Vec::new();
    if let Some(dir) = planes::cache_dir() {
        let jobs = planes::warm_jobs(doc).into_iter();
        out.extend(jobs.filter_map(|job| Some((job.instance, dir.join(job.key?.file_name())))));
    }
    if let Some(dir) = roto::cache_dir() {
        let jobs = roto::warm_jobs(doc).into_iter();
        out.extend(jobs.filter_map(|job| Some((job.instance, dir.join(job.key?.file_name())))));
    }
    if let Some(dir) = track::cache_dir() {
        let jobs = track::warm_jobs(doc).into_iter();
        out.extend(jobs.filter_map(|job| Some((job.media, dir.join(job.key?.file_name())))));
    }
    out
}

/// Keep `bytes`, another machine's file for the analysis filed under `id`,
/// as this machine's own, and read it into the store before answering, as
/// opening the project would. False when it is not one, or `doc` has no such
/// analysis.
#[must_use]
pub fn adopt(doc: &Document, id: Uuid, bytes: &[u8]) -> bool {
    let never = AtomicBool::new(false);
    match keep(doc, id, bytes) {
        Some(Kept::Plane(job)) => planes::run(job, &never),
        Some(Kept::Brush(job)) => roto::run(job, &never),
        Some(Kept::Track(job)) => track::run(job, &never),
        None => return false,
    }
    true
}

/// What [`keep`] wrote, to be read into the store.
enum Kept {
    Plane(planes::PlaneJob),
    Brush(roto::RotoJob),
    Track(track::Job),
}

/// Write `bytes` into this machine's sidecar folder for the analysis filed
/// under `id`, and nothing more: the store the open project reads is left as
/// it is. For a file a later export reads, such as one in another process.
/// False when it is not one, or `doc` has no such analysis.
#[must_use]
pub fn write(doc: &Document, id: Uuid, bytes: &[u8]) -> bool {
    keep(doc, id, bytes).is_some()
}

fn keep(doc: &Document, id: Uuid, bytes: &[u8]) -> Option<Kept> {
    let plane = planes::warm_jobs(doc)
        .into_iter()
        .find(|job| job.instance == id);
    if let Some(job) = plane {
        let (dir, key) = (planes::cache_dir()?, job.key?);
        let body = sidecar::unframe(bytes, planes::MAGIC, planes::FORMAT_VERSION);
        let record = body.and_then(|body| bincode::deserialize::<planes::Record>(body).ok());
        let run = record.and_then(planes::record_to_run)?;
        planes::write_sidecar(&dir, key, &run);
        return Some(Kept::Plane(job));
    }
    let brush = roto::warm_jobs(doc)
        .into_iter()
        .find(|job| job.instance == id);
    if let Some(job) = brush {
        let (dir, key) = (roto::cache_dir()?, job.key?);
        let small = bytes.len() as u64 <= roto::MAX_SIDECAR_BYTES;
        let run = small.then(|| roto::decode(bytes, None)).flatten();
        let run = run.and_then(roto::record_to_run)?;
        roto::write_sidecar(&dir, key, &run);
        return Some(Kept::Brush(job));
    }
    let job = track::warm_jobs(doc)
        .into_iter()
        .find(|job| job.media == id)?;
    let (dir, key) = (track::cache_dir()?, job.key?);
    // A track is named by the shot and its settings alone, the same on every
    // machine, so the file is kept as it came once it says it is that one.
    track::decode(bytes, key)?;
    sidecar::write(&dir, &key.file_name(), bytes);
    Some(Kept::Track(job))
}

/// Read every analysis `doc` has in this machine's sidecar folders into the
/// stores, on this thread. Opening a project does the same on threads of its
/// own, which an export that starts straight after would race.
pub fn warm(doc: &Document) {
    let never = AtomicBool::new(false);
    for job in planes::warm_jobs(doc) {
        planes::run(job, &never);
    }
    for job in roto::warm_jobs(doc) {
        roto::run(job, &never);
    }
    for job in track::warm_jobs(doc) {
        track::run(job, &never);
    }
}
