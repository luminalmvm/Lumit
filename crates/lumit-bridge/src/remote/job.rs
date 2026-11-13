//! A remote export as one computer keeps it: what it is, how far it has got,
//! and what it needs to carry on after a restart. Kept as `record.json` in
//! the job's own folder on both computers.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::remote::{BridgeRemoteJob, BridgeRemoteLands, BridgeRemoteStage};

/// How far a job has got, as both computers say it. The words are the ones
/// the two say to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Stage {
    Away,
    #[default]
    Checking,
    Sending,
    Queued,
    Paused,
    Exporting,
    Returning,
    Done,
    Failed,
    Cancelled,
}

impl Stage {
    pub(super) fn ended(self) -> bool {
        matches!(self, Stage::Done | Stage::Failed | Stage::Cancelled)
    }

    /// Whether its export has started, after which it can't move elsewhere.
    pub(super) fn started(self) -> bool {
        matches!(self, Stage::Exporting | Stage::Returning) || self.ended()
    }

    pub(super) fn word(self) -> &'static str {
        match self {
            Stage::Away => "away",
            Stage::Checking => "checking",
            Stage::Sending => "sending",
            Stage::Queued => "queued",
            Stage::Paused => "paused",
            Stage::Exporting => "exporting",
            Stage::Returning => "returning",
            Stage::Done => "done",
            Stage::Failed => "failed",
            Stage::Cancelled => "cancelled",
        }
    }

    /// A stage as the other computer said it: in a job's own words, or as a
    /// queue item in the room has it (`stage` beside `state`).
    pub(super) fn heard(stage: &str, state: &str, error: &str) -> Option<Stage> {
        Some(match (stage, state) {
            ("away", _) => Stage::Away,
            ("checking", _) => Stage::Checking,
            ("sending", _) => Stage::Sending,
            ("queued", _) | ("", "waiting") => Stage::Queued,
            ("paused", _) => Stage::Paused,
            ("exporting", _) | ("", "running") => Stage::Exporting,
            ("returning", _) => Stage::Returning,
            ("done" | "landed" | "kept", _) | ("", "done") => Stage::Done,
            ("cancelled", _) => Stage::Cancelled,
            ("failed", _) | ("", "failed") if error == "cancelled" => Stage::Cancelled,
            ("failed", _) | ("", "failed") => Stage::Failed,
            _ => return None,
        })
    }
}

impl From<Stage> for BridgeRemoteStage {
    fn from(stage: Stage) -> Self {
        match stage {
            Stage::Away => BridgeRemoteStage::Away,
            Stage::Checking => BridgeRemoteStage::Checking,
            Stage::Sending => BridgeRemoteStage::Sending,
            Stage::Queued => BridgeRemoteStage::Queued,
            Stage::Paused => BridgeRemoteStage::Paused,
            Stage::Exporting => BridgeRemoteStage::Exporting,
            Stage::Returning => BridgeRemoteStage::Returning,
            Stage::Done => BridgeRemoteStage::Done,
            Stage::Failed => BridgeRemoteStage::Failed,
            Stage::Cancelled => BridgeRemoteStage::Cancelled,
        }
    }
}

/// Where the file ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Lands {
    #[default]
    Sender,
    Worker,
}

impl From<BridgeRemoteLands> for Lands {
    fn from(lands: BridgeRemoteLands) -> Self {
        match lands {
            BridgeRemoteLands::Sender => Lands::Sender,
            BridgeRemoteLands::Worker => Lands::Worker,
        }
    }
}

/// A file named by its hash, and its size.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Piece {
    pub hash: String,
    pub size: u64,
}

/// One remote export, on either computer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Job {
    pub job: Uuid,
    /// The account it belongs to. Jobs of another wait.
    pub account: String,
    pub sent: bool,
    pub other: String,
    pub other_name: String,
    pub comp: Uuid,
    pub comp_name: String,
    pub preset: String,
    pub codec: String,
    pub file: String,
    /// Where the file is, or goes, on this computer.
    pub path: String,
    /// The file comes back over one already at `path`. Set when one was
    /// there as it was sent or brought, which is asked about first.
    pub overwrite: bool,
    /// Where it is on the other computer, once it has said.
    pub there: String,
    pub lands: Lands,
    pub stage: Stage,
    pub paused: bool,
    pub bytes: u64,
    pub size: u64,
    pub frame: u64,
    pub total: u64,
    pub error: String,
    pub created: i64,
    pub ended: i64,
    /// When the sender last asked for a file the worker kept. Its week is
    /// counted from then.
    pub fetched: i64,
    /// The worker's queue item, or the sender's mirror in the room.
    pub item: u32,
    /// The link's key, in hex.
    pub key: String,
    /// The export's settings as the engine has them.
    pub spec: Value,
    /// The sender's own *when done* ticks, honoured when the file lands.
    pub noise: bool,
    pub reveal: bool,
    /// The project's file set and the document, once worked out.
    pub manifest: Piece,
    pub document: Piece,
    /// What the project needs ([`super::needs::Needs`]), as JSON.
    pub needs: Value,
    /// All the bytes the sender holds for it.
    pub all: u64,
    /// The project's folder on the sender, which a snapshot's relative paths
    /// are read against.
    pub project_dir: String,
    /// Where its project's simulations are baked, on the sender.
    pub bakes: String,
    /// The schema and the version of the Lumit that sent it.
    pub schema: String,
    pub version: String,
    /// The two computers reach the server from one address.
    pub near: bool,
    /// The worker said yes, and where it listens.
    pub accepted: bool,
    pub addresses: Vec<String>,
    /// The sender is bringing back a file the worker kept.
    pub fetching: bool,
    /// The files of the finished export as they land here: the item each is
    /// sent as, where it goes, and its size.
    pub landing: Vec<(Uuid, String, u64)>,
    /// Where those that landed went.
    pub landed: Vec<String>,
}

impl Job {
    pub(super) fn bridge(&self, rate: u64) -> BridgeRemoteJob {
        BridgeRemoteJob {
            job: self.job.to_string(),
            sent: self.sent,
            other: self.other.clone(),
            other_name: self.other_name.clone(),
            comp_name: self.comp_name.clone(),
            preset: self.preset.clone(),
            codec: self.codec.clone(),
            file: self.file.clone(),
            // A file that stayed there has no place here until it is brought.
            path: if self.sent
                && self.lands == Lands::Worker
                && !self.fetching
                && self.landed.is_empty()
            {
                String::new()
            } else {
                self.path.clone()
            },
            there: self.there.clone(),
            lands: match self.lands {
                Lands::Sender => BridgeRemoteLands::Sender,
                Lands::Worker => BridgeRemoteLands::Worker,
            },
            stage: self.shown().into(),
            bytes: self.bytes,
            size: self.size,
            rate,
            frame: self.frame,
            total: self.total,
            error: self.error.clone(),
            created: self.created,
            ended: self.ended,
            item: self.item,
        }
    }

    /// Its stage as it is shown: held, or the step it has reached.
    pub(super) fn shown(&self) -> Stage {
        if self.paused && !self.stage.ended() {
            Stage::Paused
        } else {
            self.stage
        }
    }

    /// How it stands, as the worker tells the sender. A held job says the
    /// step it has reached beside it, so it goes on from there.
    pub(super) fn view(&self) -> Value {
        json!({
            "job": self.job,
            "stage": self.stage.word(),
            "paused": self.paused,
            "frame": self.frame,
            "total": self.total,
            "bytes": self.bytes,
            "size": self.size,
            "error": self.error,
            "there": if self.lands == Lands::Worker { self.path.as_str() } else { "" },
        })
    }

    /// End it here, as `stage`, with `error`.
    pub(super) fn end(&mut self, stage: Stage, error: &str) {
        self.stage = stage;
        self.error = error.to_owned();
        self.paused = false;
        if self.ended == 0 {
            self.ended = now();
        }
    }

    pub(super) fn key(&self) -> Option<[u8; 32]> {
        let bytes = super::files::unhex(&self.key)?;
        bytes.try_into().ok()
    }
}

/// Unix seconds now.
pub(super) fn now() -> i64 {
    // web_time's, as std's clock panics on the web.
    let since = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH);
    since.map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The folder a job keeps its things in, on either computer.
pub(super) fn folder(root: &Path, job: Uuid) -> PathBuf {
    root.join("jobs").join(job.to_string())
}

/// Write `job`'s record, to one side first so a record is always whole.
pub(super) fn save(root: &Path, job: &Job) {
    let dir = folder(root, job.job);
    let Ok(text) = serde_json::to_vec_pretty(job) else {
        return;
    };
    let (making, made) = (dir.join("record.json.new"), dir.join("record.json"));
    let saved = std::fs::create_dir_all(&dir).is_ok()
        && std::fs::write(&making, text).is_ok()
        && std::fs::rename(&making, &made).is_ok();
    if !saved {
        note!("remote: the record of {} could not be written", job.job);
    }
}

/// Every job kept under `root`.
pub(super) fn load(root: &Path) -> Vec<Job> {
    let Ok(folders) = std::fs::read_dir(root.join("jobs")) else {
        return Vec::new();
    };
    folders
        .flatten()
        .filter_map(|folder| {
            let text = std::fs::read(folder.path().join("record.json")).ok()?;
            serde_json::from_slice::<Job>(&text).ok()
        })
        .collect()
}
