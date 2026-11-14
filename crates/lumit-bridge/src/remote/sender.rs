//! The sender's side of a remote export: choosing a computer, working out
//! the project's files, offering the job, following it, and landing the
//! file that comes back. And what either computer does to a job it has:
//! cancel it, hold it, forget it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lumit_core::model::Document;
use lumit_share::bulk::Wanted;
use serde_json::{json, Value};
use uuid::Uuid;
use web_time::{Duration, Instant};

use super::files::{self, Entry, Manifest};
use super::job::{self, Job, Lands, Piece, Stage};
use super::room::{Computer, For};
use super::{end, needs, Io, Remote};
use crate::api::composition::CompositionReference;
use crate::api::export::{BridgeCaptionsOut, BridgeExportSpec};
use crate::api::remote::{BridgeExportNewsKind, BridgeRemoteFit};
use crate::api::BridgeError;

/// How often the sender asks the worker how a job stands.
const ASK_EVERY: Duration = Duration::from_secs(3);

/// How often the sender asks again for a finished export's files.
const WANT_EVERY: Duration = Duration::from_secs(5);

/// What a job's files are worked out from.
pub(super) enum Snapshot {
    /// The project as it stood when it was sent.
    Project {
        doc: Arc<Document>,
        project_dir: PathBuf,
        bakes: Option<PathBuf>,
    },
    /// The snapshot of an earlier job, sent again.
    Job(Uuid),
    /// Its own snapshot, after a restart.
    Kept,
}

impl Remote {
    // --- Sending -----------------------------------------------------------

    pub(super) fn send(
        self: &Arc<Self>,
        comp: &CompositionReference,
        computer: &str,
        spec: &BridgeExportSpec,
        path: &str,
        lands: Lands,
    ) -> Result<String, BridgeError> {
        if path.trim().is_empty() {
            return Err(BridgeError::NoProjectPath);
        }
        let project = comp.project()?;
        let (doc, revision, project_dir, bakes) = {
            let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
            let (doc, revision) = state.store.snapshot_and_revision();
            let dir = state.path.as_deref().and_then(Path::parent);
            let (_, location) = crate::framecache::disk::location();
            let bakes = crate::project_cache::root(&state, &location).map(|r| r.join("sim"));
            (
                doc,
                revision,
                dir.map(Path::to_path_buf).unwrap_or_default(),
                bakes,
            )
        };
        let found = doc.comp(comp.id).ok_or(BridgeError::InvalidComp)?;
        let mut engine = crate::export::to_export_spec(spec, found.width, found.height)
            .map_err(BridgeError::ExportFailed)?;
        // Only the main file is written there, as only one file comes back.
        engine.also.clear();
        // What happens when it lands is this computer's to do.
        engine.when_done = lumit_render::export::WhenDone::Nothing;
        let frames = if spec.range_end_frame > spec.range_start_frame && spec.range_start_frame >= 0
        {
            spec.range_end_frame - spec.range_start_frame
        } else {
            found
                .frame_rate
                .frame_at(lumit_core::time::CompTime(found.duration.0))
        };
        let other = self.choose(computer, comp.id, revision)?;
        let out = crate::export::with_format_extension(path, &spec.codec);
        // Only the file itself comes back, never captions beside it.
        let overwrite =
            crate::export::taken::path_taken(&out, &spec.codec, BridgeCaptionsOut::Burn, &[]);
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|_| BridgeError::WriteFailed)?;
        let job = Job {
            job: Uuid::now_v7(),
            account: self.account.lock().map(|a| a.clone()).unwrap_or_default(),
            sent: true,
            other: other.id,
            other_name: other.name,
            comp: comp.id,
            comp_name: found.name.clone(),
            preset: spec.preset.clone(),
            codec: spec.codec.clone(),
            file: files::name_of(&out),
            path: out,
            overwrite,
            lands,
            stage: Stage::Checking,
            total: u64::try_from(frames).unwrap_or(0),
            created: job::now(),
            item: self
                .next_mirror
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            key: files::hex(&key),
            spec: serde_json::to_value(&engine).unwrap_or(Value::Null),
            noise: spec.make_a_noise,
            reveal: spec.open_folder,
            project_dir: project_dir.to_string_lossy().into_owned(),
            bakes: bakes
                .as_ref()
                .map(|b| b.to_string_lossy().into_owned())
                .unwrap_or_default(),
            schema: lumit_project::SCHEMA_VERSION.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            near: other.near,
            ..Job::default()
        };
        let id = job.job;
        self.add(job);
        let from = Snapshot::Project {
            doc,
            project_dir,
            bakes,
        };
        self.io
            .try_send(Io::Prepare(id, from))
            .map_err(|_| BridgeError::WriteFailed)?;
        Ok(id.to_string())
    }

    /// Keep a new sent job, whose files are being worked out.
    fn add(&self, job: Job) {
        let id = job.job;
        if let Ok(mut started) = self.job_started.lock() {
            started.insert(id);
        }
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(id, job);
        }
        if let Ok(mut dirty) = self.dirty.lock() {
            dirty.insert(id);
        }
    }

    /// The computer to send to: `computer`, or with none the best of those
    /// online that take exports and can read the project. Idle before busy,
    /// near before far, then the most graphics memory.
    fn choose(&self, computer: &str, comp: Uuid, revision: u64) -> Result<Computer, BridgeError> {
        let room = self.room.lock().map_err(|_| BridgeError::ReadFailed)?;
        if !computer.is_empty() {
            let found = room.find(computer).filter(|c| c.id != room.me);
            return found.cloned().ok_or(BridgeError::NoComputer);
        }
        let fits = self.fits.lock().map_err(|_| BridgeError::ReadFailed)?;
        let too_old = |c: &Computer| {
            let fit = fits.get(&(c.id.clone(), comp, revision));
            matches!(fit, Some(BridgeRemoteFit::TooOld { .. }))
        };
        let mut able: Vec<&Computer> = room
            .computers
            .iter()
            .filter(|c| c.online && c.takes && !too_old(c))
            .collect();
        able.sort_by_key(|c| {
            let busy = c.exporting || c.queued > 0;
            (busy, !c.near, std::cmp::Reverse(c.gpu_memory))
        });
        able.first()
            .map(|c| (*c).clone())
            .ok_or(BridgeError::NoComputer)
    }

    /// Work out a sent job's files and their hashes, on the disk thread.
    pub(super) fn prepare(self: &Arc<Self>, id: Uuid, from: Snapshot) {
        let Some(job) = self.job(id) else {
            return;
        };
        let dir = job::folder(&self.root, id);
        let _ = std::fs::create_dir_all(&dir);
        let document = dir.join("document.json");
        let manifest = dir.join("manifest.json");
        let (found, needs) = match from {
            Snapshot::Project {
                doc,
                project_dir,
                bakes,
            } => {
                // As a project file holds it: no path of this computer's.
                let written = serde_json::to_vec(&*doc)
                    .ok()
                    .is_some_and(|text| std::fs::write(&document, text).is_ok());
                if !written {
                    return self.give_up(id, "failed");
                }
                let found = files::needed(&doc, job.comp, &project_dir, bakes.as_deref());
                (found, needs::of(&doc, job.comp))
            }
            Snapshot::Job(old) => {
                let old = job::folder(&self.root, old);
                let copied = std::fs::copy(old.join("document.json"), &document).is_ok()
                    && std::fs::copy(old.join("manifest.json"), &manifest).is_ok();
                let Some(found) = listed(&manifest).filter(|_| copied) else {
                    return self.give_up(id, "failed");
                };
                (
                    found,
                    serde_json::from_value(job.needs.clone()).unwrap_or_default(),
                )
            }
            // Only a manifest written whole. One cut short by a crash, or
            // never written, is worked out again.
            Snapshot::Kept => match listed(&manifest) {
                Some(found) => (
                    found,
                    serde_json::from_value(job.needs.clone()).unwrap_or_default(),
                ),
                None => {
                    let Some(mut doc) = read_document(&document) else {
                        return self.give_up(id, "failed");
                    };
                    let project_dir = PathBuf::from(&job.project_dir);
                    lumit_project::resolve_all_media(&mut doc, &project_dir, &[]);
                    let bakes = (!job.bakes.is_empty()).then(|| PathBuf::from(&job.bakes));
                    let found = files::needed(&doc, job.comp, &project_dir, bakes.as_deref());
                    (found, needs::of(&doc, job.comp))
                }
            },
        };
        // Hashed, saying how far it has got.
        let size: u64 = found
            .iter()
            .filter_map(|f| std::fs::metadata(&f.from).ok())
            .map(|m| m.len())
            .sum();
        self.progress(id, |j| {
            j.stage = Stage::Checking;
            (j.bytes, j.size) = (0, size);
        });
        let stop = || {
            self.stop.load(std::sync::atomic::Ordering::Relaxed)
                || self.job(id).is_none_or(|j| j.stage.ended())
        };
        let mut done = 0u64;
        let mut entries = Vec::new();
        for file in found {
            let mut moved = |n: u64| {
                done += n;
                self.progress(id, |j| j.bytes = done);
            };
            let hash = self.hashes.of(&file.from, &stop, &mut moved);
            if stop() {
                return;
            }
            // A file that went meanwhile is left out, as one missing is.
            let (Some(hash), Ok(meta)) = (hash, std::fs::metadata(&file.from)) else {
                continue;
            };
            entries.push(Entry {
                hash,
                size: meta.len(),
                kind: file.kind,
                id: file.id,
                path: file.path,
                from: file.from.to_string_lossy().into_owned(),
            });
        }
        let listed = Manifest { files: entries };
        // To one side first, so a manifest is always whole.
        let making = dir.join("manifest.json.new");
        let written = serde_json::to_vec(&listed).ok().is_some_and(|text| {
            std::fs::write(&making, text).is_ok() && std::fs::rename(&making, &manifest).is_ok()
        });
        let piece = |path: &Path| {
            let size = std::fs::metadata(path).ok()?.len();
            let hash = files::hash_file(path, &|| false, &mut |_| {})?;
            Some(Piece { hash, size })
        };
        let (Some(manifest), Some(document), true) = (piece(&manifest), piece(&document), written)
        else {
            return self.give_up(id, "failed");
        };
        let all = listed.files.iter().map(|f| f.size).sum::<u64>() + manifest.size + document.size;
        self.hold(id, &listed, &manifest, &document);
        self.update(id, |j| {
            j.manifest = manifest;
            j.document = document;
            j.all = all;
            j.needs = serde_json::to_value(&needs).unwrap_or(Value::Null);
            (j.bytes, j.size) = (0, all);
        });
    }

    /// Keep what the sender holds for `job`, by the item each is asked for as.
    fn hold(&self, id: Uuid, listed: &Manifest, manifest: &Piece, document: &Piece) {
        let dir = job::folder(&self.root, id);
        let mut holding: HashMap<Uuid, (PathBuf, u64)> = listed
            .files
            .iter()
            .map(|f| (files::item(&f.hash), (PathBuf::from(&f.from), f.size)))
            .collect();
        holding.insert(
            files::item(&manifest.hash),
            (dir.join("manifest.json"), manifest.size),
        );
        holding.insert(
            files::item(&document.hash),
            (dir.join("document.json"), document.size),
        );
        if let Ok(mut all) = self.holding.lock() {
            all.insert(id, Arc::new(holding));
        }
    }

    /// What the sender holds for `job`, for its link to say.
    pub(super) fn holds(&self, id: Uuid) -> Vec<lumit_share::bulk::Held> {
        let holding = self.holding.lock().ok().and_then(|h| h.get(&id).cloned());
        holding.map(|h| end::holds(&h)).unwrap_or_default()
    }

    /// Move a sent job on, once a turn.
    pub(super) fn drive(self: &Arc<Self>, job: &Job) {
        let id = job.job;
        // As it stands now, which may have ended earlier in this turn.
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        let job = &job;
        if job.manifest.hash.is_empty() {
            // Its files are being worked out, or were when Lumit last quit.
            if self.job_started.lock().is_ok_and(|mut s| s.insert(id)) {
                let _ = self.io.try_send(Io::Prepare(id, Snapshot::Kept));
            }
            return;
        }
        let online = self.computer(&job.other).is_some_and(|c| c.online);
        // Held before it went, it waits here until it is let go.
        if !job.accepted {
            if !online {
                self.progress(id, |j| j.stage = Stage::Away);
            } else if !job.paused && !self.asking(&For::Offer(id)) {
                self.progress(id, |j| j.stage = Stage::Checking);
                let body = self.offer_body(job);
                self.say(&job.other, body, For::Offer(id));
            }
            return;
        }
        if self.holding.lock().is_ok_and(|h| !h.contains_key(&id)) {
            let dir = job::folder(&self.root, id);
            if let Some(listed) = read_manifest(&dir.join("manifest.json")) {
                self.hold(id, &listed, &job.manifest, &job.document);
            }
        }
        let Ok(link) = self.start_link(job, false) else {
            return self.give_up(id, "lost");
        };
        // Ended on the disk thread meanwhile: no link is left behind for it.
        if self.over(id) {
            return self.unlink(id);
        }
        if !online {
            self.progress(id, |j| j.stage = Stage::Away);
            return;
        }
        // Back from away to the step it was at. A file on its way here goes
        // on coming, which the worker's word won't say for one it kept.
        if job.stage == Stage::Away {
            let back = if job.fetching || !job.landing.is_empty() {
                Stage::Returning
            } else {
                Stage::Sending
            };
            self.progress(id, |j| j.stage = back);
        }
        let due = |map: &std::sync::Mutex<HashMap<Uuid, Instant>>, every: Duration| {
            let Ok(mut map) = map.lock() else {
                return false;
            };
            let due = map.get(&id).is_none_or(|at| at.elapsed() >= every);
            if due {
                map.insert(id, Instant::now());
            }
            due
        };
        // Asked while held too, so a hold let go at the worker is heard.
        if !self.asking(&For::Status(id)) && due(&self.polled, ASK_EVERY) {
            let body = json!({ "lumit": "remote-status", "job": id, "fetching": job.fetching });
            self.say(&job.other, body, For::Status(id));
        }
        if job.stage == Stage::Returning && !job.paused && due(&self.asked, WANT_EVERY) {
            if job.landing.is_empty() {
                link.want(Wanted::Export { job: id });
            }
            for (item, ..) in &job.landing {
                link.want(Wanted::Original { item: *item });
            }
        }
    }

    fn offer_body(&self, job: &Job) -> Value {
        json!({
            "lumit": "remote-offer",
            "job": job.job,
            "key": job.key,
            "comp": job.comp,
            "comp_name": job.comp_name,
            "file": job.file,
            "preset": job.preset,
            "codec": job.codec,
            "frames": job.total,
            "bytes": job.all,
            "lands": job.lands,
            "needs": job.needs,
            "schema": job.schema,
            "version": job.version,
            "manifest": job.manifest,
            "document": job.document,
            "near": job.near,
            "spec": job.spec,
        })
    }

    /// The worker answered an offer.
    pub(super) fn offered(self: &Arc<Self>, id: Uuid, body: Option<&Value>, error: &str) {
        if !error.is_empty() {
            return self.fail(id, error);
        }
        let addresses = listed_addresses(body);
        self.update(id, |j| {
            j.accepted = true;
            j.addresses = addresses;
            j.stage = Stage::Sending;
        });
    }

    /// The other computer is away: a job waiting on it says so.
    pub(super) fn away(&self, what: &For) {
        if let For::Offer(id) = what {
            self.progress(*id, |j| j.stage = Stage::Away);
        }
    }

    /// End a job of this computer's as failed, as the other computer said.
    pub(super) fn fail(&self, id: Uuid, error: &str) {
        let Some(j) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        if self.unfetch(&j, error) {
            return;
        }
        let stage = if error == "cancelled" {
            Stage::Cancelled
        } else {
            Stage::Failed
        };
        let error: String = error.chars().take(200).collect();
        self.update(id, |j| j.end(stage, &error));
        self.stopped(id);
        self.forget_landing(&j);
        if let Some(j) = self.job(id) {
            self.news(BridgeExportNewsKind::RemoteFailed, &j);
        }
    }

    /// The worker said how a job stands.
    pub(super) fn status(self: &Arc<Self>, id: Uuid, body: Option<&Value>, error: &str) {
        if error == "not_found" {
            // A kept file it no longer knows of has gone with it.
            match self.job(id).filter(|j| j.accepted) {
                Some(j) if j.fetching => return self.fail(id, "not_found"),
                Some(_) => return self.fail(id, "lost"),
                None => {}
            }
        }
        let Some(body) = body.filter(|_| error.is_empty()) else {
            return;
        };
        // The worker listens somewhere new once it has started again: the
        // link is made again to there.
        let addresses = listed_addresses(Some(body));
        let moved = self.job(id).is_some_and(|j| j.addresses != addresses);
        if !addresses.is_empty() && moved {
            self.update(id, |j| j.addresses = addresses);
            self.unlink(id);
        }
        self.seen(id, body);
    }

    /// Follow the room's list: the worker's queue items of jobs sent from
    /// here move them on as an answer to a status does.
    pub(super) fn follow_room(self: &Arc<Self>) {
        let seen: Vec<(Uuid, Value)> = {
            let Ok(room) = self.room.lock() else {
                return;
            };
            // What an offline computer last said is old news, and would undo
            // its jobs here being away.
            let items = room
                .computers
                .iter()
                .filter(|c| c.online)
                .flat_map(|c| c.jobs.iter().map(move |i| (c, i)));
            items
                .filter_map(|(c, item)| {
                    let job = item.get("job")?.as_str()?.parse::<Uuid>().ok()?;
                    let ours = self.job(job).is_some_and(|j| j.sent && j.other == c.id);
                    ours.then(|| (job, item.clone()))
                })
                .collect()
        };
        for (job, item) in seen {
            self.seen(job, &item);
        }
    }

    /// What the worker said of a job, in its own words or as its queue item.
    /// A held job's item says `paused` where its stage would be, and its
    /// `state` as it was.
    fn seen(self: &Arc<Self>, id: Uuid, seen: &Value) {
        let text = |name: &str| seen.get(name).and_then(Value::as_str).unwrap_or_default();
        let number = |name: &str| seen.get(name).and_then(Value::as_u64).unwrap_or(0);
        let held = text("stage") == "paused";
        let paused = held || seen.get("paused").and_then(Value::as_bool) == Some(true);
        let word = if held { "" } else { text("stage") };
        let Some(stage) = Stage::heard(word, text("state"), text("error")) else {
            return;
        };
        let Some(job) = self
            .job(id)
            .filter(|j| j.sent && !j.stage.ended() && j.accepted)
        else {
            return;
        };
        // Held or let go there, by someone at the worker or a phone. Not
        // while a hold of this computer's is on its way, which it is from
        // before.
        if job.paused != paused && !self.asking(&For::Pause(id)) {
            self.update(id, |j| j.paused = paused);
        }
        match stage {
            Stage::Sending | Stage::Checking | Stage::Queued | Stage::Paused | Stage::Exporting => {
                // A file being brought back is this computer's business now.
                if job.stage == Stage::Returning || job.fetching {
                    return;
                }
                self.progress(id, |j| {
                    if stage != Stage::Paused {
                        j.stage = stage;
                    }
                    if stage == Stage::Sending {
                        (j.bytes, j.size) = (number("bytes").max(j.bytes), number("size"));
                    }
                    if number("total") > 0 {
                        (j.frame, j.total) = (number("frame"), number("total"));
                    }
                });
            }
            Stage::Returning if job.lands == Lands::Sender => {
                if job.stage != Stage::Returning {
                    self.update(id, |j| {
                        j.stage = Stage::Returning;
                        j.frame = j.total;
                        (j.bytes, j.size) = (0, number("size"));
                    });
                }
            }
            // Ended by the worker's answer, which says where the file is.
            // Its item in the room doesn't, so that waits for the next.
            Stage::Done
                if job.lands == Lands::Worker && !job.fetching && seen.get("there").is_some() =>
            {
                let there = text("there").to_owned();
                self.update(id, |j| {
                    j.there = there;
                    j.frame = j.total;
                    j.end(Stage::Done, "");
                });
                self.stopped(id);
                if let Some(j) = self.job(id) {
                    self.news(BridgeExportNewsKind::Kept, &j);
                }
            }
            Stage::Failed | Stage::Cancelled => {
                let error = text("error");
                let error = if stage == Stage::Cancelled || error.is_empty() {
                    if stage == Stage::Cancelled {
                        "cancelled"
                    } else {
                        "failed"
                    }
                } else {
                    error
                };
                self.fail(id, error);
            }
            _ => {}
        }
    }

    /// A file of a sent job arrived: the list of what its export wrote, or
    /// one of those files. On the disk thread.
    pub(super) fn land(self: &Arc<Self>, id: Uuid, item: Uuid, path: &Path) {
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            let _ = std::fs::remove_file(path);
            return;
        };
        if item == id {
            return self.listed_out(&job, path);
        }
        let Some((_, to, _)) = job.landing.iter().find(|(i, ..)| *i == item).cloned() else {
            let _ = std::fs::remove_file(path);
            return;
        };
        // Never over a file that turned up meanwhile, unless it was said to.
        let to = if Path::new(&to).exists() && !job.overwrite {
            crate::export::taken::free_path(&to, "", BridgeCaptionsOut::Burn, &[])
        } else {
            to
        };
        let moved = std::fs::rename(path, &to).is_ok()
            || (std::fs::copy(path, &to).is_ok() && std::fs::remove_file(path).is_ok());
        if !moved {
            return self.give_up(id, "failed");
        }
        // A file of its own is named in the notice as it landed.
        let alone = job.landing.len() == 1 && job.landed.is_empty();
        let left = self.update(id, |j| {
            j.landing.retain(|(i, ..)| *i != item);
            if j.landed.is_empty() {
                j.path.clone_from(&to);
            }
            if alone {
                j.file = files::name_of(&to);
            }
            j.landed.push(to.clone());
            j.landing.len()
        });
        if left == Some(0) {
            self.landed_here(id);
        }
    }

    /// The list of what a finished export wrote: where each file goes here.
    fn listed_out(&self, job: &Job, list: &Path) {
        let text = lumit_ingress::read_to_string_capped(list, 16 << 20).ok();
        let list: Option<Value> = text.and_then(|t| serde_json::from_str(&t).ok());
        let Some(list) = list else {
            return self.give_up(job.job, "lost");
        };
        let sequence = list
            .get("sequence")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let base = if job.overwrite
            || !crate::export::taken::path_taken(
                &job.path,
                &job.codec,
                BridgeCaptionsOut::Burn,
                &[],
            ) {
            job.path.clone()
        } else {
            crate::export::taken::free_path(&job.path, &job.codec, BridgeCaptionsOut::Burn, &[])
        };
        let mut landing = Vec::new();
        for file in list
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let item = file
                .get("item")
                .and_then(Value::as_str)
                .and_then(|i| i.parse().ok());
            let bytes = file.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            let n = file.get("n").and_then(Value::as_u64).unwrap_or(0);
            let Some(item) = item else {
                continue;
            };
            let to = if sequence {
                let n = usize::try_from(n).unwrap_or(0);
                lumit_media::encode::sequence_frame_path(Path::new(&base), n)
                    .to_string_lossy()
                    .into_owned()
            } else {
                base.clone()
            };
            landing.push((item, to, bytes));
        }
        if landing.is_empty() {
            return self.give_up(job.job, "lost");
        }
        let size = landing.iter().map(|(.., b)| *b).sum();
        if let Ok(mut moving) = self.moving.lock() {
            moving.remove(&job.job);
        }
        self.update(job.job, |j| {
            j.landing = landing;
            (j.bytes, j.size) = (0, size);
            // Kept apart from a file already there: named as it lands.
            if base != j.path {
                j.file = files::name_of(&base);
                j.path = base;
            }
        });
        // Asked for at once rather than on the next turn.
        if let Ok(mut asked) = self.asked.lock() {
            asked.remove(&job.job);
        }
    }

    /// Every file of a sent job is here.
    fn landed_here(&self, id: Uuid) {
        let Some(job) = self.job(id) else {
            return;
        };
        self.update(id, |j| {
            j.bytes = j.size;
            j.fetching = false;
            j.end(Stage::Done, "");
        });
        self.stopped(id);
        self.tell(&job.other, "remote-landed", id, Value::Null);
        if job.noise {
            lumit_render::export::play_done_sound();
        }
        if job.reveal {
            crate::export::reveal_in_folder(&job.path);
        }
        if let Some(job) = self.job(id) {
            self.news(BridgeExportNewsKind::Landed, &job);
        }
    }

    /// Throw away what came back of a job that ended before it all had.
    pub(super) fn forget_landing(&self, job: &Job) {
        for (_, to, _) in &job.landing {
            let room = end::landing_room(to, job.job);
            let mut part = room.clone().into_os_string();
            part.push(".part");
            let _ = std::fs::remove_file(&room);
            let _ = std::fs::remove_file(PathBuf::from(part));
        }
    }

    /// Send a job once more: the same snapshot and settings, to `to` or the
    /// same computer.
    pub(super) fn again(
        self: &Arc<Self>,
        id: Uuid,
        to: Option<Computer>,
    ) -> Result<String, BridgeError> {
        let old = self
            .job(id)
            .filter(|j| j.sent)
            .ok_or(BridgeError::InvalidItem)?;
        let (other, other_name, near) = match to {
            Some(c) => (c.id, c.name, c.near),
            None => {
                let c = self.computer(&old.other);
                let near = c.as_ref().is_some_and(|c| c.near);
                (
                    old.other.clone(),
                    c.map_or(old.other_name.clone(), |c| c.name),
                    near,
                )
            }
        };
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|_| BridgeError::WriteFailed)?;
        let path = if old.landed.is_empty() {
            old.path.clone()
        } else {
            old.landed.first().cloned().unwrap_or_default()
        };
        // Over its own file, as an export queued again here goes.
        let overwrite =
            crate::export::taken::path_taken(&path, &old.codec, BridgeCaptionsOut::Burn, &[]);
        let job = Job {
            job: Uuid::now_v7(),
            account: self.account.lock().map(|a| a.clone()).unwrap_or_default(),
            other,
            other_name,
            near,
            path,
            overwrite,
            stage: Stage::Checking,
            created: job::now(),
            item: self
                .next_mirror
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            key: files::hex(&key),
            manifest: Piece::default(),
            document: Piece::default(),
            ..old.clone()
        };
        let job = Job {
            bytes: 0,
            size: 0,
            frame: 0,
            error: String::new(),
            ended: 0,
            fetched: 0,
            there: String::new(),
            accepted: false,
            addresses: Vec::new(),
            fetching: false,
            landing: Vec::new(),
            landed: Vec::new(),
            paused: false,
            ..job
        };
        let new = job.job;
        self.add(job);
        self.io
            .try_send(Io::Prepare(new, Snapshot::Job(id)))
            .map_err(|_| BridgeError::WriteFailed)?;
        Ok(new.to_string())
    }

    pub(super) fn move_to(
        self: &Arc<Self>,
        id: Uuid,
        computer: &str,
    ) -> Result<String, BridgeError> {
        let old = self
            .job(id)
            .filter(|j| j.sent)
            .ok_or(BridgeError::InvalidItem)?;
        if old.stage.started() {
            return Err(BridgeError::InvalidItem);
        }
        let to = self.choose(computer, old.comp, 0);
        // Not back to the one it is leaving, when the choice was left open.
        let to = match to {
            Ok(c) if c.id == old.other && computer.is_empty() => {
                let room = self.room.lock().map_err(|_| BridgeError::ReadFailed)?;
                let others = room.computers.iter().filter(|c| c.online && c.takes);
                let other = others
                    .filter(|c| c.id != old.other)
                    .max_by_key(|c| c.gpu_memory);
                other.cloned().ok_or(BridgeError::NoComputer)?
            }
            to => to?,
        };
        // A move, not a copy: the job it leaves is stopped and goes from the
        // list once the new one is in it. Its snapshot is copied first, as
        // the disk thread takes its work in order.
        let new = self.again(id, Some(to))?;
        self.remove(id);
        Ok(new)
    }

    pub(super) fn fetch(&self, id: Uuid, path: &str) -> Result<(), BridgeError> {
        let job = self
            .job(id)
            .filter(|j| j.sent)
            .ok_or(BridgeError::InvalidItem)?;
        if job.lands != Lands::Worker || job.stage != Stage::Done || path.trim().is_empty() {
            return Err(BridgeError::InvalidItem);
        }
        let path = crate::export::with_format_extension(path, &job.codec);
        // Brought here under the name it was given, which its notice says.
        let file = Path::new(&path)
            .file_name()
            .map_or_else(|| job.file.clone(), |n| n.to_string_lossy().into_owned());
        // The save dialogue asked about a file already there.
        let overwrite =
            crate::export::taken::path_taken(&path, &job.codec, BridgeCaptionsOut::Burn, &[]);
        self.update(id, |j| {
            j.file = file;
            j.path = path;
            j.overwrite = overwrite;
            j.fetching = true;
            j.landing.clear();
            j.landed.clear();
            j.stage = Stage::Returning;
            (j.bytes, j.size, j.ended) = (0, 0, 0);
            j.fetched = job::now();
        });
        let body = json!({ "lumit": "remote-fetch", "job": id });
        self.say(&job.other, body, For::Fetch(id));
        Ok(())
    }

    /// The worker answered a fetch: where it now listens for this job.
    pub(super) fn fetched(&self, id: Uuid, body: Option<&Value>, error: &str) {
        if !error.is_empty() {
            return self.fail(id, error);
        }
        let addresses = listed_addresses(body);
        // A new link, to the worker as it listens now.
        self.unlink(id);
        self.update(id, |j| j.addresses = addresses);
        if let Ok(mut asked) = self.asked.lock() {
            asked.remove(&id);
        }
    }
}

/// Where the worker said a job's link listens.
fn listed_addresses(body: Option<&Value>) -> Vec<String> {
    let listed = body
        .and_then(|b| b.get("addresses"))
        .and_then(Value::as_array);
    let listed = listed.into_iter().flatten().filter_map(Value::as_str);
    listed.take(8).map(str::to_owned).collect()
}

/// A document from a job's snapshot, as this Lumit reads one.
pub(super) fn read_document(path: &Path) -> Option<Document> {
    let text = lumit_ingress::read_to_string_capped(path, files::DOCUMENT_MOST).ok()?;
    serde_json::from_str(&text).ok()
}

/// The files a manifest lists, to hash again. `None` when it can't be read.
fn listed(manifest: &Path) -> Option<Vec<files::Found>> {
    let found = read_manifest(manifest)?
        .files
        .into_iter()
        .map(|e| files::Found {
            kind: e.kind,
            id: e.id,
            path: e.path,
            from: PathBuf::from(e.from),
        });
    Some(found.collect())
}

/// A job's manifest.
pub(super) fn read_manifest(path: &Path) -> Option<Manifest> {
    let text = lumit_ingress::read_to_string_capped(path, files::MANIFEST_MOST).ok()?;
    serde_json::from_str(&text).ok()
}
