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
    let plane = planes::warm_jobs(doc)
        .into_iter()
        .find(|job| job.instance == id);
    if let Some(job) = plane {
        let (Some(dir), Some(key)) = (planes::cache_dir(), job.key) else {
            return false;
        };
        let body = sidecar::unframe(bytes, planes::MAGIC, planes::FORMAT_VERSION);
        let record = body.and_then(|body| bincode::deserialize::<planes::Record>(body).ok());
        let Some(run) = record.and_then(planes::record_to_run) else {
            return false;
        };
        planes::write_sidecar(&dir, key, &run);
        planes::run(job, &AtomicBool::new(false));
        return true;
    }
    let brush = roto::warm_jobs(doc)
        .into_iter()
        .find(|job| job.instance == id);
    if let Some(job) = brush {
        let (Some(dir), Some(key)) = (roto::cache_dir(), job.key) else {
            return false;
        };
        let small = bytes.len() as u64 <= roto::MAX_SIDECAR_BYTES;
        let run = small.then(|| roto::decode(bytes, None)).flatten();
        let Some(run) = run.and_then(roto::record_to_run) else {
            return false;
        };
        roto::write_sidecar(&dir, key, &run);
        roto::run(job, &AtomicBool::new(false));
        return true;
    }
    let tracked = track::warm_jobs(doc)
        .into_iter()
        .find(|job| job.media == id);
    let Some(job) = tracked else {
        return false;
    };
    let (Some(dir), Some(key)) = (track::cache_dir(), job.key) else {
        return false;
    };
    // A track is named by the shot and its settings alone, the same on every
    // machine, so the file is kept as it came once it says it is that one.
    if track::decode(bytes, key).is_none() {
        return false;
    }
    sidecar::write(&dir, &key.file_name(), bytes);
    track::run(job, &AtomicBool::new(false));
    true
}
