//! A job's end of its link: the files this computer gives for it, where what
//! it takes is written, and what the link says of them.
//!
//! Called on the link's threads, with no lock of this module held. Anything
//! that takes a while is handed to the jobs' thread or the disk thread.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

use lumit_share::bulk::{Footage, News, Wanted};
use serde_json::Value;
use uuid::Uuid;
use web_time::{Duration, Instant};

use super::{files, job, Event, Io, Remote, Stage};

/// The most an export's list of files may be.
const LIST_MOST: u64 = 16 << 20;

pub(super) struct End {
    pub remote: Weak<Remote>,
    pub job: Uuid,
}

impl Footage for End {
    fn make(&self, wanted: &Wanted, _: &AtomicBool) -> Option<PathBuf> {
        let remote = self.remote.upgrade()?;
        let job = remote.job(self.job)?;
        match wanted {
            // Only ever a list already written: asked again after a dropped
            // link, the same list is sent, and no export starts for it.
            Wanted::Export { job: asked } if *asked == self.job && !job.sent => {
                let list = job::folder(&remote.root, self.job).join("out-list.json");
                list.is_file().then_some(list)
            }
            Wanted::Original { item } if job.sent => {
                let holding = remote.holding.lock().ok()?.get(&self.job).cloned()?;
                let (path, size) = holding.get(item)?;
                // A file changed since it was hashed is not what was offered.
                let same = std::fs::metadata(path).is_ok_and(|m| m.len() == *size);
                same.then(|| path.clone())
            }
            Wanted::Original { item } => out_files(&remote.root, self.job)
                .into_iter()
                .find(|(listed, ..)| listed == item)
                .map(|(_, path, _)| path),
            _ => None,
        }
    }

    fn room(&self, wanted: &Wanted) -> Option<PathBuf> {
        let remote = self.remote.upgrade()?;
        let job = remote.job(self.job)?;
        let folder = job::folder(&remote.root, self.job);
        match wanted {
            Wanted::Export { job: asked } if *asked == self.job && job.sent => {
                Some(folder.join("out-list.json"))
            }
            Wanted::Original { item } if job.sent => {
                let (_, path, _) = job.landing.iter().find(|(landing, ..)| landing == item)?;
                Some(landing_room(path, self.job))
            }
            Wanted::Original { item } => {
                if files::item(&job.manifest.hash) == *item {
                    return Some(folder.join("manifest.json"));
                }
                if files::item(&job.document.hash) == *item {
                    return Some(folder.join("document.json"));
                }
                // The job's own until it is checked, as another job may be
                // taking the same file at once. Then it goes to the store.
                let gathering = remote.gathering.lock().ok()?;
                let file = gathering.get(&self.job)?.file(*item)?;
                Some(folder.join("in").join(&file.hash))
            }
            _ => None,
        }
    }

    fn most(&self, wanted: &Wanted) -> u64 {
        let Some(remote) = self.remote.upgrade() else {
            return 0;
        };
        let Some(job) = remote.job(self.job) else {
            return 0;
        };
        match wanted {
            Wanted::Export { .. } => LIST_MOST,
            Wanted::Original { item } if job.sent => job
                .landing
                .iter()
                .find(|(landing, ..)| landing == item)
                .map_or(0, |(.., bytes)| *bytes),
            Wanted::Original { item } => {
                if files::item(&job.manifest.hash) == *item {
                    return job.manifest.size;
                }
                if files::item(&job.document.hash) == *item {
                    return job.document.size;
                }
                let gathering = remote.gathering.lock();
                let file = gathering
                    .ok()
                    .and_then(|g| g.get(&self.job)?.file(*item).cloned());
                file.map_or(0, |f| f.size)
            }
            _ => 0,
        }
    }

    fn told(&self, news: News) {
        let Some(remote) = self.remote.upgrade() else {
            return;
        };
        // Progress that finds no room is dropped: more is coming. The rest
        // waits its turn, which is never long.
        let event = Event::Told(self.job, news);
        match &event {
            Event::Told(_, News::Moving { .. }) => {
                let _ = remote.events.try_send(event);
            }
            _ => {
                let _ = remote.events.send(event);
            }
        }
    }
}

/// Where a file of a finished export is written as it comes back, beside
/// where it goes and named so nothing else is ever written over.
pub(super) fn landing_room(path: &str, job: Uuid) -> PathBuf {
    let short: String = job.simple().to_string().chars().take(8).collect();
    PathBuf::from(format!("{path}.lumit-{short}"))
}

/// The files of `job`'s finished export, as its list on the worker has
/// them: the item each is sent as, where it is, and its size.
pub(super) fn out_files(root: &std::path::Path, job: Uuid) -> Vec<(Uuid, PathBuf, u64)> {
    let list = job::folder(root, job).join("out-list.json");
    let read = lumit_ingress::read_to_string_capped(&list, LIST_MOST).ok();
    let list: Option<Value> = read.and_then(|text| serde_json::from_str(&text).ok());
    let files = list
        .as_ref()
        .and_then(|l| l.get("files"))
        .and_then(Value::as_array);
    files
        .into_iter()
        .flatten()
        .filter_map(|file| {
            let item = file.get("item")?.as_str()?.parse().ok()?;
            let path = PathBuf::from(file.get("path")?.as_str()?);
            Some((item, path, file.get("bytes")?.as_u64()?))
        })
        .collect()
}

impl Remote {
    /// Something a job's link said, on the jobs' thread.
    pub(super) fn told(&self, job: Uuid, news: News) {
        match news {
            News::Moving {
                wanted,
                done,
                total,
                ..
            } => self.moved(job, &wanted, done, total),
            News::Arrived { wanted, path } => {
                let item = match wanted {
                    Wanted::Original { item } => item,
                    Wanted::Export { job } => job,
                    _ => return,
                };
                let _ = self.io.send(Io::Arrived(job, item, path));
            }
            News::Refused { wanted } => {
                let Some(j) = self.job(job) else {
                    return;
                };
                if !j.sent && matches!(wanted, Wanted::Original { .. }) && !j.stage.ended() {
                    // The sender has not got what it said it had.
                    self.give_up(job, "lost");
                }
                // A sender asking for a file that is not ready yet asks
                // again on a later turn.
                if let Ok(mut asked) = self.asked.lock() {
                    asked.remove(&job);
                }
            }
            News::Holders => {}
        }
    }

    /// `done` of `total` bytes of `wanted` have crossed for `job`.
    fn moved(&self, job: Uuid, wanted: &Wanted, done: u64, total: u64) {
        let item = match wanted {
            Wanted::Original { item } => *item,
            Wanted::Export { job } => *job,
            _ => return,
        };
        let bytes = {
            let Ok(mut moving) = self.moving.lock() else {
                return;
            };
            let moving = moving.entry(job).or_default();
            moving.files.insert(item, (done, total));
            let bytes: u64 = moving.files.values().map(|(done, _)| *done).sum();
            let now = Instant::now();
            moving.readings.push((now, bytes));
            moving
                .readings
                .retain(|(at, _)| now.duration_since(*at) <= Duration::from_secs(5));
            bytes
        };
        self.update(job, |j| {
            if matches!(j.stage, Stage::Sending | Stage::Returning | Stage::Paused) {
                j.bytes = bytes.max(j.bytes.min(j.size));
                if j.size < j.bytes {
                    j.size = j.bytes;
                }
            }
        });
    }

    /// Bytes a second over the last few seconds of `job`'s transfers.
    pub(super) fn rate(&self, job: Uuid) -> u64 {
        let Ok(moving) = self.moving.lock() else {
            return 0;
        };
        let Some(readings) = moving.get(&job).map(|m| &m.readings) else {
            return 0;
        };
        match (readings.first(), readings.last()) {
            (Some((from, a)), Some((to, b))) if to > from => {
                let seconds = to.duration_since(*from).as_secs_f64();
                (b.saturating_sub(*a) as f64 / seconds) as u64
            }
            _ => 0,
        }
    }

    /// Start `job`'s link with this end, the worker's when `work`.
    pub(super) fn start_link(
        self: &Arc<Self>,
        job: &job::Job,
        work: bool,
    ) -> Result<Arc<lumit_share::remote::Link>, lumit_share::ShareError> {
        if let Some(link) = self.link(job.job) {
            return Ok(link);
        }
        let key = job.key().ok_or(lumit_share::ShareError::OutOfTurn)?;
        let end: Arc<dyn Footage> = Arc::new(End {
            remote: Arc::downgrade(self),
            job: job.job,
        });
        let limits = crate::footage::LIMITS.clone();
        let quiet = Box::new(|_| {});
        let link = if work {
            lumit_share::remote::work(
                job.job,
                key,
                job.near,
                Some(lumit_share::CLOUD_RELAY),
                end,
                limits,
                quiet,
            )?
        } else {
            let holds = self.holds(job.job);
            let addresses = job
                .addresses
                .iter()
                .filter_map(|a| a.parse().ok())
                .collect();
            lumit_share::remote::send(
                job.job,
                key,
                addresses,
                Some(lumit_share::CLOUD_RELAY),
                holds,
                end,
                limits,
                quiet,
            )?
        };
        let link = Arc::new(link);
        if let Ok(mut links) = self.links.lock() {
            links.insert(job.job, link.clone());
        }
        Ok(link)
    }
}

/// Every item of `holding`, as the sender says it holds them.
pub(super) fn holds(holding: &HashMap<Uuid, (PathBuf, u64)>) -> Vec<lumit_share::bulk::Held> {
    holding
        .iter()
        .map(|(item, (_, bytes))| lumit_share::bulk::Held {
            item: *item,
            bytes: *bytes,
        })
        .collect()
}
