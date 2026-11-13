//! Exports one of the account's computers does for another (docs/01-GLOSSARY.md:
//! Remote export), and the news any export has when it ends.
//!
//! # In plain terms
//!
//! The interface holds the line to the account's room on Lumit's server and the
//! key that seals what crosses it, and passes on what it hears. Everything else
//! is decided here: what this computer says about itself, which of the others
//! can take an export, the jobs on both computers, their files, and their place
//! in the export queue. A job's files go straight between the two computers, or
//! through Lumit's relay when they can't reach each other (`lumit_share::remote`).
//! Every job is kept on disk on both computers, so either can sleep, lose the
//! network or quit, and the job carries on when it is back.

use flutter_rust_bridge::frb;

use crate::api::{composition::CompositionReference, export::BridgeExportSpec, BridgeError};
use crate::frb_generated::StreamSink;

/// One of the account's other computers, as the room last listed it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeComputer {
    /// Its sign-in's id, which commands name it by.
    pub id: String,
    pub name: String,
    pub online: bool,
    /// It reaches Lumit's server from the same address as this computer, so
    /// the two are most likely on one network.
    pub near: bool,
    /// It takes exports from the account's other computers.
    pub takes: bool,
    /// When it was last online, in unix seconds. Zero when never.
    pub seen: i64,
    /// `windows`, `macos` or `linux`.
    pub platform: String,
    /// Its Lumit's version, such as `0.11.1`. Empty when it never said.
    pub version: String,
    /// The graphics card its exports run on. Empty while unknown.
    pub gpu: String,
    pub gpu_memory: u64,
    pub memory: u64,
    pub cores: u32,
    pub plugins: u32,
    pub fonts: u32,
    /// The model packs it has, by their own names.
    pub models: Vec<String>,
    /// How many exports wait in its queue or run there now.
    pub queued: u32,
    /// One is running there now.
    pub exporting: bool,
}

/// Whether a computer can do a composition's export, as far as is known.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeRemoteFit {
    /// Asked, and not answered yet.
    Checking,
    /// It has everything the export needs.
    Ready,
    /// It lacks these. The export still runs there without them, as it
    /// would here with them missing.
    Lacks {
        plugins: Vec<String>,
        fonts: Vec<String>,
        models: Vec<String>,
    },
    /// Its Lumit can't read this project. `needs` is the version that can.
    TooOld { needs: String },
    /// It is offline or doesn't take exports, so it could not be asked.
    Unknown,
}

/// Where a remote export's file ends up.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRemoteLands {
    /// On the computer that sent it.
    Sender,
    /// On the computer that exported it.
    Worker,
}

/// How far a remote export has got.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRemoteStage {
    /// The other computer is offline or asleep. The job goes on when it is back.
    Away,
    /// Working out what the worker lacks.
    Checking,
    /// The project and the files the worker lacks are moving.
    Sending,
    /// The worker has everything, and the export waits in its queue.
    Queued,
    /// Held by someone, at whatever step it had reached.
    Paused,
    Exporting,
    /// The file is on its way back to the sender.
    Returning,
    /// The file is on the computer it was to end up on.
    Done,
    Failed,
    Cancelled,
}

/// A remote export as this computer sees it: one it sent, or one it does.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRemoteJob {
    pub job: String,
    /// True on the computer that sent it.
    pub sent: bool,
    /// The other computer's id, and its name as the room last gave it.
    pub other: String,
    pub other_name: String,
    pub comp_name: String,
    /// The delivery preset, empty for a custom export.
    pub preset: String,
    /// The format key, as the export queue's.
    pub codec: String,
    /// The export's file name.
    pub file: String,
    /// Where the file is, or will be, on this computer. Empty when it ends up
    /// on the other one.
    pub path: String,
    /// Where it is on the other computer, when it ends up there and that
    /// computer has said. Empty otherwise.
    pub there: String,
    pub lands: BridgeRemoteLands,
    pub stage: BridgeRemoteStage,
    /// The transfer the stage names, in bytes: so far, in all, and a second
    /// over the last few seconds. Zero when nothing is moving.
    pub bytes: u64,
    pub size: u64,
    pub rate: u64,
    /// The export's frames: so far and in all. Zero until it runs.
    pub frame: u64,
    pub total: u64,
    /// Why it failed or stopped, as a code (`too_old`, `no_space`, `lost`,
    /// `not_found`, `turned_off`, `cancelled`) or the export's own words.
    /// Empty otherwise.
    pub error: String,
    /// When it was sent and when it ended, in unix seconds. Zero for not yet.
    pub created: i64,
    pub ended: i64,
    /// On the worker, its item in the export queue (0 before it has one). On
    /// the sender, the id its mirror goes by in the room.
    pub item: u32,
}

/// Something this computer has to say in the account's room.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRemoteSay {
    /// The command's id: a new one for a command, or the one being answered.
    pub id: String,
    /// The computer it is for.
    pub to: String,
    /// The public key to seal `body` for. Empty when there is no body.
    pub key: String,
    /// True for an answer to a command another computer sent here.
    pub reply: bool,
    /// What to seal and send, as JSON. Empty for none.
    pub body: String,
    /// An answer's error code. Empty for none.
    pub error: String,
}

/// What a piece of export news is.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeExportNewsKind {
    /// An export this computer did for itself wrote its file.
    Exported,
    /// One stopped short, or was cancelled.
    Failed,
    /// Another computer sent this one an export to do.
    Arrived,
    /// The file of an export this computer sent came back.
    Landed,
    /// An export this computer sent is done, and its file stays there.
    Kept,
    /// An export between two computers failed, or the other one cancelled it.
    RemoteFailed,
}

/// An export ending, or a remote one arriving, as it happens.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeExportNews {
    pub kind: BridgeExportNewsKind,
    pub comp_name: String,
    pub file: String,
    /// Where the file is on this computer. Empty when it isn't here.
    pub path: String,
    /// The other computer's name, for news about a remote export.
    pub other_name: String,
    /// As [`BridgeRemoteJob::error`].
    pub error: String,
    /// The remote export's job, empty for one done here for itself.
    pub job: String,
}

/// What the interface heard in the room about the account's computers: the
/// whole `computers` message, as it came.
#[frb(sync)]
pub fn remote_heard(said: String) {
    crate::remote::heard(&said);
}

/// A command another computer sent this one: its id, who sent it, and its body
/// as the interface opened it. It is answered through [`remote_outbox`].
#[frb(sync)]
pub fn remote_command(id: String, from: String, body: String) {
    crate::remote::command(&id, &from, &body);
}

/// The answer to a command this computer sent: its body as the interface opened
/// it, or the error code the room or the other computer gave. Either may be
/// empty.
#[frb(sync)]
pub fn remote_reply(id: String, body: String, error: String) {
    crate::remote::reply(&id, &body, &error);
}

/// What this computer has to say in the room, as it comes up. A command that
/// gets no answer is said again, so one lost on a dropped line costs nothing.
pub fn remote_outbox(sink: StreamSink<BridgeRemoteSay>) -> Result<(), BridgeError> {
    crate::remote::outbox(sink);
    Ok(())
}

/// Exports ending here, and remote ones arriving or ending, as they happen.
pub fn export_news(sink: StreamSink<BridgeExportNews>) -> Result<(), BridgeError> {
    crate::remote::news(sink);
    Ok(())
}

/// The fields this computer adds to what it says about itself in the room, as
/// a JSON object: its card, memory, cores, plugins, fonts and model packs.
#[frb(sync)]
#[must_use]
pub fn remote_state_fields() -> String {
    crate::remote::state_fields()
}

/// The account is now `account` (its id), or none. Jobs are an account's, and
/// wait while another is signed in or none is.
#[frb(sync)]
pub fn remote_account(account: Option<String>) {
    crate::remote::account(account);
}

/// Whether this computer takes exports from the account's other computers.
#[frb(sync)]
pub fn remote_set_takes(on: bool) {
    crate::remote::set_takes(on);
}

/// The folders this computer looks in for footage it already has, so a job
/// doesn't send it again.
#[frb(sync)]
pub fn remote_set_folders(folders: Vec<String>) {
    crate::remote::set_folders(folders);
}

/// The account's other computers, those that can take an export first.
#[frb(sync)]
#[must_use]
pub fn remote_computers() -> Vec<BridgeComputer> {
    crate::remote::computers()
}

impl CompositionReference {
    /// Whether `computer` can do this composition's export as the project now
    /// stands, asking it when it hasn't been asked since the project changed.
    #[frb(sync)]
    #[must_use]
    pub fn remote_fit(&self, computer: String) -> BridgeRemoteFit {
        crate::remote::fit(self, &computer)
    }

    /// Send this composition's export by `spec` to `computer`, or with an empty
    /// one to whichever of the account's computers can do it soonest. With
    /// `lands` `Sender` the file comes back to `path`. With `Worker` it stays
    /// there, named as `path` names it. Answers the job.
    #[frb(sync)]
    pub fn remote_send(
        &self,
        computer: String,
        spec: BridgeExportSpec,
        path: String,
        lands: BridgeRemoteLands,
    ) -> Result<String, BridgeError> {
        crate::remote::send(self, &computer, &spec, &path, lands)
    }
}

/// Every remote export this computer knows of, oldest first.
#[frb(sync)]
#[must_use]
pub fn remote_jobs() -> Vec<BridgeRemoteJob> {
    crate::remote::jobs()
}

/// Stop a remote export on both computers.
#[frb(sync)]
pub fn remote_cancel(job: String) {
    crate::remote::cancel(&job);
}

/// Hold a remote export where it is, or let it go on.
#[frb(sync)]
pub fn remote_pause(job: String, paused: bool) {
    crate::remote::pause(&job, paused);
}

/// Forget a remote export here, stopping it first if it hasn't ended.
#[frb(sync)]
pub fn remote_remove(job: String) {
    crate::remote::remove(&job);
}

/// Send an export this computer sent once again, to the same computer and to
/// the same place. Answers the new job.
#[frb(sync)]
pub fn remote_again(job: String) -> Result<String, BridgeError> {
    crate::remote::again(&job)
}

/// Send an export that hasn't started on its computer to `computer` instead,
/// or with an empty one to whichever can do it soonest. Answers the new job.
#[frb(sync)]
pub fn remote_move(job: String, computer: String) -> Result<String, BridgeError> {
    crate::remote::move_to(&job, &computer)
}

/// Bring the file of an export that stayed on its computer here, to `path`.
#[frb(sync)]
pub fn remote_fetch(job: String, path: String) -> Result<(), BridgeError> {
    crate::remote::fetch(&job, &path)
}

/// Where the record of an export this computer did for another is kept, for
/// one that failed. Empty when there is none.
#[frb(sync)]
#[must_use]
pub fn remote_log(job: String) -> String {
    crate::remote::log(&job)
}

/// How much the files kept for the account's other computers take, in bytes.
#[frb(sync)]
#[must_use]
pub fn remote_kept_bytes() -> u64 {
    crate::remote::kept_bytes()
}

/// Delete the files kept for the account's other computers, except those a job
/// needs now.
#[frb(sync)]
pub fn remote_kept_clear() {
    crate::remote::kept_clear();
}
