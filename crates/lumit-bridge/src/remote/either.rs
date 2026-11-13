//! What either computer does to a remote export it has: list it, cancel
//! it, hold it, or forget it.

use lumit_share::bulk::Wanted;
use serde_json::{json, Value};
use uuid::Uuid;

use super::job::{Job, Stage};
use super::room::For;
use super::{Io, Remote};
use crate::api::remote::{BridgeExportNewsKind, BridgeRemoteJob};

impl Remote {
    pub(super) fn jobs(&self) -> Vec<BridgeRemoteJob> {
        let account = self.account.lock().map(|a| a.clone()).unwrap_or_default();
        let mut jobs: Vec<Job> = self.jobs.lock().map_or_else(
            |_| Vec::new(),
            |jobs| {
                jobs.values()
                    .filter(|j| j.account == account)
                    .cloned()
                    .collect()
            },
        );
        // Oldest first, as the export queue lists its own.
        jobs.sort_by(|a, b| a.created.cmp(&b.created).then(a.job.cmp(&b.job)));
        jobs.iter().map(|j| j.bridge(self.rate(j.job))).collect()
    }

    /// Stop `job` here, and with `tell` on the other computer too.
    pub(super) fn cancel_here(&self, id: Uuid, tell: bool) {
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        if self.unfetch(&job, "cancelled") {
            return;
        }
        self.update(id, |j| j.end(Stage::Cancelled, "cancelled"));
        self.stopped(id);
        self.forget_landing(&job);
        if tell {
            self.tell(&job.other, "remote-cancel", id, Value::Null);
        } else if let Some(job) = self.job(id) {
            self.news(BridgeExportNewsKind::RemoteFailed, &job);
        }
    }

    /// A Bring here that stopped short, cancelled or for `why`: the file
    /// stays on the other computer, to be brought again, and its link there
    /// goes. False when `job` was not one, or the file has gone from there.
    pub(super) fn unfetch(&self, job: &Job, why: &str) -> bool {
        if !job.sent || !job.fetching || why == "not_found" {
            return false;
        }
        let id = job.job;
        self.update(id, |j| {
            j.fetching = false;
            j.landing.clear();
            j.landed.clear();
            (j.bytes, j.size) = (0, 0);
            j.end(Stage::Done, "");
        });
        self.unlink(id);
        self.unsay(id);
        self.forget_landing(job);
        self.tell(&job.other, "remote-cancel", id, Value::Null);
        if why != "cancelled" {
            if let Some(mut j) = self.job(id) {
                j.error = why.to_owned();
                self.news(BridgeExportNewsKind::RemoteFailed, &j);
            }
        }
        true
    }

    pub(super) fn pause(&self, id: Uuid, paused: bool) {
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        self.pause_here(id, paused);
        let body = json!({ "lumit": "remote-pause", "job": id, "paused": paused });
        self.say(&job.other, body, For::Pause(id));
    }

    /// Hold `job` where it is, or let it go on: its export, and the
    /// transfers this computer takes. What came of a file is kept, and so is
    /// the step it had reached, which it goes on from.
    pub(super) fn pause_here(&self, id: Uuid, paused: bool) {
        let Some(job) = self.job(id).filter(|j| !j.stage.ended()) else {
            return;
        };
        self.update(id, |j| j.paused = paused);
        let mut taking: Vec<Wanted> = Vec::new();
        if job.sent {
            if job.stage == Stage::Returning {
                taking.push(Wanted::Export { job: id });
                let items = job
                    .landing
                    .iter()
                    .map(|(item, ..)| Wanted::Original { item: *item });
                taking.extend(items);
            }
        } else {
            crate::export::queue_pause(job.item, paused);
            if let Ok(gathering) = self.gathering.lock() {
                if let Some(g) = gathering.get(&id) {
                    taking.extend(g.wanted().map(|item| Wanted::Original { item }));
                }
            }
        }
        if let Some(link) = self.link(id) {
            for wanted in taking {
                if paused {
                    link.unwant(&wanted);
                } else {
                    link.want(wanted);
                }
            }
        }
    }

    pub(super) fn remove(&self, id: Uuid) {
        let Some(job) = self.job(id) else {
            return;
        };
        if !job.stage.ended() {
            self.cancel_here(id, true);
        }
        if !job.sent {
            crate::export::queue_remove(job.item);
        }
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(&id);
        }
        if let Ok(mut dirty) = self.dirty.lock() {
            dirty.remove(&id);
        }
        for map in [&self.holding] {
            if let Ok(mut map) = map.lock() {
                map.remove(&id);
            }
        }
        if let Ok(mut gathering) = self.gathering.lock() {
            gathering.remove(&id);
        }
        let _ = self.io.try_send(Io::Forget(id));
    }
}
