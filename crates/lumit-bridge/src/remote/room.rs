//! The account's room as this computer hears it: the other computers, and
//! the commands said to them and answered by them.
//!
//! A command that gets no answer is said again every [`AGAIN`] with the same
//! id, which the other computer answers as often as it is asked. One the room
//! answers `offline` waits until that computer is back and is said then.

use std::sync::Arc;

use serde_json::{json, Value};
use uuid::Uuid;
use web_time::{Duration, Instant};

use super::Remote;
use crate::api::remote::{BridgeComputer, BridgeRemoteSay};

/// How long a command waits for its answer before it is said again.
const AGAIN: Duration = Duration::from_secs(15);

/// How often a check is said before it is given up on.
const CHECK_TRIES: u32 = 4;

/// How long a computer may be away, in seconds, before it leaves the list
/// of those an export can go to.
const AWAY_MOST: i64 = 30 * 24 * 60 * 60;

/// One of the account's computers as the room last listed it.
#[derive(Debug, Clone, Default)]
pub(super) struct Computer {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub near: bool,
    pub takes: bool,
    pub seen: i64,
    pub platform: String,
    pub version: String,
    pub gpu: String,
    pub gpu_memory: u64,
    pub memory: u64,
    pub cores: u32,
    pub plugins: u32,
    pub fonts: u32,
    pub models: Vec<String>,
    /// Its public box key, which what is said to it is sealed for.
    pub key: String,
    pub queued: u32,
    pub exporting: bool,
    /// Its queue's items that belong to a job.
    pub jobs: Vec<Value>,
    /// Its mirrors of the jobs it sent: how each stands there.
    pub mirrors: Vec<Value>,
}

impl Computer {
    fn of(value: &Value) -> Computer {
        let text = |name: &str| {
            let found = value.get(name).and_then(Value::as_str).unwrap_or_default();
            found.chars().take(200).collect::<String>()
        };
        let number = |name: &str| value.get(name).and_then(Value::as_u64).unwrap_or(0);
        let yes = |name: &str| value.get(name).and_then(Value::as_bool).unwrap_or(false);
        let queue = value.get("queue").and_then(Value::as_array);
        let queue = queue.map(Vec::as_slice).unwrap_or_default();
        // A mirror is another computer's export, shown here only.
        let own = || queue.iter().filter(|item| item.get("on").is_none());
        let state = |item: &Value| {
            item.get("state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Computer {
            id: text("id"),
            name: text("name"),
            online: yes("online"),
            near: yes("near"),
            takes: yes("takes"),
            seen: value.get("seen").and_then(Value::as_i64).unwrap_or(0),
            platform: text("platform"),
            version: text("version"),
            gpu: text("gpu"),
            gpu_memory: number("gpuMemory"),
            memory: number("memory"),
            cores: u32::try_from(number("cores")).unwrap_or(u32::MAX),
            plugins: u32::try_from(number("plugins")).unwrap_or(u32::MAX),
            fonts: u32::try_from(number("fonts")).unwrap_or(u32::MAX),
            models: value
                .get("models")
                .and_then(Value::as_array)
                .map(|models| {
                    let names = models.iter().take(16);
                    names
                        .filter_map(|m| m.get("name").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            key: text("key"),
            queued: u32::try_from(
                own()
                    .filter(|item| matches!(state(item).as_str(), "waiting" | "running"))
                    .count(),
            )
            .unwrap_or(u32::MAX),
            exporting: own().any(|item| state(item) == "running"),
            jobs: own()
                .filter(|item| item.get("job").is_some())
                .take(200)
                .cloned()
                .collect(),
            mirrors: queue
                .iter()
                .filter(|item| item.get("on").and_then(Value::as_str).is_some())
                .take(200)
                .cloned()
                .collect(),
        }
    }

    pub(super) fn bridge(&self) -> BridgeComputer {
        BridgeComputer {
            id: self.id.clone(),
            name: self.name.clone(),
            online: self.online,
            near: self.near,
            takes: self.takes,
            seen: self.seen,
            platform: self.platform.clone(),
            version: self.version.clone(),
            gpu: self.gpu.clone(),
            gpu_memory: self.gpu_memory,
            memory: self.memory,
            cores: self.cores,
            plugins: self.plugins,
            fonts: self.fonts,
            models: self.models.clone(),
            queued: self.queued,
            exporting: self.exporting,
        }
    }
}

/// The room as last heard: this computer's id, and the others.
#[derive(Debug, Default)]
pub(super) struct Room {
    pub me: String,
    pub computers: Vec<Computer>,
}

impl Room {
    pub(super) fn find(&self, id: &str) -> Option<&Computer> {
        self.computers.iter().find(|c| c.id == id)
    }

    /// Whether `c` is worth listing at `now`: not away for a month, and not
    /// an old sign-in of a computer that has signed in again since, which
    /// goes by the same name on the same system.
    fn listed(&self, c: &Computer, now: i64) -> bool {
        if c.online {
            return true;
        }
        if c.seen > 0 && now - c.seen > AWAY_MOST {
            return false;
        }
        !self.computers.iter().any(|newer| {
            newer.id != c.id
                && newer.name == c.name
                && newer.platform == c.platform
                && (newer.online || newer.seen > c.seen)
        })
    }
}

/// What a command was for, so its answer can be put to use.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum For {
    /// A fit, kept under this composition and revision.
    Check {
        comp: Uuid,
        revision: u64,
    },
    Offer(Uuid),
    Status(Uuid),
    Fetch(Uuid),
    /// A hold, or letting go. While it is unanswered, what the other
    /// computer says of the job's hold is from before it heard.
    Pause(Uuid),
    /// Nothing comes of the answer: a cancel, a landing.
    Tell,
}

/// A command said and not answered yet.
#[derive(Debug, Clone)]
pub(super) struct Said {
    pub id: String,
    pub to: String,
    pub body: Value,
    pub what: For,
    /// When it was last said. `None` before it could be.
    pub at: Option<Instant>,
    pub tries: u32,
    /// Answered `offline`: said again once that computer is back.
    pub waits: bool,
}

impl Remote {
    /// Take in the room's list of the account's computers.
    pub(super) fn hear(&self, said: &Value) {
        let Some(listed) = said.get("computers").and_then(Value::as_array) else {
            return;
        };
        let mut me = String::new();
        let mut computers = Vec::new();
        for entry in listed.iter().take(64) {
            let computer = Computer::of(entry);
            if entry.get("me").and_then(Value::as_bool) == Some(true) {
                me = computer.id;
            } else if !computer.id.is_empty() {
                computers.push(computer);
            }
        }
        if let Ok(mut room) = self.room.lock() {
            room.me = me;
            room.computers = computers;
        }
    }

    /// The account's other computers, those that can take an export first,
    /// leaving out those [`Room::listed`] passes over.
    pub(super) fn computers(&self) -> Vec<BridgeComputer> {
        let Ok(room) = self.room.lock() else {
            return Vec::new();
        };
        let now = super::job::now();
        let mut all: Vec<&Computer> = room
            .computers
            .iter()
            .filter(|c| room.listed(c, now))
            .collect();
        all.sort_by(|a, b| {
            let ready = |c: &Computer| !(c.online && c.takes);
            (ready(a), !a.near, a.name.to_lowercase()).cmp(&(
                ready(b),
                !b.near,
                b.name.to_lowercase(),
            ))
        });
        all.into_iter().map(Computer::bridge).collect()
    }

    pub(super) fn computer(&self, id: &str) -> Option<Computer> {
        self.room.lock().ok()?.find(id).cloned()
    }

    /// Say `body` to the computer `to`, and keep it to say again until it is
    /// answered. Answers the command's id.
    pub(super) fn say(&self, to: &str, body: Value, what: For) -> String {
        let id = Uuid::now_v7().simple().to_string();
        let said = Said {
            id: id.clone(),
            to: to.to_owned(),
            body,
            what,
            at: None,
            tries: 0,
            waits: false,
        };
        if let Ok(mut all) = self.said.lock() {
            all.push(said);
        }
        self.resay();
        id
    }

    /// Whether a command for `what` is waiting for its answer.
    pub(super) fn asking(&self, what: &For) -> bool {
        self.said
            .lock()
            .is_ok_and(|all| all.iter().any(|s| s.what == *what))
    }

    /// Forget the commands for `job`, once it has ended.
    pub(super) fn unsay(&self, job: Uuid) {
        if let Ok(mut all) = self.said.lock() {
            all.retain(|s| {
                !matches!(s.what, For::Offer(j) | For::Status(j) | For::Fetch(j) | For::Pause(j) if j == job)
            });
        }
    }

    /// Say each command that is due: not said yet, or not answered in
    /// [`AGAIN`], and whose computer is there to hear it.
    pub(super) fn resay(&self) {
        let mut out = Vec::new();
        let mut gave_up = Vec::new();
        {
            let (Ok(mut all), Ok(room)) = (self.said.lock(), self.room.lock()) else {
                return;
            };
            all.retain(|said| {
                let check = matches!(said.what, For::Check { .. });
                let gone = check && said.tries >= CHECK_TRIES;
                if gone {
                    gave_up.push(said.clone());
                }
                !gone
            });
            for said in all.iter_mut() {
                let Some(to) = room.find(&said.to) else {
                    continue;
                };
                if to.key.is_empty() || (said.waits && !to.online) {
                    continue;
                }
                if said.at.is_some_and(|at| at.elapsed() < AGAIN) && !said.waits {
                    continue;
                }
                said.at = Some(Instant::now());
                said.tries += 1;
                said.waits = false;
                out.push(BridgeRemoteSay {
                    id: said.id.clone(),
                    to: said.to.clone(),
                    key: to.key.clone(),
                    reply: false,
                    body: said.body.to_string(),
                    error: String::new(),
                });
            }
        }
        for said in gave_up {
            if let For::Check { comp, revision } = said.what {
                self.checked(&said.to, comp, revision, None, "no_answer");
            }
        }
        if out.is_empty() {
            return;
        }
        let Ok(sink) = self.outbox.lock() else {
            return;
        };
        match sink.as_ref() {
            Some(sink) => out.into_iter().for_each(sink),
            // Nobody to say it through yet: said as soon as there is.
            None => {
                drop(sink);
                if let Ok(mut all) = self.said.lock() {
                    for said in all.iter_mut().filter(|s| out.iter().any(|o| o.id == s.id)) {
                        said.at = None;
                        said.tries = said.tries.saturating_sub(1);
                    }
                }
            }
        }
    }

    /// Answer the command `id` from `to`, with a body or an error code.
    pub(super) fn answer(&self, id: &str, to: &str, body: Option<Value>, error: &str) {
        let key = self.computer(to).map(|c| c.key).unwrap_or_default();
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let say = BridgeRemoteSay {
            id: id.to_owned(),
            to: to.to_owned(),
            key: if body.is_empty() { String::new() } else { key },
            reply: true,
            body,
            error: error.to_owned(),
        };
        if let Ok(sink) = self.outbox.lock() {
            if let Some(sink) = sink.as_ref() {
                sink(say);
            }
        }
    }

    /// The answer to the command `id`, as the room or the other computer
    /// gave it.
    pub(super) fn settle(self: &Arc<Self>, id: &str, body: &str, error: &str) {
        let said = {
            let Ok(mut all) = self.said.lock() else {
                return;
            };
            let Some(at) = all.iter().position(|s| s.id == id) else {
                return;
            };
            // The other computer is away: said again once it is back.
            if matches!(error, "offline" | "no_answer") && said_again(&all[at].what) {
                all[at].waits = true;
                let what = all[at].what.clone();
                drop(all);
                self.away(&what);
                return;
            }
            all.remove(at)
        };
        let body: Option<Value> = serde_json::from_str(body).ok();
        self.answered(said, body, error);
    }

    /// Put an answer to use.
    fn answered(self: &Arc<Self>, said: Said, body: Option<Value>, error: &str) {
        match said.what {
            For::Check { comp, revision } => {
                self.checked(&said.to, comp, revision, body.as_ref(), error);
            }
            For::Offer(job) => self.offered(job, body.as_ref(), error),
            For::Status(job) => self.status(job, body.as_ref(), error),
            For::Fetch(job) => self.fetched(job, body.as_ref(), error),
            For::Pause(_) | For::Tell => {}
        }
    }

    /// Say `lumit` about `job` to the other computer, with `more` beside it.
    pub(super) fn tell(&self, to: &str, lumit: &str, job: Uuid, more: Value) {
        let mut body = json!({ "lumit": lumit, "job": job });
        if let (Some(body), Value::Object(more)) = (body.as_object_mut(), more) {
            body.extend(more);
        }
        self.say(to, body, For::Tell);
    }
}

/// Whether a command answered `offline` is kept to say again later. A
/// status is asked again anyway, and a check is moot by then.
fn said_again(what: &For) -> bool {
    matches!(
        what,
        For::Offer(_) | For::Fetch(_) | For::Pause(_) | For::Tell
    )
}
