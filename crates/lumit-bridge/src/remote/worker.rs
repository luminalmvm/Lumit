//! The worker's side of a remote export: taking an offer, gathering the
//! files it lacks, laying them out for the export, following its item in the
//! queue, and giving the finished file back.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lumit_core::model::Document;
use lumit_share::bulk::Wanted;
use serde_json::{json, Value};
use uuid::Uuid;

use super::files::{self, Entry, Kind, Manifest};
use super::job::{self, Job, Lands, Piece, Stage};
use super::sender::read_manifest;
use super::{needs, Io, Remote};
use crate::api::remote::BridgeExportNewsKind;
use crate::export::QueueRowState;

/// What the worker keeps free on the store's disk besides a job's files.
const SPARE: u64 = 1 << 30;

/// How many files and folders the folders it was told to look in are
/// walked through, at most, for one job.
const LOOK_MOST: usize = 200_000;

/// The most one analysis file is read at, the most a roto one may be.
const ANALYSIS_MOST: u64 = 512 << 20;

/// The worker's side of a job's files while it gathers them.
#[derive(Debug, Default, Clone)]
pub(super) struct Gathering {
    pub manifest: Manifest,
    /// The files to take from the sender, by hash, with their sizes.
    wanted: HashMap<String, u64>,
    /// Files that arrived wrong once, and were asked for again.
    retried: HashSet<String>,
    /// Where each file this computer had already is.
    local: HashMap<String, PathBuf>,
    /// The manifest has been read and the files it lists asked for.
    gathered: bool,
}

impl Gathering {
    /// The file a manifest lists under `item`.
    pub(super) fn file(&self, item: Uuid) -> Option<&Entry> {
        self.manifest
            .files
            .iter()
            .find(|f| files::item(&f.hash) == item)
    }

    /// What is still asked for, by item.
    pub(super) fn wanted(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.wanted.keys().map(|hash| files::item(hash))
    }
}

impl Remote {
    /// Another computer offers this one an export to do.
    pub(super) fn offer(
        self: &Arc<Self>,
        id: Uuid,
        from: &str,
        body: &Value,
    ) -> Result<Value, &'static str> {
        if let Some(job) = self.job(id) {
            // Said again: answered as it was the first time.
            if job.other != from || job.sent {
                return Err("bad_request");
            }
            if job.stage == Stage::Done {
                return Ok(json!({ "addresses": [] }));
            }
            if job.stage.ended() {
                return Err(code(&job.error));
            }
            let link = self.start_link(&job, true).map_err(|_| "lost")?;
            return Ok(addresses(&link));
        }
        if !self.takes.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("turned_off");
        }
        let text = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or_default();
        let number = |name: &str| body.get(name).and_then(Value::as_u64).unwrap_or(0);
        let needs: needs::Needs = body
            .get("needs")
            .and_then(|n| serde_json::from_value(n.clone()).ok())
            .unwrap_or_default();
        if needs::older_than(text("schema")) || !needs::lacks(&needs).too_old.is_empty() {
            return Err("too_old");
        }
        let piece = |name: &str| -> Option<Piece> {
            let piece: Piece = serde_json::from_value(body.get(name)?.clone()).ok()?;
            files::is_hash(&piece.hash).then_some(piece)
        };
        let key = text("key");
        let comp = text("comp").parse::<Uuid>().ok();
        let spec = body.get("spec").cloned().unwrap_or(Value::Null);
        let bridged = serde_json::from_value::<lumit_render::export::ExportSpec>(spec.clone())
            .ok()
            .and_then(|s| crate::export::from_export_spec(&s));
        let (Some(manifest), Some(document), Some(comp), Some(mut bridged)) =
            (piece("manifest"), piece("document"), comp, bridged)
        else {
            return Err("bad_request");
        };
        if files::unhex(key).is_none_or(|k| k.len() != 32) {
            return Err("bad_request");
        }
        let bytes = number("bytes");
        let room = crate::framecache::disk::free_bytes(&self.root);
        if room.is_some_and(|free| free < bytes.saturating_add(SPARE)) {
            return Err("no_space");
        }
        let lands = match text("lands") {
            "worker" => Lands::Worker,
            _ => Lands::Sender,
        };
        let file = files::name_of(text("file"));
        let codec = text("codec").to_owned();
        let folder = job::folder(&self.root, id);
        let out = match lands {
            Lands::Sender => folder
                .join("out")
                .join(&file)
                .to_string_lossy()
                .into_owned(),
            Lands::Worker => kept_path(&file, &codec),
        };
        let other_name = self.computer(from).map(|c| c.name).unwrap_or_default();
        let job = Job {
            job: id,
            account: self.account.lock().map(|a| a.clone()).unwrap_or_default(),
            sent: false,
            other: from.to_owned(),
            other_name,
            comp,
            comp_name: text("comp_name").chars().take(200).collect(),
            preset: text("preset").chars().take(200).collect(),
            codec,
            file,
            path: out,
            lands,
            stage: Stage::Sending,
            total: number("frames"),
            created: job::now(),
            key: key.to_owned(),
            spec,
            manifest,
            document,
            all: bytes,
            needs: body.get("needs").cloned().unwrap_or(Value::Null),
            schema: text("schema").to_owned(),
            version: text("version").to_owned(),
            near: body.get("near").and_then(Value::as_bool).unwrap_or(false),
            accepted: true,
            ..Job::default()
        };
        let link = match self.start_link(&job, true) {
            Ok(link) => link,
            Err(_) => return Err("lost"),
        };
        // Named in the queue as the sender named it.
        bridged.preset.clone_from(&job.preset);
        let item = crate::export::queue_add_job(
            id,
            folder,
            comp,
            job.comp_name.clone(),
            &bridged,
            job.path.clone(),
        );
        let job = Job { item, ..job };
        let answer = addresses(&link);
        self.news(BridgeExportNewsKind::Arrived, &job);
        self.begin_gathering(&job);
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(id, job);
        }
        if let Ok(mut dirty) = self.dirty.lock() {
            dirty.insert(id);
        }
        if let Ok(mut queued) = self.queued.lock() {
            queued.insert(id);
        }
        Ok(answer)
    }

    /// Ask for the job's manifest and document, those not here yet, and
    /// gather the rest once both are.
    fn begin_gathering(&self, job: &Job) {
        let folder = job::folder(&self.root, job.job);
        let mut gathering = Gathering::default();
        for (piece, name) in [
            (&job.manifest, "manifest.json"),
            (&job.document, "document.json"),
        ] {
            let here = std::fs::metadata(folder.join(name)).is_ok_and(|m| m.len() == piece.size);
            if !here {
                gathering.wanted.insert(piece.hash.clone(), piece.size);
            }
        }
        let wanted: Vec<Uuid> = gathering.wanted().collect();
        if let Ok(mut all) = self.gathering.lock() {
            all.insert(job.job, gathering);
        }
        if wanted.is_empty() {
            let _ = self.io.try_send(Io::Gather(job.job));
        }
        // A held job asks once it is let go.
        if let Some(link) = self.link(job.job).filter(|_| !job.paused) {
            for item in wanted {
                link.want(Wanted::Original { item });
            }
        }
    }

    /// Whether this computer can export what another computer asks about.
    pub(super) fn check(&self, body: &Value) -> Value {
        let needs: needs::Needs = body
            .get("needs")
            .and_then(|n| serde_json::from_value(n.clone()).ok())
            .unwrap_or_default();
        json!({ "lacks": needs::lacks(&needs), "version": env!("CARGO_PKG_VERSION") })
    }

    /// A file of a job arrived here, on the disk thread: one a sent job is
    /// waiting for, or one this computer was taking for its export.
    pub(super) fn arrived(self: &Arc<Self>, id: Uuid, item: Uuid, path: &Path) {
        match self.job(id) {
            Some(job) if job.sent => self.land(id, item, path),
            Some(job) if !job.stage.ended() => self.verify(&job, item, path),
            _ => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Keep a file that is what the manifest says it is, and ask once more
    /// for one that is not.
    fn verify(self: &Arc<Self>, job: &Job, item: Uuid, path: &Path) {
        let id = job.job;
        // The store's name for it, for a file the manifest lists.
        let expected = [&job.manifest, &job.document]
            .into_iter()
            .find(|p| files::item(&p.hash) == item)
            .map(|p| (p.hash.clone(), p.size, None))
            .or_else(|| {
                let gathering = self.gathering.lock().ok()?;
                let file = gathering.get(&id)?.file(item)?;
                Some((file.hash.clone(), file.size, Some(file.path.clone())))
            });
        let Some((hash, size, stored)) = expected else {
            let _ = std::fs::remove_file(path);
            return;
        };
        let got = files::hash_file(path, &|| self.over(id), &mut |_| {});
        // Ended while it was hashed: nothing more to do with it.
        if self.over(id) {
            let _ = std::fs::remove_file(path);
            return;
        }
        if got.as_deref() != Some(hash.as_str()) {
            let _ = std::fs::remove_file(path);
            let again = self.gathering.lock().is_ok_and(|mut g| {
                g.get_mut(&id)
                    .is_some_and(|g| g.retried.insert(hash.clone()))
            });
            match (again, self.link(id)) {
                (true, Some(link)) => link.want(Wanted::Original { item }),
                _ => self.give_up(id, "lost"),
            }
            return;
        }
        match stored {
            Some(name) => {
                if !files::keep(&self.root, &hash, size, &name, path) {
                    return self.give_up(id, "no_space");
                }
            }
            None => files::touch(path),
        }
        if let Ok(mut moving) = self.moving.lock() {
            moving
                .entry(id)
                .or_default()
                .files
                .insert(item, (size, size));
        }
        let (gathered, left, pieces) = {
            let Ok(mut all) = self.gathering.lock() else {
                return;
            };
            let Some(g) = all.get_mut(&id) else {
                return;
            };
            g.wanted.remove(&hash);
            let pieces = !g.wanted.contains_key(&job.manifest.hash)
                && !g.wanted.contains_key(&job.document.hash);
            (g.gathered, g.wanted.len(), pieces)
        };
        if !gathered && pieces {
            self.gather(id);
        } else if gathered && left == 0 {
            self.lay(id);
        }
    }

    /// Read the manifest, check this Lumit can read the document, and ask
    /// the sender for every file this computer lacks. On the disk thread.
    pub(super) fn gather(self: &Arc<Self>, id: Uuid) {
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        let folder = job::folder(&self.root, id);
        let manifest = read_manifest(&folder.join("manifest.json"));
        let safe = |m: &Manifest| {
            m.files
                .iter()
                .all(|f| files::is_hash(&f.hash) && files::inside(&f.path).is_some())
        };
        let Some(manifest) = manifest.filter(safe) else {
            return self.give_up(id, "lost");
        };
        if read_job_document(&folder, &job.schema).is_none() {
            return self.give_up(id, "too_old");
        }
        let looked = self.look(&manifest);
        let mut local = HashMap::new();
        let mut wanted = HashMap::new();
        let stop = || self.over(id);
        for file in &manifest.files {
            if stop() {
                return;
            }
            if local.contains_key(&file.hash) || wanted.contains_key(&file.hash) {
                continue;
            }
            let same = |path: &Path| {
                std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == file.size)
                    && self.hashes.of(path, &stop, &mut |_| {}).as_deref() == Some(&file.hash)
            };
            // Where the sender has it, only as a plain path on a disk of
            // this computer's: never one that reaches another machine.
            let at_sender = files::local_path(&file.from);
            let found = files::in_store(&self.root, &file.hash, file.size)
                .or_else(|| at_sender.filter(|p| same(p)))
                .or_else(|| {
                    let named = (files::name_of(&file.path).to_lowercase(), file.size);
                    looked.get(&named)?.iter().find(|p| same(p)).cloned()
                });
            match found {
                Some(path) => {
                    local.insert(file.hash.clone(), path);
                }
                None => {
                    wanted.insert(file.hash.clone(), file.size);
                }
            }
        }
        if stop() {
            return;
        }
        let size: u64 = wanted.values().sum();
        let asked: Vec<Uuid> = wanted.keys().map(|h| files::item(h)).collect();
        let done = wanted.is_empty();
        if let Ok(mut all) = self.gathering.lock() {
            let g = all.entry(id).or_default();
            g.manifest = manifest;
            g.local = local;
            g.wanted = wanted;
            g.gathered = true;
        }
        if let Ok(mut moving) = self.moving.lock() {
            moving.remove(&id);
        }
        let held = self.update(id, |j| {
            (j.bytes, j.size) = (0, size);
            j.paused
        });
        // A held job asks once it is let go.
        if let Some(link) = self.link(id).filter(|_| held == Some(false)) {
            for item in asked {
                link.want(Wanted::Original { item });
            }
        }
        if done {
            self.lay(id);
        }
    }

    /// Files under the folders this computer looks in, by lower-case name
    /// and size, of the names `manifest` lists. Walked once for a job.
    fn look(&self, manifest: &Manifest) -> HashMap<(String, u64), Vec<PathBuf>> {
        let named: HashSet<(String, u64)> = manifest
            .files
            .iter()
            .map(|f| (files::name_of(&f.path).to_lowercase(), f.size))
            .collect();
        let folders = self.folders.lock().map(|f| f.clone()).unwrap_or_default();
        let mut found: HashMap<(String, u64), Vec<PathBuf>> = HashMap::new();
        let mut next: Vec<PathBuf> = folders;
        let mut seen = 0usize;
        // ponytail: a bounded walk. A drive of millions of files is looked
        // through only so far, and anything past that is sent instead.
        while let Some(dir) = next.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                seen += 1;
                if seen > LOOK_MOST {
                    return found;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    next.push(entry.path());
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_lowercase();
                let size = entry.metadata().map_or(0, |m| m.len());
                let key = (name, size);
                if named.contains(&key) {
                    found.entry(key).or_default().push(entry.path());
                }
            }
        }
        found
    }

    /// Every file is here: lay them out in the job's folder for its export,
    /// and let its item run. On the disk thread.
    pub(super) fn lay(self: &Arc<Self>, id: Uuid) {
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        let Some(gathering) = self.gathering.lock().ok().and_then(|g| g.get(&id).cloned()) else {
            return;
        };
        let folder = job::folder(&self.root, id);
        let Some(mut doc) = read_job_document(&folder, &job.schema) else {
            return self.give_up(id, "too_old");
        };
        let source = |file: &Entry| {
            gathering
                .local
                .get(&file.hash)
                .cloned()
                .or_else(|| files::in_store(&self.root, &file.hash, file.size))
        };
        // Each reference's files, in the order listed, its first file first.
        let mut by_id: BTreeMap<Uuid, Vec<&Entry>> = BTreeMap::new();
        for file in &gathering.manifest.files {
            by_id.entry(file.id).or_default().push(file);
        }
        let (media, cache) = (folder.join("media"), folder.join("cache"));
        let mut references: BTreeMap<String, String> = BTreeMap::new();
        for (reference, group) in &by_id {
            let Some(first) = group.first() else {
                continue;
            };
            match first.kind {
                Kind::Analysis | Kind::Bake => continue,
                Kind::Footage | Kind::Proxy | Kind::Config | Kind::Effect => {}
            }
            // Where every file is at the same place here as on the sender,
            // the export reads it there.
            let in_place = group.iter().all(|f| {
                gathering
                    .local
                    .get(&f.hash)
                    .is_some_and(|p| *p == Path::new(&f.from))
            });
            if in_place {
                references.insert(reference.to_string(), first.from.clone());
                continue;
            }
            let base = media.join(reference.to_string());
            for file in group {
                let (Some(from), Some(below)) = (source(file), files::inside(&file.path)) else {
                    return self.give_up(id, "lost");
                };
                if !files::place(&from, &base.join(below)) {
                    return self.give_up(id, "no_space");
                }
            }
            let first_at = files::inside(&first.path)
                .map(|p| base.join(p))
                .unwrap_or(base);
            references.insert(
                reference.to_string(),
                first_at.to_string_lossy().into_owned(),
            );
        }
        for (reference, path) in &references {
            if let Some(slot) = reference.parse().ok().and_then(|r| doc.packed_path_mut(r)) {
                slot.clone_from(path);
            }
        }
        for file in &gathering.manifest.files {
            let Some(from) = source(file) else {
                return self.give_up(id, "lost");
            };
            match file.kind {
                // Filed here under this computer's own names, for the
                // export's warm pass to read. The open project's stores are
                // left alone.
                Kind::Analysis => {
                    let bytes = lumit_ingress::read_capped(&from, ANALYSIS_MOST);
                    let kept = bytes.is_ok_and(|b| lumit_render::sent::write(&doc, file.id, &b));
                    if !kept {
                        note!("remote: an analysis for {} could not be kept", file.id);
                    }
                }
                Kind::Bake => {
                    let Some(below) = files::inside(&file.path) else {
                        continue;
                    };
                    let to = cache.join("sim").join(file.id.to_string()).join(below);
                    if !files::place(&from, &to) {
                        return self.give_up(id, "no_space");
                    }
                }
                _ => {}
            }
        }
        let order = json!({
            "comp": job.comp,
            "spec": job.spec,
            "out": job.path,
            "schema": job.schema,
            "references": references,
            "cache": cache,
        });
        if let Some(out) = Path::new(&job.path).parent() {
            let _ = std::fs::create_dir_all(out);
        }
        let written = serde_json::to_vec_pretty(&order)
            .ok()
            .is_some_and(|text| std::fs::write(folder.join("job.json"), text).is_ok());
        if !written {
            return self.give_up(id, "no_space");
        }
        let laid = self.update(id, |j| {
            // Ended meanwhile, it keeps the end it had.
            if j.stage.ended() {
                return false;
            }
            j.bytes = j.size;
            j.stage = Stage::Queued;
            true
        });
        if laid != Some(true) {
            return;
        }
        crate::export::queue_job_ready(job.item);
        let keep = self.store_needed();
        files::trim(&self.root, &keep, false);
    }

    /// Follow each job's item in the export queue.
    pub(super) fn follow_queue(&self, rows: &[crate::export::QueueRow]) {
        let account = self.account.lock().map(|a| a.clone()).unwrap_or_default();
        let live: Vec<Job> = self.jobs.lock().map_or_else(
            |_| Vec::new(),
            |jobs| {
                let ours = jobs.values().filter(|j| !j.sent && j.account == account);
                ours.filter(|j| !j.stage.ended()).cloned().collect()
            },
        );
        let queued = self.queued.lock().map(|q| q.clone()).unwrap_or_default();
        for job in live.iter().filter(|j| queued.contains(&j.job)) {
            let id = job.job;
            let row = rows.iter().find(|r| r.job == Some(id));
            match row.map(|r| &r.state) {
                // A finished row cleared away while its file goes back: the
                // file still goes.
                None if job.stage == Stage::Returning => {}
                // Cleared away once done, before this saw it done: the file
                // goes back, or stays, all the same.
                None if crate::export::queue_job_landed(id) => self.exported_here(job),
                // Taken out of the queue here: cancelled, on both computers.
                None => self.cancel_here(id, true),
                // Its stage is the files' to say until it runs.
                Some(QueueRowState::Waiting) => {}
                Some(QueueRowState::Running {
                    frame,
                    total,
                    paused,
                    ..
                }) => {
                    self.progress(id, |j| {
                        if !j.stage.ended() {
                            j.stage = Stage::Exporting;
                            (j.frame, j.total) = (*frame, *total);
                        }
                    });
                    // Held or let go in the queue window here.
                    if job.paused != *paused {
                        self.update(id, |j| j.paused = *paused);
                    }
                }
                // Once, as it lands: the file then goes back, or stays.
                Some(QueueRowState::Done { .. }) if job.stage != Stage::Returning => {
                    self.exported_here(job);
                }
                Some(QueueRowState::Done { .. }) => {}
                Some(QueueRowState::Failed(error)) if error == "cancelled" => {
                    self.cancel_here(id, true);
                }
                Some(QueueRowState::Failed(error)) => {
                    let error: String = error.chars().take(200).collect();
                    self.update(id, |j| j.end(Stage::Failed, &error));
                    self.stopped(id);
                    self.news(BridgeExportNewsKind::RemoteFailed, job);
                }
            }
        }
    }

    /// A job's export is done here: its file goes back, or stays.
    fn exported_here(&self, job: &Job) {
        let id = job.job;
        let Some(size) = self.write_out_list(job) else {
            return self.give_up(id, "failed");
        };
        if let Ok(mut moving) = self.moving.lock() {
            moving.remove(&id);
        }
        match job.lands {
            Lands::Sender => {
                self.update(id, |j| {
                    j.stage = Stage::Returning;
                    j.frame = j.total;
                    (j.bytes, j.size) = (0, size);
                });
            }
            Lands::Worker => {
                self.update(id, |j| {
                    j.frame = j.total;
                    j.end(Stage::Done, "");
                });
                self.stopped(id);
                let _ = self.io.try_send(Io::Forget(id));
            }
        }
    }

    /// Write the list of what a job's export wrote, for the sender to fetch.
    /// Answers how many bytes it lists.
    fn write_out_list(&self, job: &Job) -> Option<u64> {
        let out = PathBuf::from(&job.path);
        let first = crate::export::first_file(&job.path, &job.codec);
        let sequence = first != out;
        let mut listed = Vec::new();
        let mut entry = |n: usize, path: PathBuf| -> bool {
            let Ok(meta) = std::fs::metadata(&path) else {
                return false;
            };
            let name = files::name_of(&path.to_string_lossy());
            listed.push(json!({
                "item": Uuid::new_v5(&job.job, name.as_bytes()),
                "name": name,
                "n": n,
                "bytes": meta.len(),
                "path": path,
            }));
            true
        };
        if sequence {
            let mut n = 1;
            while entry(n, lumit_media::encode::sequence_frame_path(&out, n)) {
                n += 1;
            }
        } else {
            entry(0, out);
        }
        if listed.is_empty() {
            return None;
        }
        let size = listed.iter().filter_map(|f| f.get("bytes")?.as_u64()).sum();
        let list = json!({ "sequence": sequence, "files": listed });
        let at = job::folder(&self.root, job.job).join("out-list.json");
        std::fs::write(at, list.to_string()).ok()?;
        Some(size)
    }

    /// Keep a worker's job going after a restart: its link, its gathering
    /// and its place in the queue.
    pub(super) fn keep_linked(self: &Arc<Self>, job: &Job) {
        let id = job.job;
        // As it stands now, which may have ended earlier in this turn.
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        let job = &job;
        if self.start_link(job, true).is_err() {
            return self.give_up(id, "lost");
        }
        // Ended on the disk thread meanwhile: no link is left behind for it.
        if self.over(id) {
            return self.unlink(id);
        }
        let gathering = self.gathering.lock().is_ok_and(|g| g.contains_key(&id));
        if job.stage == Stage::Returning {
            return;
        }
        if !gathering {
            self.begin_gathering(job);
        }
        let queued = self.queued.lock().is_ok_and(|q| q.contains(&id));
        if queued {
            return;
        }
        let spec = serde_json::from_value::<lumit_render::export::ExportSpec>(job.spec.clone())
            .ok()
            .and_then(|s| crate::export::from_export_spec(&s));
        let Some(mut spec) = spec else {
            return self.give_up(id, "failed");
        };
        spec.preset.clone_from(&job.preset);
        let folder = job::folder(&self.root, id);
        let item = crate::export::queue_add_job(
            id,
            folder.clone(),
            job.comp,
            job.comp_name.clone(),
            &spec,
            job.path.clone(),
        );
        if job.paused {
            crate::export::queue_pause(item, true);
        }
        let ready = folder.join("job.json").is_file()
            && matches!(job.stage, Stage::Queued | Stage::Exporting | Stage::Paused);
        if ready {
            crate::export::queue_job_ready(item);
        }
        self.update(id, |j| {
            j.item = item;
            if j.stage == Stage::Exporting {
                j.stage = Stage::Queued;
            }
        });
        if let Ok(mut queued) = self.queued.lock() {
            queued.insert(id);
        }
    }

    /// Follow the room's list: a job done here ends once the sender's mirror
    /// of it says it has, though the sender's word of that never came. The
    /// room keeps what a computer last said while it is away, so this holds
    /// when either starts again, and when this one stopped taking exports,
    /// which the room then answers for it.
    pub(super) fn follow_mirrors(&self) {
        let ended: Vec<(Uuid, String, bool, String)> = {
            let Ok(room) = self.room.lock() else {
                return;
            };
            let me = room.me.as_str();
            let mirrors = room
                .computers
                .iter()
                .flat_map(|c| c.mirrors.iter().map(move |m| (c, m)));
            mirrors
                .filter_map(|(c, mirror)| {
                    let text = |name: &str| mirror.get(name).and_then(Value::as_str);
                    if me.is_empty() || text("on") != Some(me) {
                        return None;
                    }
                    let job = text("job")?.parse::<Uuid>().ok()?;
                    let landed = text("stage") == Some("landed");
                    let failed = text("state") == Some("failed");
                    let error = text("error").filter(|_| failed).unwrap_or_default();
                    (landed || failed).then(|| (job, c.id.clone(), landed, error.to_owned()))
                })
                .collect()
        };
        for (id, from, landed, error) in ended {
            let ours = self.job(id).filter(|j| !j.sent && j.other == from);
            if ours.is_none_or(|j| j.stage.ended()) {
                continue;
            }
            if landed {
                self.landed_there(id);
            } else {
                self.ended_there(id, &error);
            }
        }
    }

    /// The other computer ended `id`: cancelled, or given up on for `error`.
    pub(super) fn ended_there(&self, id: Uuid, error: &str) {
        if error.is_empty() || error == "cancelled" {
            self.cancel_here(id, false);
        } else {
            self.fail(id, error);
        }
    }

    /// The sender has the file: the copy here goes.
    pub(super) fn landed_there(&self, id: Uuid) {
        let Some(job) = self.job(id) else {
            return;
        };
        self.unlink(id);
        if job.lands == Lands::Sender && !job.stage.ended() {
            self.update(id, |j| {
                j.bytes = j.size;
                j.end(Stage::Done, "");
            });
            self.stopped(id);
            let _ = self.io.try_send(Io::Forget(id));
        }
    }

    /// How a job stands, as the sender asks: and where its link listens now,
    /// which moves when this computer starts again. A kept file the sender
    /// is `fetching` has its link made ready again, should it have gone.
    pub(super) fn status_here(self: &Arc<Self>, job: &Job, fetching: bool) -> Value {
        let kept = job.lands == Lands::Worker && job.stage == Stage::Done;
        if fetching && kept && self.link(job.job).is_none() {
            let _ = self.fetch_here(job.job);
        }
        let mut view = job.view();
        if let (Some(link), Some(fields)) = (self.link(job.job), view.as_object_mut()) {
            fields.insert("addresses".into(), json!(listening(&link)));
        }
        view
    }

    /// The sender wants the file this computer kept: answer where the link
    /// to fetch it on listens.
    pub(super) fn fetch_here(self: &Arc<Self>, id: Uuid) -> Result<Value, &'static str> {
        let job = self.job(id).ok_or("not_found")?;
        // A sequence's path names no one file: its first frame is asked about.
        let first = crate::export::first_file(&job.path, &job.codec);
        if job.lands != Lands::Worker || job.stage != Stage::Done || !first.exists() {
            return Err("not_found");
        }
        self.write_out_list(&job).ok_or("not_found")?;
        let link = self.start_link(&job, true).map_err(|_| "lost")?;
        Ok(addresses(&link))
    }
}

/// Where a link listens, as an answer says it.
fn addresses(link: &lumit_share::remote::Link) -> Value {
    json!({ "addresses": listening(link) })
}

fn listening(link: &lumit_share::remote::Link) -> Vec<String> {
    link.addresses().iter().map(ToString::to_string).collect()
}

/// The error code a job ended with, as an answer gives it.
fn code(error: &str) -> &'static str {
    match error {
        "turned_off" => "turned_off",
        "too_old" => "too_old",
        "no_space" => "no_space",
        "cancelled" => "cancelled",
        _ => "lost",
    }
}

/// Where a job whose file stays here writes it: the folder Settings > Export
/// names, or the videos folder's Lumit, never over a file already there.
fn kept_path(file: &str, codec: &str) -> String {
    let defaults = crate::export::defaults_get();
    let folder = if defaults.destination == "folder" && !defaults.folder.is_empty() {
        PathBuf::from(&defaults.folder)
    } else {
        lumit_project::videos::dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Lumit")
    };
    let _ = std::fs::create_dir_all(&folder);
    let path = folder.join(file).to_string_lossy().into_owned();
    let burn = crate::api::export::BridgeCaptionsOut::Burn;
    if crate::export::taken::path_taken(&path, codec, burn) {
        crate::export::taken::free_path(&path, codec, burn)
    } else {
        path
    }
}

/// A job's document as this Lumit reads it: brought up from the schema it
/// was written at. `None` when it can't be read.
pub(super) fn read_job_document(folder: &Path, schema: &str) -> Option<Document> {
    let path = folder.join("document.json");
    let text = lumit_ingress::read_to_string_capped(&path, files::DOCUMENT_MOST).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let value = lumit_project::migrate(value, schema);
    let mut doc: Document = serde_json::from_value(value).ok()?;
    lumit_project::forward_migrate(&mut doc).ok()?;
    Some(doc)
}
