//! Shared projects: one person hosts a project and others join it, all
//! editing at once. `lumit-share` does the sharing, and this is the surface
//! the frontend drives it through.

use std::{
    collections::BTreeMap,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
};

use flutter_rust_bridge::frb;
use lumit_core::{Document, DocumentStore};
use lumit_share::{
    Conflict, Ending, Event, Footage, Invite, News, Person, Presence, Reach, Refusal, Relayed,
    ShareError, Sharing, Versions, Wanted,
};
use uuid::Uuid;

use crate::{
    api::{
        composition::CompositionReference,
        footage::FootageReference,
        layer::LayerReference,
        project::{PackProgressStream, ProjectReference},
        state::{adopt, CallbackStream},
        BridgeError,
    },
    frb_generated::StreamSink,
};

/// Each shared project that is open, by project. An entry goes when sharing
/// stops or the project closes.
///
/// Taken before a project's own lock, never after. The share threads never
/// take it, so holding it across a call into `lumit-share` can't deadlock
/// against them.
static SHARED: LazyLock<Mutex<BTreeMap<Uuid, Sharing>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Where a host listens. Every interface, so other machines can reach it. A
/// test stays on this machine, which also keeps the firewall from asking.
const LISTEN: IpAddr = IpAddr::V4(if cfg!(test) {
    Ipv4Addr::LOCALHOST
} else {
    Ipv4Addr::UNSPECIFIED
});

/// One person in a shared project, and what they are looking at.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeSharePerson {
    pub id: u32,
    pub name: String,
    /// An index into the label palette, from 1, given out in joining order.
    pub colour: u8,
    /// This is the person at this machine.
    pub me: bool,
    /// This is the host. In a project kept online that is anyone signed in
    /// to the account that keeps it, so it can be more than one person, or
    /// nobody while the host is not here.
    pub host: bool,
    /// This person can only look. They joined a project kept online by the
    /// invite that changes nothing: they see it and are seen, and nothing
    /// they do reaches anyone.
    pub look: bool,
    /// The composition they have open, if it is still in the project.
    pub comp: Option<CompositionReference>,
    /// The layers they have selected in it.
    pub layers: Vec<LayerReference>,
    /// The frame their playhead is on in `comp`.
    pub playhead: Option<i64>,
    /// Their pointer over the Viewer, in composition pixels.
    pub cursor_x: Option<f64>,
    pub cursor_y: Option<f64>,
    /// The property rows they have selected in the Timeline and the
    /// keyframes, as the frontend named them in [`ProjectReference::share_presence`].
    pub properties: Vec<String>,
    pub keys: Vec<String>,
    /// The controls they are changing right now, as their frontend named
    /// them in [`ProjectReference::share_presence`], to be held for them.
    /// Empty when they are changing nothing. In a project kept online it
    /// empties by itself two seconds after their machine last said so.
    pub changing: Vec<String>,
}

/// Why sharing stopped for a guest.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareEnding {
    /// The host stopped sharing.
    Closed,
    /// The host runs another version of Lumit, which `host` names.
    VersionMismatch { host: String },
    /// The project already has as many people as it takes.
    Full,
    /// The host's project is one this machine will not open.
    Unsafe,
    /// The host took this person out of the project.
    Removed,
}

/// Whether people outside the host's network can reach it.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareReach {
    /// The router was not asked. People outside need a VPN, or the port
    /// forwarded by hand.
    Off,
    /// The router is being asked to open the port.
    Asking,
    /// The router sends the port to this machine. `address` is the one it
    /// has on the internet, for the invite of someone outside.
    Open { address: String },
    /// No router answered, or it would not open the port.
    Refused,
    /// The router is behind another, or behind an address its provider
    /// shares, so its port opens onto nobody.
    Behind,
}

#[frb(ignore)]
fn reach(reach: Reach) -> BridgeShareReach {
    match reach {
        Reach::Off => BridgeShareReach::Off,
        Reach::Asking => BridgeShareReach::Asking,
        Reach::Open { address } => BridgeShareReach::Open { address },
        Reach::Refused => BridgeShareReach::Refused,
        Reach::Behind => BridgeShareReach::Behind,
    }
}

/// Whether a host has a room at a relay, for people no address of this
/// machine lets in.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeShareRelayed {
    /// It was given no relay.
    Off,
    /// The relay is being asked.
    Asking,
    /// The room is open, and the link leads to it.
    Open,
    /// The relay did not answer or would not open a room. It is asked again
    /// every few seconds.
    Unreachable,
}

#[frb(ignore)]
fn relayed(relayed: Relayed) -> BridgeShareRelayed {
    match relayed {
        Relayed::Off => BridgeShareRelayed::Off,
        Relayed::Asking => BridgeShareRelayed::Asking,
        Relayed::Open => BridgeShareRelayed::Open,
        Relayed::Unreachable => BridgeShareRelayed::Unreachable,
    }
}

/// What a shared project tells the frontend as it goes.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareEvent {
    /// Who is here and what each is looking at. The whole list each time.
    People { people: Vec<BridgeSharePerson> },
    /// This guest has lost its host and is working on alone until it is back.
    Away,
    /// The host is back. The document was replaced by the host's with this
    /// guest's own edits merged in, so everything on screen is stale. `held`
    /// conflicts wait in `share_conflicts`, and `refused` edits no longer
    /// applied and are gone. `dropped` says what those were to, the first
    /// twenty of them. [`ProjectReference::share_return_copies`] has this
    /// machine's document from before, when anything was held or dropped.
    Back {
        held: u32,
        refused: u32,
        dropped: Vec<BridgeShareDropped>,
    },
    /// The invite this guest is looking for its host by leads to a different
    /// project. It is still away and still looking.
    Elsewhere,
    /// For a copy of a project kept online: the invite this machine finds
    /// the project by was replaced while it was away, and opens nothing
    /// now. It is still [`Self::Away`], with what was made here kept, and
    /// asks Lumit's server no more until
    /// [`ProjectReference::share_reinvite`] gives it the new invite, which
    /// the host has. It comes once for an invite.
    Replaced,
    /// Sharing is over for this guest. The project stays open as it is.
    Ended { reason: BridgeShareEnding },
    /// For a host: what came of asking the router to let people outside
    /// this network in.
    Reach { reach: BridgeShareReach },
    /// For a host: what came of asking a relay for a room.
    Relayed { relayed: BridgeShareRelayed },
    /// Something about the project's footage changed: who has what, a
    /// transfer, or an export being fetched for or done by someone else.
    /// Whoever shows any of it reads it again. `placed` is a footage item
    /// now being read from a file that has just arrived, so every picture
    /// of it on screen is stale.
    Footage { placed: bool },
    /// The person numbered `from` asks this machine to export `comp` and
    /// send them the file. Answered with [`ProjectReference::share_answer_export`].
    ExportAsked {
        job: String,
        from: u32,
        comp: String,
    },
    /// For a project kept online: whether Lumit's server is taking edits to
    /// it. It stops when the host's account no longer has Lumit Pro, and
    /// for a project that has grown too large once the server has kept as
    /// many edits past its last snapshot as it will:
    /// [`BridgeShareOnline::too_large`] says which, and is set before this
    /// comes. A machine that makes an edit then goes [`Self::Away`], and
    /// stays so until the server takes edits again. What it makes meanwhile
    /// is kept and merged as after any time away.
    Writable { writable: bool },
    /// For a project kept online: what [`ProjectReference::share_activity`]
    /// or [`ProjectReference::share_notes`] gives has changed. It comes as
    /// often as edits go in, which while someone is at work is many times a
    /// second, so the lists are worth reading again no more often than they
    /// are worth drawing.
    Activity,
    /// For a copy of a project kept online: another version of Lumit keeps
    /// the project, so this machine is [`Self::Away`] from it, and
    /// `versions` says what would let it back. It comes when that is found
    /// and again if it changes. The machine goes on asking, and is
    /// [`Self::Back`] by itself once the two match.
    /// [`ProjectReference::share_online_versions`] gives the same at any
    /// time.
    Versions { versions: BridgeShareVersions },
    /// For a machine that can only look at a project kept online
    /// ([`BridgeShareOnline::look`]): something the person made on it has
    /// just been taken back, as everything made there is. Without a word
    /// here that reads as a fault. It comes at the start of a run of such
    /// edits and not again until they have stopped for two seconds, so
    /// dragging something says it once.
    LookOnly,
    /// For a project kept online: the invite that can only look is now on
    /// or off ([`BridgeShareOnline::look_on`]). It comes to every machine
    /// in the project when the host turns it, and once on joining a
    /// project that has it on.
    LookInvite { look_on: bool },
    /// For a project kept online: it has grown too large to go on being
    /// kept, or fits again ([`BridgeShareOnline::too_large`]). On each
    /// machine that finds it out, when it does, and when that changes.
    TooLarge { too_large: bool },
}

pub type ShareEventStream = StreamSink<BridgeShareEvent>;

/// One edit this machine made while away that was dropped on its return,
/// because what it was to was gone by then.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareDropped {
    /// What the edit is called in the History list, in the engine's English.
    pub step: String,
    /// The composition it was in, or the project item it was to, by the
    /// name it had on this machine before the merge. `None` when the edit
    /// names nothing this machine still had.
    pub item: Option<String>,
    /// The layer it was to, by the name it had here, for an edit to one.
    pub layer: Option<String>,
}

#[frb(ignore)]
fn dropped(dropped: lumit_share::Dropped) -> BridgeShareDropped {
    BridgeShareDropped {
        step: dropped.step,
        item: dropped.item,
        layer: dropped.layer,
    }
}

/// This machine's own project from just before a return that held back or
/// dropped any of its edits, and what that return dropped.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareReturnCopy {
    /// The project file. Everything the person had made by then is in it.
    /// It is a project of its own, and opening it joins nothing.
    pub path: String,
    /// When it was kept, in seconds since 1970.
    pub at: i64,
    /// How many edits that return dropped: the `refused` of its
    /// [`BridgeShareEvent::Back`]. 0 for a return that only held edits
    /// back.
    pub refused: u32,
    /// What the first twenty of those were to, as that `Back` named them.
    /// Kept beside the file, so it is still known after Lumit closes.
    pub dropped: Vec<BridgeShareDropped>,
}

/// How this machine stands to a project kept online that another version
/// of Lumit keeps. Everyone in a project has to run the Lumit that keeps
/// it. `version` is that Lumit's, as it is shown: `0.6.0`.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareVersions {
    /// A newer Lumit keeps it. This machine has to be updated to `version`.
    Update { version: String },
    /// An older Lumit keeps it, and this is the machine of its host, who
    /// may move it forward to this Lumit. Nobody on the older Lumit can
    /// join after that until they update, so the person is asked first.
    /// On a yes: [`ProjectReference::share_online_forward`] for a copy that
    /// is open, [`online_forward`] for an invite being joined.
    Movable { version: String },
    /// An older Lumit keeps it, and this is a guest's machine. The host
    /// has to open the project with the newer Lumit first.
    HostFirst { version: String },
}

/// How moving a project kept online forward to this Lumit went.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeOnlineForward {
    /// The project is this Lumit's now, or was already. Everyone connected
    /// was cut off to find that out. A copy that is open joins by itself,
    /// and an invite is joined by asking [`join_shared_project`] again.
    Moved,
    /// This Lumit could not read what the older one stored, or an edit in
    /// it would not apply. Nothing was changed: the project is still the
    /// older Lumit's, and still works there.
    Unreadable,
    /// It is not this machine's to move. Lumit's server does not know the
    /// account signed in here for the host's.
    NotHost,
    /// A newer Lumit than this one keeps the project by now. This machine
    /// has to be updated to `version`.
    Update { version: String },
    /// Lumit's server would not take it. `code` is its own word for why.
    Refused { code: String },
    /// No door to Lumit's server is open, or it could not be reached
    /// through the one that is.
    Unreachable,
    /// The project is not kept online, the link is not an invite to one or
    /// lacks its password, or people on the older Lumit went on editing
    /// through every try.
    Failed,
}

/// How starting to share went.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareStarted {
    /// Sharing, and listening on `port`. `key` is the invite's secret, to
    /// hand back to [`ProjectReference::share`] when this project is shared
    /// again, so the invite people already hold still finds it, with the
    /// password it had. `restored`
    /// edits made since the project was last saved were put back first: the
    /// project had been closed without saving, and the people coming back
    /// were working on a document with them in.
    Sharing {
        port: u16,
        key: String,
        restored: u32,
    },
    /// Something else on this machine has the port.
    PortInUse,
    /// The project is already shared, or the system would not listen.
    Failed,
}

/// How joining a shared project went.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeJoinOutcome {
    Joined {
        project: ProjectReference,
    },
    /// The text is not an invite.
    BadInvite,
    /// The host set a password, and none was given with the invite.
    PasswordNeeded,
    /// Nobody answered at that address, or the invite was not accepted. A
    /// wrong password looks the same: the host does not answer to one.
    Unreachable,
    VersionMismatch {
        host: String,
    },
    /// The project is kept online by another version of Lumit, and
    /// `versions` says which and what would let this machine in. The two
    /// refusals are [`BridgeShareVersions::Update`] and
    /// [`BridgeShareVersions::HostFirst`]. On
    /// [`BridgeShareVersions::Movable`] the person is asked, and on a yes
    /// [`online_forward`] and then this again.
    Versions {
        versions: BridgeShareVersions,
    },
    Full,
    Unsafe,
    Failed,
}

/// Edits this guest made while its host was away that touch something the
/// host's side changed too. Held until the person chooses.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareConflict {
    /// What the first held edit is called in the History list, in the
    /// engine's English.
    pub step: String,
    /// The project item it touched, or the composition its layer is in.
    pub item: Option<String>,
    pub layer: Option<String>,
    /// How many edits are held.
    pub edits: u32,
}

/// How putting a project online went.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeOnlineStarted {
    /// It is kept online, and `link` is the invite to hand out.
    Online { link: String },
    /// Lumit's server would not keep it. `code` is the server's own word
    /// for why, or the door's: `signed_out` with nobody signed in.
    Refused { code: String },
    /// No door to Lumit's server is open, or it could not be reached
    /// through the one that is.
    Unreachable,
    /// The project is shared already, another is being put online just
    /// now, or it is more than the server keeps.
    Failed,
}

/// What a project that is kept online is to this machine.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareOnline {
    /// This is the machine of the person whose account keeps the project
    /// online, who can take people out, make a new invite and stop keeping
    /// it. As Lumit's server said when it was last found.
    pub host: bool,
    /// Lumit's server is taking edits. See [`BridgeShareEvent::Writable`].
    /// False with `too_large` true is the project having grown too large.
    /// False without it is the host's account having no Lumit Pro.
    pub writable: bool,
    /// The project has grown too large for Lumit's server to keep whole:
    /// this machine went to store it as it stands and it did not fit.
    ///
    /// With `writable` still true it is a warning. Edits go in as before,
    /// for a while: the server keeps only so many past the last time the
    /// project was stored whole, and then takes no more. Making the project
    /// smaller before then mends it, and this goes false the next time this
    /// machine stores it, which is after some two thousand more edits.
    ///
    /// With `writable` false that point has been reached. Nothing made on
    /// this machine reaches anyone. It is away, what it makes is kept on
    /// disk as for any time away, and nothing is dropped. No edit can make
    /// the project smaller now, since none goes in. It ends when someone's
    /// Lumit does store the project, which this machine sees and comes
    /// back for, or when the host stops keeping it online.
    pub too_large: bool,
    /// This machine is without the server just now, and working on alone.
    pub away: bool,
    /// What Lumit's server knows the project by, for the calls the
    /// interface makes to it itself: footage, and the host's own.
    pub id: String,
    /// What those calls show the server, as `X-Lumit-Online`.
    pub access: String,
    /// The invite to hand out.
    pub link: String,
    /// This machine can only look: it joined by the invite that changes
    /// nothing. An edit made on it is taken back at once, and it gives the
    /// server no footage. The interface says so, and does not offer what
    /// would change the project.
    pub look: bool,
    /// The invite that can only look, to hand out beside `link`: whoever
    /// joins by it sees the project and the people, follows, and is seen,
    /// and changes nothing. It opens by itself, whatever password `link`
    /// needs: a password guards editing, and nobody who only looks is
    /// given it. So anyone this link reaches can read the project, while
    /// `look_on` is true. A new invite replaces it with `link`. On a
    /// machine that can only look it is the same as `link`.
    pub look_link: String,
    /// The invite that can only look is on. A project starts with it off,
    /// and off it opens nothing, whoever holds `look_link`. The host turns
    /// it with [`ProjectReference::share_online_look`]. While it is on,
    /// whoever holds `link` can read the project without its password,
    /// since `look_link` can be worked out from `link`: a password guards
    /// editing and nothing else. As Lumit's server last said, so false for
    /// a copy opened again until it has found the server.
    pub look_on: bool,
}

/// A new invite for a project kept online, before the server has it.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeOnlineMove {
    /// The new invite. Handed to [`ProjectReference::share_online_moved`]
    /// once the server has taken `body`, and to people from then on.
    pub link: String,
    /// The body of the call that moves the project to the new invite,
    /// `PUT /v1/online/<id>/access`, to send as it is.
    pub body: String,
}

/// Which doors a shared project is reached through from this machine.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy)]
pub struct BridgeShareDoors {
    /// The door to Lumit's relay, which [`share_cloud_door`] names.
    pub relay: bool,
    /// The door to projects kept online, which [`share_online_door`] names.
    pub online: bool,
}

/// One footage item of a project kept online, as this machine has it.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeOnlineFootage {
    pub item: FootageReference,
    pub name: String,
    /// What Lumit's server files it under, `<hash32>-<size>`, which is what
    /// its folder is called in this machine's own cache of sent footage. An
    /// item whose file was never fingerprinted goes by its own id.
    pub key: String,
    /// Where it is on this machine.
    pub here: BridgeFootageHere,
    /// A composition uses it.
    pub used: bool,
    /// How big the original is here. 0 when this machine has not got it.
    pub bytes: u64,
}

/// A composition some of a person's edits were in.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeSittingComp {
    /// Its id, which it keeps whatever it is called.
    pub id: String,
    /// What it is called now. Empty when it is no longer in the project.
    pub name: String,
    /// The composition itself, while it is still in the project.
    pub comp: Option<CompositionReference>,
}

/// Some words someone left in a project kept online, with where they were
/// when they wrote them. A note is no part of the document: it is not in
/// the project's file, and an undo does not touch it.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareNote {
    /// Tells one note from another.
    pub id: String,
    /// Who wrote it, as they were called then.
    pub name: String,
    /// This machine's person wrote it.
    pub mine: bool,
    /// When it was written, in seconds since 1970, by the clock of the
    /// machine it was written on.
    pub at: i64,
    /// The words. Five hundred characters at most.
    pub text: String,
    /// The composition its writer had open, when they had one.
    pub comp: Option<BridgeSittingComp>,
    /// The frame their playhead was on in it. `None` when they had no
    /// composition open, or it is no longer in the project.
    pub frame: Option<i64>,
    /// The layer they had chosen, while it is still in that composition.
    pub layer: Option<LayerReference>,
    /// It was left while this machine was not there to see it, and its
    /// person has not been shown the list since. For this machine alone.
    pub is_new: bool,
    /// It was written here and is not in the project yet: this machine is
    /// away from the server, or it was written this moment. It is sent
    /// when the server is found, and nobody else has it until then.
    pub waiting: bool,
}

/// One person's edits to a project kept online at one sitting: a run of
/// them with no gap of more than ten minutes.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareSitting {
    /// What the person was called when they made the last of them.
    pub name: String,
    /// They were made on this machine, in this copy of the project.
    pub mine: bool,
    /// When the sitting began and ended, in seconds since 1970, by the
    /// clock of the machine the edits were made on. Never later than this
    /// machine's own clock says it is.
    pub began: i64,
    pub ended: i64,
    /// How many edits. Each is one step of that person's undo history.
    pub edits: u32,
    /// The compositions the edits were in, in the order they were first
    /// touched, and at most 32 of them. An edit that is in no composition,
    /// such as renaming an item in the Project panel, counts in `edits` and
    /// adds nothing here.
    pub comps: Vec<BridgeSittingComp>,
    /// Some of it happened while this machine was not there to see: before
    /// it joined, while its copy was closed, or while it was away. Never
    /// true of this machine's own. It stays true until
    /// [`ProjectReference::share_activity_seen`] is called.
    pub is_new: bool,
}

/// The port a host listens on unless the person picks another.
#[frb(sync)]
pub fn share_default_port() -> u16 {
    lumit_share::DEFAULT_PORT
}

/// The invite in `text`, written as a link, or `None` when there is none in
/// it. What tells a link Lumit was started with from a file to open, and
/// what tidies whatever a person pasted.
#[frb(sync)]
pub fn share_link_in(text: String) -> Option<String> {
    let invite = text.parse::<Invite>().ok()?;
    Some(invite.to_string())
}

/// Where one footage item is on this machine, in a shared project.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFootageHere {
    /// The file itself: this person's own, or one fetched from someone.
    Original,
    /// A stand-in someone sent: the whole clip, small, to cut with.
    StandIn,
    /// Neither.
    Missing,
}

/// One footage item as a shared project knows it from this machine.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeFootageShare {
    pub here: BridgeFootageHere,
    /// The people who have the original, by the number the people list
    /// gives them. Empty when nobody here has it.
    pub holders: Vec<u32>,
    /// How big the original is on a holder's disk, which is what fetching
    /// it costs. 0 when nobody has it.
    pub bytes: u64,
    /// Nobody could send it when it was last asked for.
    pub refused: bool,
}

/// One transfer in flight.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeShareTransfer {
    /// The footage item's name, or empty for an export's file.
    pub name: String,
    /// The original, and otherwise a stand-in or the frames of one.
    pub original: bool,
    /// This machine is sending it, and otherwise taking it.
    pub sending: bool,
    pub done: u64,
    pub total: u64,
}

/// A footage item a composition uses that this machine has no original of.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub struct BridgeFootageLack {
    pub name: String,
    /// There is a stand-in of it here, and otherwise nothing at all.
    pub stand_in: bool,
    pub holders: Vec<u32>,
    pub bytes: u64,
}

/// How fetching what an export lacks is getting on.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareFetching {
    Idle,
    /// `done` of `total` bytes are here. Both 0 while the others' machines
    /// are still making the files.
    Working {
        done: u64,
        total: u64,
    },
    /// Everything arrived and the export is in the queue under `id`.
    Queued {
        id: u32,
    },
    /// Nobody could send something, or the export would not queue.
    Failed,
}

/// How an export asked of someone else's machine is getting on.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareAsking {
    Idle,
    /// Sent, and the other person has not answered.
    Waiting,
    Running {
        frame: u64,
        total: u64,
    },
    /// Their export finished and the file is on its way here.
    Fetching {
        done: u64,
        total: u64,
    },
    Done {
        path: String,
    },
    /// `why` is `refused` when they said no, `lacking` when their machine
    /// has not all the footage either, and anything else when it stopped.
    Failed {
        why: String,
    },
}

/// How fast this machine sends and takes footage in a shared project, in
/// kilobytes a second. Nought is as fast as the line goes. For the machine,
/// not one project, and takes effect on transfers already running.
#[frb(sync)]
pub fn share_set_limits(up_kilobytes: u32, down_kilobytes: u32) {
    let bytes = |kilobytes: u32| u64::from(kilobytes) * 1024;
    crate::footage::LIMITS.set(bytes(up_kilobytes), bytes(down_kilobytes));
}

/// Whether this machine makes stand-ins of its own footage for the others
/// (`give`), and whether it asks for a stand-in of what it lacks without
/// being told to (`take`).
#[frb(sync)]
pub fn share_set_footage(give: bool, take: bool) {
    crate::footage::set_sharing(give, take);
}

#[frb(ignore)]
fn asking(asking: crate::footage::Asking) -> BridgeShareAsking {
    use crate::footage::Asking;
    match asking {
        Asking::Idle => BridgeShareAsking::Idle,
        Asking::Waiting => BridgeShareAsking::Waiting,
        Asking::Running { frame, total } => BridgeShareAsking::Running { frame, total },
        Asking::Fetching { done, total } => BridgeShareAsking::Fetching { done, total },
        Asking::Done { path } => BridgeShareAsking::Done { path },
        Asking::Failed { why } => BridgeShareAsking::Failed { why },
    }
}

/// Who has the original of `item`, and how big it is.
#[frb(ignore)]
fn holders_of(project: Uuid, item: Uuid) -> (Vec<u32>, u64) {
    let holders = with_sharing(project, |sharing| sharing.holders(item)).unwrap_or_default();
    let bytes = holders.iter().map(|(_, bytes)| *bytes).max().unwrap_or(0);
    (holders.into_iter().map(|(peer, _)| peer).collect(), bytes)
}

impl crate::api::footage::FootageReference {
    /// Where this footage item is on this machine and who has the original,
    /// while its project is shared. An item of a project that is not shared
    /// reads as the original or missing, with nobody holding it.
    #[frb(sync)]
    pub fn share_state(&self) -> Result<BridgeFootageShare, BridgeError> {
        use crate::footage::Here;
        let project = ProjectReference::new(self.project);
        let doc = {
            let state = project.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let here = match doc.item(self.id) {
            Some(lumit_core::model::ProjectItem::Footage(footage)) => {
                match crate::footage::here(footage) {
                    Here::Original(_) => BridgeFootageHere::Original,
                    Here::StandIn(_) => BridgeFootageHere::StandIn,
                    Here::Missing => BridgeFootageHere::Missing,
                }
            }
            _ => BridgeFootageHere::Missing,
        };
        let (holders, bytes) = holders_of(self.project, self.id);
        let wanted = lumit_share::Wanted::StandIn { item: self.id };
        let refused = crate::footage::carrier(self.project).is_some_and(|c| c.was_refused(&wanted));
        Ok(BridgeFootageShare {
            here,
            holders,
            bytes,
            refused,
        })
    }

    /// Ask the others for this footage item: a stand-in of it, or with
    /// `original` the file itself. Does nothing when the project is not
    /// shared. What comes of it arrives as [`BridgeShareEvent::Footage`].
    #[frb(sync)]
    pub fn share_fetch(&self, original: bool) -> Result<(), BridgeError> {
        if let Some(carrier) = crate::footage::carrier(self.project) {
            carrier.fetch(self.id, original);
        }
        Ok(())
    }

    /// The file this machine can give Lumit's server for this footage item
    /// of a project kept online: a stand-in of it, made now if there is
    /// none yet, or with `original` the file itself. It is what
    /// [`ProjectReference::online_seal`] is given as its source. `None`
    /// when this machine has nothing to give, or is set not to.
    ///
    /// Not sync: making a stand-in is an encode, as long as the clip is.
    /// [`online_cancel`] does not stop it.
    pub fn online_give(&self, original: bool) -> Result<Option<String>, BridgeError> {
        let Some((carrier, wanted)) = online_item(self, original) else {
            return Ok(None);
        };
        let made = carrier.make(&wanted, &AtomicBool::new(false));
        Ok(made.map(|path| path.to_string_lossy().into_owned()))
    }

    /// Where a file fetched from Lumit's server for this footage item is to
    /// be opened to, which is [`ProjectReference::online_open`]'s `out`: its
    /// place in this machine's cache of sent footage, where a stand-in or an
    /// original a person sent would go. `None` when the project is not kept
    /// online or has no such item.
    #[frb(sync)]
    pub fn online_room(&self, original: bool) -> Result<Option<String>, BridgeError> {
        let room = online_item(self, original).and_then(|(carrier, wanted)| carrier.room(&wanted));
        Ok(room.map(|path| path.to_string_lossy().into_owned()))
    }

    /// A file fetched from Lumit's server for this footage item has been
    /// opened into [`Self::online_room`]. The project reads the item from
    /// it from now on, by the road a file sent by a person takes: an
    /// original is held against the item's fingerprint first and thrown
    /// away if it is not that file, and a stand-in is only taken where
    /// there was nothing. What comes of it arrives as
    /// [`BridgeShareEvent::Footage`].
    #[frb(sync)]
    pub fn online_arrived(&self, original: bool) -> Result<(), BridgeError> {
        if let Some((carrier, wanted)) = online_item(self, original) {
            if let Some(path) = carrier.room(&wanted).filter(|path| path.is_file()) {
                carrier.told(News::Arrived { wanted, path });
            }
        }
        Ok(())
    }
}

impl CompositionReference {
    /// The footage this composition uses, through every composition inside
    /// it, that this machine has no original of. Empty when the project is
    /// not shared: there is then nobody to get any of it from.
    #[frb(sync)]
    pub fn share_lacking(&self) -> Result<Vec<BridgeFootageLack>, BridgeError> {
        if crate::footage::carrier(self.project).is_none() {
            return Ok(Vec::new());
        }
        let project = ProjectReference::new(self.project);
        let doc = {
            let state = project.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let lacks = crate::footage::lacking(&doc, self.id).into_iter();
        Ok(lacks
            .filter_map(|id| {
                let Some(lumit_core::model::ProjectItem::Footage(footage)) = doc.item(id) else {
                    return None;
                };
                let (holders, bytes) = holders_of(self.project, id);
                Some(BridgeFootageLack {
                    name: footage.name.clone(),
                    stand_in: crate::footage::here(footage) != crate::footage::Here::Missing,
                    holders,
                    bytes,
                })
            })
            .collect())
    }

    /// Fetch what an export of this composition lacks and then queue it, as
    /// [`Self::queue_export`] would: the originals whole, or with `parts`
    /// only the frames the export reads, at a quality fit to deliver from.
    /// Answers at once. [`ProjectReference::share_fetching`] says how it is
    /// going, and a [`BridgeShareEvent::Footage`] when that has changed.
    #[frb(sync)]
    pub fn share_fetch_export(
        &self,
        spec: crate::api::export::BridgeExportSpec,
        path: String,
        parts: bool,
        start: bool,
    ) -> Result<(), BridgeError> {
        let name = self.get_settings()?.name;
        if let Some(carrier) = crate::footage::carrier(self.project) {
            carrier.fetch_then_export(self.id, name, spec, path, parts, start);
        }
        Ok(())
    }

    /// Ask the person numbered `to` to export this composition on their
    /// machine and send the file back, to be written at `path`. They are
    /// asked first. [`ProjectReference::share_asking`] says how it is going.
    #[frb(sync)]
    pub fn share_ask_export(
        &self,
        to: u32,
        spec: crate::api::export::BridgeExportSpec,
        path: String,
    ) -> Result<(), BridgeError> {
        if let Some(carrier) = crate::footage::carrier(self.project) {
            carrier.ask_export(to, self.id, &spec, &path);
        }
        Ok(())
    }
}

/// Whether the invite in `text` needs a password given with it. False for
/// anything that is not an invite.
#[frb(sync)]
pub fn share_link_locked(text: String) -> bool {
    text.parse::<Invite>().is_ok_and(|invite| invite.locked)
}

/// Whether the invite in `text` is to a project that is kept online, which
/// is joined through the door [`share_online_door`] names. False for
/// anything that is not an invite.
#[frb(sync)]
pub fn share_link_online(text: String) -> bool {
    text.parse::<Invite>()
        .is_ok_and(|invite| invite.is_online())
}

/// What a host keeps to share by the same invite again: its secret, and
/// after a full stop what its password came to if it set one.
#[frb(ignore)]
fn kept_text(host: &lumit_share::Host) -> String {
    let key = lumit_share::key_text(&host.key());
    match host.lock() {
        Some(lock) => format!("{key}.{}", lock.text()),
        None => key,
    }
}

/// The invite in `text` with `password` given, or `None` when it needs one
/// and has none.
#[frb(ignore)]
fn unlocked(invite: Invite, password: Option<&str>) -> Option<Invite> {
    match password.filter(|password| !password.is_empty()) {
        Some(password) => Some(invite.unlocked(password)),
        None if invite.locked => None,
        None => Some(invite),
    }
}

/// The port a relay listens on unless its owner picked another, for an
/// address typed without one.
#[frb(sync)]
pub fn share_relay_port() -> u16 {
    lumit_share::RELAY_PORT
}

/// What Lumit's own relay is called in an invite, to give [`ProjectReference::share`]
/// as its relay. It is reached through the door [`share_cloud_door`] names.
#[frb(sync)]
pub fn share_cloud_relay() -> String {
    lumit_share::CLOUD_RELAY.to_owned()
}

/// Say which port on this machine the door to Lumit's own relay is on, or
/// `None` when it has shut. The interface keeps the door, since it is what
/// holds the account the relay asks a host for. With none, an invite that
/// names the relay is tried at its other addresses only.
#[frb(sync)]
pub fn share_cloud_door(port: Option<u16>) {
    lumit_share::set_cloud_door(port);
}

/// Say which port on this machine the door to projects kept online is on,
/// or `None` when it has shut. The interface keeps the door, since it is
/// what can reach Lumit's server and holds the account the server knows a
/// host by. With none, a project kept online is away: it is edited on, and
/// looked for again the moment a door opens.
#[frb(sync)]
pub fn share_online_door(port: Option<u16>) {
    lumit_share::set_online_door(port);
}

/// Stop every file being sealed or opened by
/// [`ProjectReference::online_seal`] and [`ProjectReference::online_open`].
/// Harmless when there is none.
#[frb(sync)]
pub fn online_cancel() {
    crate::footage::online_cancel();
}

/// A project is being put online. One at a time: the store is made a
/// member's before the server has answered, and two at once would each
/// undo the other.
static STARTING: AtomicBool = AtomicBool::new(false);

/// What starts one end of a shared project once its project is open.
#[frb(ignore)]
type Start =
    Box<dyn FnOnce(Arc<DocumentStore>, lumit_share::Events) -> Result<Sharing, ShareError>>;

/// The footage item a reference names and the carrier of its project, for
/// a project that is kept online. `None` for any other.
#[frb(ignore)]
fn online_item(
    item: &FootageReference,
    original: bool,
) -> Option<(Arc<crate::footage::Carrier>, Wanted)> {
    let online = |sharing: &Sharing| matches!(sharing, Sharing::Online(_));
    if with_sharing(item.project, online) != Some(true) {
        return None;
    }
    let wanted = if original {
        Wanted::Original { item: item.id }
    } else {
        Wanted::StandIn { item: item.id }
    };
    Some((crate::footage::carrier(item.project)?, wanted))
}

/// A relay's address as typed, with the usual port when it has none.
#[frb(ignore)]
fn at_relay(typed: &str) -> String {
    let typed = typed.trim();
    // A name or an IPv4 address with a port has one colon, and an IPv6
    // address with one ends its brackets before it.
    let ported = match typed.rsplit_once(':') {
        Some((host, port)) => {
            port.parse::<u16>().is_ok() && (!host.contains(':') || host.ends_with(']'))
        }
        None => false,
    };
    if ported || typed.is_empty() {
        return typed.to_owned();
    }
    let bare = typed.contains(':') && !typed.starts_with('[');
    let (open, close) = if bare { ("[", "]") } else { ("", "") };
    format!("{open}{typed}{close}:{}", lumit_share::RELAY_PORT)
}

#[frb(ignore)]
fn ending(ending: Ending) -> BridgeShareEnding {
    match ending {
        Ending::Closed => BridgeShareEnding::Closed,
        Ending::Refused(Refusal::Version { host }) => BridgeShareEnding::VersionMismatch { host },
        Ending::Refused(Refusal::Full) => BridgeShareEnding::Full,
        Ending::Unsafe => BridgeShareEnding::Unsafe,
        Ending::Removed => BridgeShareEnding::Removed,
    }
}

#[frb(ignore)]
fn versions(versions: Versions) -> BridgeShareVersions {
    match versions {
        Versions::Update { version } => BridgeShareVersions::Update { version },
        Versions::Movable { version } => BridgeShareVersions::Movable { version },
        Versions::HostFirst { version } => BridgeShareVersions::HostFirst { version },
    }
}

/// What came of moving a project forward, as the frontend hears it.
#[frb(ignore)]
fn forwarded(moved: Result<(), ShareError>) -> BridgeOnlineForward {
    match moved {
        Ok(()) => BridgeOnlineForward::Moved,
        Err(ShareError::Unsafe) => BridgeOnlineForward::Unreadable,
        Err(ShareError::Versions(Versions::Update { version })) => {
            BridgeOnlineForward::Update { version }
        }
        Err(ShareError::Versions(_)) => BridgeOnlineForward::NotHost,
        Err(ShareError::Server(code)) if code == "forbidden" => BridgeOnlineForward::NotHost,
        Err(ShareError::Server(code)) => BridgeOnlineForward::Refused { code },
        Err(ShareError::Unreachable | ShareError::Io(_)) => BridgeOnlineForward::Unreachable,
        Err(_) => BridgeOnlineForward::Failed,
    }
}

/// A person as the frontend draws them. A composition or a time that is not
/// in this machine's document yet is left out rather than guessed at.
#[frb(ignore)]
fn person(project: Uuid, doc: &Document, me: u32, person: Person) -> BridgeSharePerson {
    let Presence {
        comp,
        layers,
        playhead,
        cursor,
        properties,
        keys,
        changing,
    } = person.presence;
    let comp = comp.and_then(|id| doc.comp(id));
    // They name rows of that composition, so they go when it does.
    let (properties, keys) = if comp.is_some() {
        (properties, keys)
    } else {
        (Vec::new(), Vec::new())
    };
    BridgeSharePerson {
        id: person.id,
        name: person.name,
        colour: person.colour,
        me: person.id == me,
        host: person.host,
        look: person.look,
        comp: comp.map(|c| CompositionReference::new(project, c.id)),
        layers: comp.map_or_else(Vec::new, |c| {
            let here = layers
                .iter()
                .filter(|id| c.layers.iter().any(|l| l.id == **id));
            here.map(|id| LayerReference::new(project, c.id, *id))
                .collect()
        }),
        playhead: comp.zip(playhead).map(|(c, t)| c.frame_rate.frame_at(t)),
        cursor_x: cursor.map(|(x, _)| x),
        cursor_y: cursor.map(|(_, y)| y),
        properties,
        keys,
        changing,
    }
}

/// Turn a shared project's events into the frontend's and send them down
/// `sink`, if anyone is listening.
#[frb(ignore)]
fn events_for(
    project: Uuid,
    store: Arc<DocumentStore>,
    sink: Arc<Option<ShareEventStream>>,
) -> lumit_share::Events {
    Arc::new(move |event| {
        let event = match event {
            // Not for the frontend as it comes: the footage carrier reads it
            // and says what, if anything, the person needs to see.
            Event::Note { from, body } => {
                if let Some(carrier) = crate::footage::carrier(project) {
                    carrier.note(from, body);
                }
                return;
            }
            Event::People { me, people } => {
                let doc = store.snapshot();
                let people = people.into_iter();
                BridgeShareEvent::People {
                    people: people.map(|p| person(project, &doc, me, p)).collect(),
                }
            }
            Event::Away => BridgeShareEvent::Away,
            Event::Elsewhere => BridgeShareEvent::Elsewhere,
            Event::Replaced => BridgeShareEvent::Replaced,
            Event::Back {
                held,
                refused,
                dropped,
            } => BridgeShareEvent::Back {
                held: held as u32,
                refused: refused as u32,
                dropped: dropped.into_iter().map(self::dropped).collect(),
            },
            Event::Ended(reason) => BridgeShareEvent::Ended {
                reason: ending(reason),
            },
            Event::Reach(now) => BridgeShareEvent::Reach { reach: reach(now) },
            Event::Relayed(now) => BridgeShareEvent::Relayed {
                relayed: relayed(now),
            },
            Event::Writable(writable) => BridgeShareEvent::Writable { writable },
            Event::Activity => BridgeShareEvent::Activity,
            Event::Versions(apart) => BridgeShareEvent::Versions {
                versions: versions(apart),
            },
            Event::LookOnly => BridgeShareEvent::LookOnly,
            Event::LookInvite(look_on) => BridgeShareEvent::LookInvite { look_on },
            Event::TooLarge(too_large) => BridgeShareEvent::TooLarge { too_large },
        };
        if let Some(sink) = sink.as_ref() {
            _ = sink.add(event);
        }
    })
}

/// Start carrying footage for a project that has just become shared.
#[frb(ignore)]
fn carry(
    project: Uuid,
    store: Arc<DocumentStore>,
    sink: Arc<Option<ShareEventStream>>,
    sharing: &Sharing,
) {
    use crate::footage::{Carrier, Told};
    let tell: crate::footage::Tell = Arc::new(move |told| {
        let event = match told {
            Told::Changed => BridgeShareEvent::Footage { placed: false },
            Told::Placed => BridgeShareEvent::Footage { placed: true },
            Told::Asked { job, from, comp } => BridgeShareEvent::ExportAsked {
                job: job.to_string(),
                from,
                comp,
            },
        };
        if let Some(sink) = sink.as_ref() {
            _ = sink.add(event);
        }
    });
    let carrier = Carrier::start(project, store, tell);
    sharing.carry_footage(carrier, crate::footage::LIMITS.clone());
}

/// File a sharing under a project that the registry has no project for.
#[cfg(test)]
#[frb(ignore)]
pub(crate) fn share_for_test(project: Uuid, sharing: Sharing) {
    if let Ok(mut shared) = SHARED.lock() {
        shared.insert(project, sharing);
    }
}

/// Do something with `project`'s sharing, if it is shared. Never from a
/// share thread: see [`SHARED`].
#[frb(ignore)]
pub(crate) fn with_sharing<T>(project: Uuid, with: impl FnOnce(&Sharing) -> T) -> Option<T> {
    let shared = SHARED.lock().ok()?;
    shared.get(&project).map(with)
}

/// Let go of one end of a shared project whose project is closing.
///
/// A project with a file can be opened again, so nobody is told it is over.
/// A host's guests keep what they do next and bring it back when the project
/// is shared again, and a guest without its host keeps its edits on disk for
/// when its copy is opened. A project that was never saved is gone for good,
/// which is the same as stopping.
#[frb(ignore)]
fn let_go(project: Uuid, sharing: Sharing) {
    let state = ProjectReference::new(project).state().ok();
    let saved = state
        .and_then(|state| state.read().ok().map(|state| state.path.is_some()))
        .unwrap_or(false);
    if !saved {
        sharing.stop();
    }
}

/// Let go of `project`'s sharing because the project is closing, whichever
/// end this is. Called while the project is still registered.
#[frb(ignore)]
pub(crate) fn stop(project: Uuid) {
    let sharing = SHARED.lock().ok().and_then(|mut s| s.remove(&project));
    crate::footage::Carrier::stop(project);
    // Let go of here, outside the registry lock.
    if let Some(sharing) = sharing {
        let_go(project, sharing);
    }
}

/// [`stop`] for every project. An open replaces whatever was loaded.
#[frb(ignore)]
pub(crate) fn stop_all() {
    let shared = SHARED.lock().map(|mut s| std::mem::take(&mut *s));
    for (project, sharing) in shared.into_iter().flatten() {
        crate::footage::Carrier::stop(project);
        let_go(project, sharing);
    }
}

/// What a save of `project` writes while this machine hosts it, or is its
/// guest with the host away: the document, the store's revision at it, and
/// how many of the edits kept are in it, to hand to [`saved`]. `None` for
/// any other project.
#[frb(ignore)]
pub(crate) fn saving(project: Uuid) -> Option<(Arc<Document>, u64, usize)> {
    let shared = SHARED.lock().ok()?;
    match shared.get(&project) {
        Some(Sharing::Host(host)) => Some(host.saving()),
        Some(Sharing::Guest(guest)) => guest.saving(),
        Some(Sharing::Online(member)) => member.saving(),
        None => None,
    }
}

/// `project`, whose document is `document`, was saved. A host lets go of
/// the first `mark` edits it keeps, which are in the file now. Saved while
/// nobody hosts it, whatever a host kept before goes: the file has moved on
/// from the document those edits were made on.
#[frb(ignore)]
pub(crate) fn saved(project: Uuid, document: Uuid, path: &Path, mark: Option<usize>) {
    let shared = SHARED.lock().ok();
    let hosted = shared.as_ref().and_then(|shared| shared.get(&project));
    // The copy is this file from now on, whatever it was saved as before.
    if matches!(hosted, Some(Sharing::Guest(_) | Sharing::Online(_))) {
        note_copy(document, Some(path));
    }
    match (hosted, mark) {
        (Some(Sharing::Host(host)), Some(mark)) => host.saved(mark),
        // Sharing started while the file was being written. Kept as it is.
        (Some(Sharing::Host(_)), None) => {}
        // A guest's edits stay kept, and the mark is what closing without
        // saving goes back to.
        (Some(Sharing::Guest(guest)), Some(mark)) => guest.saved(mark),
        (Some(Sharing::Online(member)), Some(mark)) => member.saved(mark),
        _ => lumit_share::forget(document),
    }
}

/// Where this machine notes which file its copy `document` of a shared
/// project is saved as.
#[frb(ignore)]
fn copy_note(document: Uuid) -> Option<PathBuf> {
    Some(lumit_project::shared_dir()?.join(format!("guest-{document}.at")))
}

/// Note that the copy `document` is saved at `path`, or has no file yet.
#[frb(ignore)]
pub(crate) fn note_copy(document: Uuid, path: Option<&Path>) {
    let Some(note) = copy_note(document) else {
        return;
    };
    let Some(path) = path else {
        let _ = std::fs::remove_file(note);
        return;
    };
    if let Some(dir) = note.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(note, path.to_string_lossy().as_bytes());
}

/// Whether the file at `path` is this machine's copy `document` of a shared
/// project, and so opens as part of it.
///
/// What is kept for a copy is filed by the document's id, and that is not
/// one file's alone: a project saved under another name, a version of it and
/// a file copied by hand all have the id of the file they came from. Opened
/// as the copy, any of them would have the shared project's document put in
/// place of its own, and the next save would write that over it.
///
/// So only the file the copy was last saved as is the copy. One noted
/// nowhere, as a copy an earlier Lumit kept is, is the first file opened.
/// So is one whose noted file has gone, which is a copy moved or renamed.
#[frb(ignore)]
pub(crate) fn copy_is_at(document: Uuid, path: &Path) -> bool {
    let noted = copy_note(document).and_then(|note| std::fs::read_to_string(note).ok());
    // As the disk names each, so two spellings of one path are one file.
    let named = |path: &Path| std::fs::canonicalize(path).ok();
    noted
        .and_then(|noted| named(Path::new(&noted)))
        .is_none_or(|noted| named(path) == Some(noted))
}

/// Carry on as the guest `project`'s file was left as: its host away, and
/// edits in it waiting to be merged. Or as the member of a project kept
/// online that it was left as. `start` is which.
#[frb(ignore)]
pub(crate) fn resume(
    project: &ProjectReference,
    start: impl FnOnce(Arc<DocumentStore>, lumit_share::Events) -> Result<Sharing, ShareError>,
    events: Option<ShareEventStream>,
) -> Result<(), BridgeError> {
    let store = {
        let state = project.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.clone()
    };
    let sink = Arc::new(events);
    let events = events_for(project.id, store.clone(), sink.clone());
    if let Ok(sharing) = start(store.clone(), events) {
        carry(project.id, store, sink, &sharing);
        SHARED
            .lock()
            .map_err(|_| BridgeError::WriteFailed)?
            .insert(project.id, sharing);
    }
    Ok(())
}

/// What to call a conflict: the first thing its edits touched that is still
/// in the document.
#[frb(ignore)]
fn describe(doc: &Document, conflict: &Conflict) -> BridgeShareConflict {
    let named = conflict.keys.iter().find_map(|(id, _)| {
        if let Some(item) = doc.item(*id) {
            return Some((item.name().to_owned(), None));
        }
        doc.items.iter().find_map(|item| match item {
            lumit_core::model::ProjectItem::Composition(c) => {
                let layer = c.layers.iter().find(|l| l.id == *id)?;
                Some((c.name.clone(), Some(layer.name.clone())))
            }
            _ => None,
        })
    });
    let (item, layer) = named.map_or((None, None), |(item, layer)| (Some(item), layer));
    BridgeShareConflict {
        step: conflict
            .ops
            .first()
            .map_or("", |(op, _)| op.name())
            .to_owned(),
        item,
        layer,
        edits: conflict.ops.len() as u32,
    }
}

impl ProjectReference {
    /// Share this project from this machine. Others join with the link
    /// [`Self::share_link`] gives.
    ///
    /// `name` is what the others see this person called. Port 0 takes any
    /// free one. `key` is what the last [`BridgeShareStarted::Sharing`] for
    /// this project gave, or `None` for a new invite. `password` is one every
    /// guest has to give as well as holding the link. Empty or `None` keeps
    /// whatever `key` was shared with, which is no password for a new
    /// invite. `outside` asks this
    /// network's router to send the port here, so people outside the network
    /// can join without a VPN. It answers later, as a
    /// [`BridgeShareEvent::Reach`]. `relay` is the `host:port` of a relay to
    /// keep a room at, for people the router does not let in, and answers as
    /// a [`BridgeShareEvent::Relayed`]. `events` is optional the way a project's
    /// change stream is, and for the same reason: nothing about sharing
    /// depends on someone watching.
    #[frb(sync)]
    #[allow(clippy::too_many_arguments)]
    pub fn share(
        &self,
        name: String,
        port: u16,
        key: Option<String>,
        password: Option<String>,
        outside: bool,
        relay: Option<String>,
        events: Option<ShareEventStream>,
    ) -> Result<BridgeShareStarted, BridgeError> {
        let (store, root) = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let root = state.path.as_deref().and_then(Path::parent);
            (state.store.clone(), root.map(Path::to_path_buf))
        };
        // A key that does not read as one is as good as none. What a
        // password came to last time follows it after a full stop. Worked
        // out before the registry is taken: a password is slow to hash on
        // purpose, and everything else that asks about sharing waits on that.
        let kept = key
            .as_deref()
            .map(|kept| kept.split_once('.').unwrap_or((kept, "")));
        let key = kept.and_then(|(key, _)| lumit_share::key_from(key));
        let lock = match password.filter(|password| !password.is_empty()) {
            Some(password) => lumit_share::Lock::new(&password).ok(),
            None => key
                .and(kept)
                .and_then(|(_, lock)| lumit_share::Lock::from_text(lock)),
        };
        let mut shared = SHARED.lock().map_err(|_| BridgeError::WriteFailed)?;
        if shared.contains_key(&self.id) {
            return Ok(BridgeShareStarted::Failed);
        }
        let sink = Arc::new(events);
        let events = events_for(self.id, store.clone(), sink.clone());
        Ok(
            match lumit_share::host(store.clone(), &name, LISTEN, port, key, lock, root, events) {
                Ok(host) => {
                    let (port, key) = (host.port(), kept_text(&host));
                    let restored = host.restored() as u32;
                    // A test never asks the machine's real router anything.
                    if outside && !cfg!(test) {
                        host.reach_out();
                    } else if !cfg!(test) {
                        host.reach_back();
                    }
                    if let Some(relay) = relay.as_deref().map(at_relay) {
                        host.relay_through(&relay);
                    }
                    let sharing = Sharing::Host(host);
                    carry(self.id, store, sink, &sharing);
                    shared.insert(self.id, sharing);
                    BridgeShareStarted::Sharing {
                        port,
                        key,
                        restored,
                    }
                }
                Err(ShareError::Io(e)) if e.kind() == ErrorKind::AddrInUse => {
                    BridgeShareStarted::PortInUse
                }
                Err(_) => BridgeShareStarted::Failed,
            },
        )
    }

    /// Whether people outside this network can get in, while this machine
    /// hosts the project. The events carry it as it changes.
    #[frb(sync)]
    pub fn share_reach(&self) -> Result<BridgeShareReach, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Host(host)) => reach(host.reach()),
            _ => BridgeShareReach::Off,
        })
    }

    /// Whether this machine has a room at a relay, while it hosts the
    /// project. The events carry it as it changes.
    #[frb(sync)]
    pub fn share_relayed(&self) -> Result<BridgeShareRelayed, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Host(host)) => relayed(host.relayed()),
            _ => BridgeShareRelayed::Off,
        })
    }

    /// The link to send to whoever is joining. It holds every way to this
    /// machine the engine knows of just now, so it is asked for again when
    /// the router or a relay answers. `address` is one more the person
    /// typed, for a VPN or a port forwarded by hand. `None` when this
    /// machine is not hosting the project. For a project that is kept
    /// online it is the invite anyone in it can hand out, and `address`
    /// means nothing.
    #[frb(sync)]
    pub fn share_link(&self, address: Option<String>) -> Result<Option<String>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Host(host)) => Some(host.invite_anywhere(address.as_deref()).to_string()),
            Some(Sharing::Online(member)) => Some(member.invite().to_string()),
            _ => None,
        })
    }

    /// The secret of the invite as it stands, to hand to [`Self::share`] the
    /// next time this project is shared. `None` when this machine is not
    /// hosting it.
    #[frb(sync)]
    pub fn share_key(&self) -> Result<Option<String>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Host(host)) => Some(kept_text(host)),
            _ => None,
        })
    }

    /// Everyone in the project right now, this person included. Empty when it
    /// is not shared. The events carry the same list as it changes.
    #[frb(sync)]
    pub fn share_people(&self) -> Result<Vec<BridgeSharePerson>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(sharing) = shared.get(&self.id) else {
            return Ok(Vec::new());
        };
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let (me, people) = (sharing.me(), sharing.people().into_iter());
        Ok(people.map(|p| person(self.id, &doc, me, p)).collect())
    }

    /// Whether this machine is a guest of the project. True straight after
    /// opening a guest's own copy that was closed with its host away: it
    /// carries on looking for the host, and what it finds comes down the
    /// events the open was given.
    #[frb(sync)]
    pub fn share_guest(&self) -> Result<bool, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(matches!(shared.get(&self.id), Some(Sharing::Guest(_))))
    }

    /// What this machine knows the project by from one run to the next, to
    /// file the key of its invite under. Not the same as a guest's copy's.
    #[frb(sync)]
    pub fn share_id(&self) -> Result<String, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.snapshot().id.to_string())
    }

    /// Stop sharing this project for everyone, or leave it if someone else
    /// hosts. The project stays open as it is. A host's invite is finished
    /// with: sharing again makes a new one.
    ///
    /// A project that is kept online is left, by its host too, and stays
    /// online for everyone else. Its host stops keeping it by asking
    /// Lumit's server first (`DELETE /v1/online/<id>`, with the account and
    /// what [`Self::share_online_state`] gives), which tells everyone else
    /// it is over, and then calling this.
    #[frb(sync)]
    pub fn stop_sharing(&self) -> Result<(), BridgeError> {
        let sharing = SHARED.lock().ok().and_then(|mut s| s.remove(&self.id));
        crate::footage::Carrier::stop(self.id);
        if let Some(sharing) = sharing {
            sharing.stop();
        }
        Ok(())
    }

    /// Take a guest out of the project this machine hosts, by the id the
    /// people list gives them. Their Lumit is told and stops coming back.
    /// The invite is replaced, so the one they hold stops working. Everyone
    /// still here is sent the new one, [`Self::share_link`] gives it, and
    /// [`Self::share_key`] the key to hand to [`Self::share`] next time.
    #[frb(sync)]
    pub fn share_remove(&self, person: u32) -> Result<(), BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        if let Some(Sharing::Host(host)) = shared.get(&self.id) {
            host.remove(person);
        }
        Ok(())
    }

    /// Give a guest that has lost its host a new invite to look for it by,
    /// for a host that has moved or made a new one. `password` goes with it
    /// when the host set one. False when the text is not an invite, it needs
    /// a password and has none, or this machine is not a guest of the
    /// project. The same for a project kept online whose host made a new
    /// invite while this machine was away.
    #[frb(sync)]
    pub fn share_reinvite(
        &self,
        invite: String,
        password: Option<String>,
    ) -> Result<bool, BridgeError> {
        // The password is hashed before the registry is taken, as in `share`.
        let invite = invite.parse::<Invite>().ok();
        let Some(invite) = invite.and_then(|invite| unlocked(invite, password.as_deref())) else {
            return Ok(false);
        };
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        match shared.get(&self.id) {
            Some(Sharing::Guest(guest)) => guest.reinvite(invite),
            // Only by an invite to a project kept online.
            Some(Sharing::Online(member)) if invite.is_online() => member.reinvite(invite),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Tell the others what this person is looking at: the composition open,
    /// the layers selected in it, the playhead's frame, and the pointer over
    /// the Viewer in composition pixels. `properties` and `keys` are the
    /// property rows and the keyframes selected in the Timeline, by whatever
    /// names the frontend matches its own rows with, which the engine passes
    /// on unread. Does nothing when the project is not shared. Latest wins,
    /// so call it as often as any of them changes.
    ///
    /// `changing` is the controls this person is dragging or typing in
    /// right now, by the frontend's own names, passed on unread in the same
    /// way for the others to hold. Pass it empty the moment they stop. In a
    /// project kept online there is no need to call again while it stays
    /// the same: the engine says it again by itself, and the others let go
    /// of it two seconds after this machine falls silent.
    #[frb(sync)]
    #[allow(clippy::too_many_arguments)]
    pub fn share_presence(
        &self,
        comp: Option<CompositionReference>,
        layers: Vec<LayerReference>,
        playhead: Option<i64>,
        cursor_x: Option<f64>,
        cursor_y: Option<f64>,
        properties: Vec<String>,
        keys: Vec<String>,
        changing: Vec<String>,
    ) -> Result<(), BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(sharing) = shared.get(&self.id) else {
            return Ok(());
        };
        let state = self.state()?;
        let doc = {
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let comp = comp.and_then(|c| doc.comp(c.id));
        sharing.set_presence(Presence {
            comp: comp.map(|c| c.id),
            layers: layers.iter().map(|l| l.layer_id).collect(),
            playhead: comp
                .zip(playhead)
                .and_then(|(c, frame)| c.frame_rate.time_of_frame(frame).ok()),
            cursor: cursor_x.zip(cursor_y),
            properties,
            keys,
            changing,
        });
        Ok(())
    }

    /// Every footage transfer in flight, to and from this machine.
    #[frb(sync)]
    pub fn share_transfers(&self) -> Result<Vec<BridgeShareTransfer>, BridgeError> {
        let Some(carrier) = crate::footage::carrier(self.id) else {
            return Ok(Vec::new());
        };
        let state = self.state()?;
        let doc = {
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let name = |item: Option<Uuid>| {
            let item = item.and_then(|id| doc.item(id));
            item.map_or_else(String::new, |item| item.name().to_owned())
        };
        let moving = carrier.moving().into_iter();
        Ok(moving
            .map(|(wanted, done, total, sending)| BridgeShareTransfer {
                name: name(wanted.item()),
                original: matches!(wanted, lumit_share::Wanted::Original { .. }),
                sending,
                done,
                total,
            })
            .collect())
    }

    /// How fetching what an export lacks is getting on.
    #[frb(sync)]
    pub fn share_fetching(&self) -> Result<BridgeShareFetching, BridgeError> {
        use crate::footage::Fetching;
        let fetching = crate::footage::carrier(self.id).map(|carrier| carrier.fetching());
        Ok(match fetching {
            Some(Fetching::Working { done, total }) => BridgeShareFetching::Working { done, total },
            Some(Fetching::Queued { id }) => BridgeShareFetching::Queued { id },
            Some(Fetching::Failed) => BridgeShareFetching::Failed,
            Some(Fetching::Idle) | None => BridgeShareFetching::Idle,
        })
    }

    /// Give up fetching what an export lacks. What has arrived is kept.
    #[frb(sync)]
    pub fn share_fetch_cancel(&self) -> Result<(), BridgeError> {
        if let Some(carrier) = crate::footage::carrier(self.id) {
            carrier.cancel_fetch();
        }
        Ok(())
    }

    /// How the export asked of someone else's machine is getting on.
    #[frb(sync)]
    pub fn share_asking(&self) -> Result<BridgeShareAsking, BridgeError> {
        let carrier = crate::footage::carrier(self.id);
        Ok(carrier.map_or(BridgeShareAsking::Idle, |carrier| asking(carrier.asking())))
    }

    /// Answer an export another person asked this machine to do, by the job
    /// a [`BridgeShareEvent::ExportAsked`] named. Yes puts it in this
    /// machine's export queue and starts it.
    #[frb(sync)]
    pub fn share_answer_export(&self, job: String, yes: bool) -> Result<(), BridgeError> {
        let carrier = crate::footage::carrier(self.id);
        if let (Some(carrier), Ok(job)) = (carrier, job.parse::<Uuid>()) {
            carrier.answer_export(job, yes);
        }
        Ok(())
    }

    /// The conflicts a merge left for this guest to choose between, in the
    /// order [`Self::share_resolve`] indexes them.
    #[frb(sync)]
    pub fn share_conflicts(&self) -> Result<Vec<BridgeShareConflict>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let conflicts = match shared.get(&self.id) {
            Some(Sharing::Guest(guest)) => guest.conflicts(),
            Some(Sharing::Online(member)) => member.conflicts(),
            _ => return Ok(Vec::new()),
        };
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        Ok(conflicts.iter().map(|c| describe(&doc, c)).collect())
    }

    /// Settle the conflict at `index`. `mine` applies this guest's held edits
    /// over the host's version, as one undo step. Otherwise they are dropped
    /// and the host's version stands. Answers how many of the edits no longer
    /// applied.
    #[frb(sync)]
    pub fn share_resolve(&self, index: u32, mine: bool) -> Result<u32, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Guest(guest)) => guest.resolve(index as usize, mine) as u32,
            Some(Sharing::Online(member)) => member.resolve(index as usize, mine) as u32,
            _ => 0,
        })
    }

    /// The person chose not to save this project, which is about to close.
    /// A guest whose host is away then keeps only the edits its file holds,
    /// so the copy opens next time as it was saved. Does nothing for anyone
    /// else.
    #[frb(sync)]
    pub fn share_discard_away(&self) -> Result<(), BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        match shared.get(&self.id) {
            Some(Sharing::Guest(guest)) => guest.discard(),
            Some(Sharing::Online(member)) => member.discard(),
            _ => {}
        }
        Ok(())
    }

    /// The project files holding this machine's own document from just
    /// before a return to this shared project that held back or dropped
    /// any of its edits: a [`BridgeShareEvent::Back`] whose `held` rose or
    /// whose `refused` is not 0. Newest first, each with when it was kept
    /// and what its own return dropped. Everything the person had made by
    /// then is in each, so they are the way back from a merge that went
    /// against them, to be opened as copies. Each is a project of its own,
    /// and opening it joins nothing. Empty when there are none.
    ///
    /// There are two at most. The first return that holds or drops
    /// anything writes one, and it is left alone from then: it alone has
    /// what that return dropped. Each such return after it writes the
    /// other, over the one before. So the list is the latest return and
    /// the earliest the person has not dealt with. A file is written
    /// before its `Back` is said, and they stay, across Lumit closing,
    /// until [`Self::share_return_copy_forget`]. They are in Lumit's own
    /// folder, so each wants saving somewhere else. Footage packed into
    /// the shared project's file is not in them.
    #[frb(sync)]
    pub fn share_return_copies(&self) -> Result<Vec<BridgeShareReturnCopy>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let copies = lumit_share::before_returns(state.store.snapshot().id);
        let copy = |kept: lumit_share::BeforeReturn| BridgeShareReturnCopy {
            path: kept.path.to_string_lossy().into_owned(),
            at: i64::try_from(kept.at).unwrap_or(i64::MAX),
            refused: kept.refused as u32,
            dropped: kept.dropped.into_iter().map(dropped).collect(),
        };
        Ok(copies.into_iter().map(copy).collect())
    }

    /// The person has dismissed what the returns dropped, or has their
    /// copies, so [`Self::share_return_copies`] has nothing more to give.
    /// All of them go.
    #[frb(sync)]
    pub fn share_return_copy_forget(&self) -> Result<(), BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        lumit_share::forget_before_return(state.store.snapshot().id);
        Ok(())
    }

    /// Keep this project online: Lumit's server holds it, sealed so that
    /// the server cannot read it, and puts everyone's edits in order.
    /// Nobody's machine has to be on for the others to work. Others join
    /// with the link this gives, by [`join_shared_project`]. Lumit Pro, for
    /// the person signed in, who is its host.
    ///
    /// `name` is what the others see this person called. `password` is one
    /// everyone has to give as well as holding the link. Empty or `None`
    /// is none. The door [`share_online_door`] names has to be open.
    ///
    /// From then on the project is shared as any other is: people,
    /// presence, conflicts and the events are the same calls. What is not
    /// is in [`Self::share_online_state`].
    ///
    /// Not sync: it waits on the whole document being stored.
    pub fn share_online(
        &self,
        name: String,
        password: Option<String>,
        events: Option<ShareEventStream>,
    ) -> Result<BridgeOnlineStarted, BridgeError> {
        let (store, root, file) = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let root = state.path.as_deref().and_then(Path::parent);
            let root = root.map(Path::to_path_buf);
            (state.store.clone(), root, state.path.clone())
        };
        // Hashed before anything is taken, as in `share`. One that will not
        // hash stops it here, where sharing with none would be a surprise.
        let lock = password
            .filter(|password| !password.is_empty())
            .map(|password| lumit_share::Lock::new(&password))
            .transpose();
        let Ok(lock) = lock else {
            return Ok(BridgeOnlineStarted::Failed);
        };
        let shared = with_sharing(self.id, |_| ()).is_some();
        if shared || STARTING.swap(true, Ordering::Relaxed) {
            return Ok(BridgeOnlineStarted::Failed);
        }
        let sink = Arc::new(events);
        let events = events_for(self.id, store.clone(), sink.clone());
        let member = lumit_share::online::share(store.clone(), &name, lock, root, events);
        let started = match member {
            Ok(member) => {
                let link = member.invite().to_string();
                let sharing = Sharing::Online(member);
                // The host's file is a copy like anyone's from here.
                note_copy(store.snapshot().id, file.as_deref());
                carry(self.id, store, sink, &sharing);
                if let Ok(mut shared) = SHARED.lock() {
                    shared.insert(self.id, sharing);
                }
                BridgeOnlineStarted::Online { link }
            }
            Err(ShareError::Server(code)) => BridgeOnlineStarted::Refused { code },
            Err(ShareError::Unreachable | ShareError::Io(_)) => BridgeOnlineStarted::Unreachable,
            Err(_) => BridgeOnlineStarted::Failed,
        };
        STARTING.store(false, Ordering::Relaxed);
        Ok(started)
    }

    /// What this project is to this machine while it is kept online: whose
    /// it is, whether the server is there and taking edits, and what the
    /// interface needs to make its own calls to the server about it. `None`
    /// when it is not kept online. There straight after opening a copy of
    /// one, however that copy was closed.
    #[frb(sync)]
    pub fn share_online_state(&self) -> Result<Option<BridgeShareOnline>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Online(member)) = shared.get(&self.id) else {
            return Ok(None);
        };
        let (id, access) = member.access();
        Ok(Some(BridgeShareOnline {
            host: member.host(),
            writable: member.writable(),
            away: member.away(),
            id,
            access,
            link: member.invite().to_string(),
            look: member.look(),
            look_link: member.look_invite().to_string(),
            look_on: member.looks(),
            too_large: member.too_large(),
        }))
    }

    /// Which doors this project's sharing goes through. The interface opens
    /// them when a copy is opened again, and again each time the project
    /// says it is away: the engine gives up a connection by itself when it
    /// is given a new invite, finds itself out of step or is turned away by
    /// the server, and a door that shuts with its last connection is then
    /// one it cannot dial. It dials the moment one opens.
    #[frb(sync)]
    pub fn share_doors(&self) -> Result<BridgeShareDoors, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let sharing = shared.get(&self.id);
        Ok(BridgeShareDoors {
            relay: matches!(sharing, Some(Sharing::Guest(guest)) if guest.by_cloud()),
            online: matches!(sharing, Some(Sharing::Online(_))),
        })
    }

    /// For the host of a project kept online: make a new invite to replace
    /// the one handed out, taking the person numbered `person` out first
    /// when one is given. The old invite then finds nothing, and nothing
    /// stored is sealed again.
    ///
    /// This only makes it. The interface sends `body` to Lumit's server as
    /// `PUT /v1/online/<id>/access`, with the account and what
    /// [`Self::share_online_state`] gives as it stands now, and on a yes
    /// calls [`Self::share_online_moved`] with `link`. A server that does
    /// not read the `seat` in `body` is asked `POST /v1/online/<id>/remove`
    /// for that person first. Everyone else here is sent the new invite by
    /// the server. Someone away just then is not, and needs it given.
    ///
    /// `None` when the project is not kept online, or this copy was opened
    /// again and has not found the server yet.
    #[frb(sync)]
    pub fn share_online_move(
        &self,
        person: Option<u32>,
    ) -> Result<Option<BridgeOnlineMove>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Online(member)) = shared.get(&self.id) else {
            return Ok(None);
        };
        let moved = member.move_to(person).ok();
        Ok(moved.map(|(invite, body)| BridgeOnlineMove {
            link: invite.to_string(),
            body,
        }))
    }

    /// For the host of a project kept online: turn the invite that can
    /// only look ([`BridgeShareOnline::look_link`]) on or off.
    ///
    /// This only makes what to say. The interface sends what it answers to
    /// Lumit's server as the body of `PUT /v1/online/<id>/look`, with the
    /// account and what [`Self::share_online_state`] gives, as it sends the
    /// body [`Self::share_online_move`] makes. On a yes the server tells
    /// every machine in the project, this one too, and
    /// [`BridgeShareEvent::LookInvite`] says so here: there is nothing to
    /// call afterwards.
    ///
    /// Turned off, the server forgets that invite, so `look_link` opens
    /// nothing, and everyone who is looking is put out, each told they were
    /// removed. `link` is not changed. Turned on again, the same
    /// `look_link` opens the project again. A new invite
    /// ([`Self::share_online_move`]) keeps it as it is, on or off, and
    /// replaces `look_link`.
    ///
    /// `None` when the project is not kept online, this copy was opened
    /// again and has not found the server yet, or this machine can only
    /// look.
    #[frb(sync)]
    pub fn share_online_look(&self, look_on: bool) -> Result<Option<String>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Online(member)) = shared.get(&self.id) else {
            return Ok(None);
        };
        Ok(member.look_body(look_on).ok())
    }

    /// Lumit's server moved this project to the invite
    /// [`Self::share_online_move`] made. It is the invite handed out from
    /// now on, [`Self::share_link`] gives it, and the project is found by
    /// it. False when `link` is not such an invite or the project is not
    /// kept online.
    #[frb(sync)]
    pub fn share_online_moved(&self, link: String) -> Result<bool, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let invite = link.parse::<Invite>().ok().filter(Invite::is_online);
        match (shared.get(&self.id), invite) {
            (Some(Sharing::Online(member)), Some(invite)) => member.moved(invite),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// How this machine stands to this project while another version of
    /// Lumit keeps it online, as [`BridgeShareEvent::Versions`] last said.
    /// `None` when nothing is in the way: the project is not kept online,
    /// this machine is in it, or this copy has not found the server yet.
    #[frb(sync)]
    pub fn share_online_versions(&self) -> Result<Option<BridgeShareVersions>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Online(member)) = shared.get(&self.id) else {
            return Ok(None);
        };
        Ok(member.versions().map(versions))
    }

    /// For the host, once the person has agreed: move this project forward
    /// from the older Lumit that keeps it online to this one. Asked when
    /// [`Self::share_online_versions`] is [`BridgeShareVersions::Movable`].
    ///
    /// On [`BridgeOnlineForward::Moved`] this copy joins by itself, with
    /// [`BridgeShareEvent::Back`], and so does everyone else who runs this
    /// Lumit. Anyone at work on the older one is cut off and told to
    /// update, and what they had not had answered is kept for when they
    /// have.
    ///
    /// Not sync: it waits on the whole project being fetched and stored.
    pub fn share_online_forward(&self) -> Result<BridgeOnlineForward, BridgeError> {
        // Taken out from under the lock, which the work is too long for.
        let forward = {
            let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
            let Some(Sharing::Online(member)) = shared.get(&self.id) else {
                return Ok(BridgeOnlineForward::Failed);
            };
            member.forward()
        };
        Ok(forwarded(forward()))
    }

    /// Who changed what in a project kept online, newest sitting first: one
    /// entry for each run of one person's edits. It covers everyone's
    /// edits since the project went online, however long ago and whoever
    /// was there to see them, up to the newest 200 sittings. This machine's
    /// own edits are in it once Lumit's server has taken them.
    /// [`BridgeShareEvent::Activity`] says when it has changed.
    ///
    /// Empty when the project is not kept online, and for a copy opened
    /// again until it has found the server.
    #[frb(sync)]
    pub fn share_activity(&self) -> Result<Vec<BridgeShareSitting>, BridgeError> {
        let (mine, list) = {
            let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
            let Some(Sharing::Online(member)) = shared.get(&self.id) else {
                return Ok(Vec::new());
            };
            (member.who(), member.activity())
        };
        let doc = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let seconds = |at: u64| i64::try_from(at).unwrap_or(i64::MAX);
        let comp = |id: Uuid| {
            let comp = doc.comp(id);
            BridgeSittingComp {
                id: id.to_string(),
                name: comp.map_or_else(String::new, |comp| comp.name.clone()),
                comp: comp.map(|comp| CompositionReference::new(self.id, comp.id)),
            }
        };
        let sittings = list.into_iter().rev();
        Ok(sittings
            .map(|sitting| BridgeShareSitting {
                name: sitting.name,
                mine: sitting.who == mine,
                began: seconds(sitting.began),
                ended: seconds(sitting.ended),
                edits: sitting.edits,
                comps: sitting.comps.into_iter().map(comp).collect(),
                is_new: sitting.new,
            })
            .collect())
    }

    /// The notes people have left in a project kept online, newest first,
    /// with the ones written here that are still waiting to be sent before
    /// them. The newest two hundred are kept. [`BridgeShareEvent::Activity`]
    /// says when the list has changed. Empty when the project is not kept
    /// online.
    #[frb(sync)]
    pub fn share_notes(&self) -> Result<Vec<BridgeShareNote>, BridgeError> {
        let (mine, notes) = {
            let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
            let Some(Sharing::Online(member)) = shared.get(&self.id) else {
                return Ok(Vec::new());
            };
            (member.who(), member.notes())
        };
        let doc = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let notes = notes.into_iter().rev();
        Ok(notes
            .map(|note| {
                let comp = note.comp.and_then(|id| doc.comp(id));
                let layer = comp.zip(note.layer).and_then(|(comp, layer)| {
                    let found = comp.layers.iter().find(|one| one.id == layer)?;
                    Some(LayerReference::new(self.id, comp.id, found.id))
                });
                BridgeShareNote {
                    id: note.id.to_string(),
                    name: note.name,
                    mine: note.who == mine,
                    at: i64::try_from(note.at).unwrap_or(i64::MAX),
                    text: note.text,
                    comp: note.comp.map(|id| BridgeSittingComp {
                        id: id.to_string(),
                        name: comp.map_or_else(String::new, |comp| comp.name.clone()),
                        comp: comp.map(|comp| CompositionReference::new(self.id, comp.id)),
                    }),
                    frame: comp
                        .zip(note.time)
                        .map(|(comp, time)| comp.frame_rate.frame_at(time)),
                    layer,
                    is_new: note.new,
                    waiting: note.last == 0,
                }
            })
            .collect())
    }

    /// Leave a note in a project kept online: `text`, with where this
    /// person is as they write it, which is the composition they have
    /// open, the frame their playhead is on and the layer they have chosen,
    /// each when there is one. Everyone in the project gets it, whoever is
    /// away gets it when they are back, and it stays with the project. It
    /// is no part of the document, so undo does not take it away.
    ///
    /// Written while this machine is away it waits, shows in
    /// [`Self::share_notes`] as waiting, and is sent when the server is
    /// found. `text` is cut to five hundred characters.
    ///
    /// False when the project is not kept online, `text` is nothing but
    /// spaces, two hundred notes already wait to be sent, or this machine
    /// can only look.
    #[frb(sync)]
    pub fn share_note(
        &self,
        text: String,
        comp: Option<CompositionReference>,
        frame: Option<i64>,
        layer: Option<LayerReference>,
    ) -> Result<bool, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Online(member)) = shared.get(&self.id) else {
            return Ok(false);
        };
        let doc = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let comp = comp.and_then(|comp| doc.comp(comp.id));
        let time = comp
            .zip(frame)
            .and_then(|(comp, frame)| comp.frame_rate.time_of_frame(frame).ok());
        let layer = layer.map(|layer| layer.layer_id);
        Ok(member.write_note(&text, comp.map(|comp| comp.id), time, layer))
    }

    /// Take a note down from a project kept online, by the `id`
    /// [`Self::share_notes`] gives it. Its writer may, and the host may take
    /// down anyone's. Everyone loses it together, whoever is away loses it
    /// when they are back, and it does not come back.
    /// [`BridgeShareEvent::Activity`] says when the list has changed, and
    /// the list then no longer has it. Asked while this machine is away it
    /// waits to be sent, and the note is gone from the list here at once.
    ///
    /// False when the project is not kept online, there is no such note,
    /// it is someone else's and this is not the host's machine, or this
    /// machine can only look.
    #[frb(sync)]
    pub fn share_note_take_down(&self, id: String) -> Result<bool, BridgeError> {
        let Ok(id) = Uuid::try_parse(&id) else {
            return Ok(false);
        };
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Online(member)) = shared.get(&self.id) else {
            return Ok(false);
        };
        Ok(member.take_note_down(id))
    }

    /// The person has been shown [`Self::share_activity`] and
    /// [`Self::share_notes`], so nothing in them is new to them any more.
    /// The copy remembers how far that was, and what is new the next time
    /// it is opened is what happened after. Does nothing when the project
    /// is not kept online.
    ///
    /// Not sync: it writes that down.
    pub fn share_activity_seen(&self) -> Result<(), BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        if let Some(Sharing::Online(member)) = shared.get(&self.id) {
            member.seen();
        }
        Ok(())
    }

    /// Every footage item of a project kept online, with what Lumit's
    /// server files it under and where it is on this machine. The interface
    /// holds this against what the server has (`GET
    /// /v1/online/<id>/footage`) to see what to give and what to fetch.
    /// Empty when the project is not kept online.
    #[frb(sync)]
    pub fn online_footage(&self) -> Result<Vec<BridgeOnlineFootage>, BridgeError> {
        use crate::footage::Here;
        let online = |sharing: &Sharing| matches!(sharing, Sharing::Online(_));
        if with_sharing(self.id, online) != Some(true) {
            return Ok(Vec::new());
        }
        let doc = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let footage = doc.items.iter().filter_map(|item| match item {
            lumit_core::model::ProjectItem::Footage(footage) => Some(footage),
            _ => None,
        });
        Ok(footage
            .map(|footage| {
                let (here, bytes) = match crate::footage::here(footage) {
                    Here::Original(path) => {
                        let bytes = std::fs::metadata(path).map_or(0, |file| file.len());
                        (BridgeFootageHere::Original, bytes)
                    }
                    Here::StandIn(_) => (BridgeFootageHere::StandIn, 0),
                    Here::Missing => (BridgeFootageHere::Missing, 0),
                };
                BridgeOnlineFootage {
                    item: FootageReference::new(self.id, footage.id),
                    name: footage.name.clone(),
                    key: crate::footage::key(footage),
                    here,
                    used: doc.item_is_used(footage.id),
                    bytes,
                }
            })
            .collect())
    }

    /// Seal the file at `source` into a new file at `sealed`, for Lumit's
    /// server to keep for this project: one of its footage items, as
    /// [`FootageReference::online_give`] answers it. The interface sends
    /// the sealed file and deletes it. It is read and written a piece at a
    /// time, so its size costs no memory.
    ///
    /// `on_progress` hears the fraction done, and [`online_cancel`] stops
    /// it. False when it failed or was stopped, or the project is not kept
    /// online, and then nothing is left at `sealed`.
    ///
    /// Not sync: it is as long as the file is big.
    pub fn online_seal(
        &self,
        source: String,
        sealed: String,
        on_progress: Option<PackProgressStream>,
    ) -> Result<bool, BridgeError> {
        let report = |fraction| {
            if let Some(sink) = &on_progress {
                let _ = sink.add(fraction);
            }
        };
        let (source, sealed) = (Path::new(&source), Path::new(&sealed));
        Ok(crate::footage::online_pass(
            self.id, false, source, sealed, report,
        ))
    }

    /// Open the sealed file at `sealed`, which came from Lumit's server,
    /// into a new file at `out`: [`FootageReference::online_room`] for a
    /// footage item, and then [`FootageReference::online_arrived`] says it
    /// is there. A file that is not whole, was changed, or was sealed for
    /// another project is refused.
    ///
    /// `on_progress` and [`online_cancel`] as [`Self::online_seal`] has
    /// them. False when it was refused, failed or was stopped, and then
    /// nothing is left at `out`.
    ///
    /// Not sync: it is as long as the file is big.
    pub fn online_open(
        &self,
        sealed: String,
        out: String,
        on_progress: Option<PackProgressStream>,
    ) -> Result<bool, BridgeError> {
        let report = |fraction| {
            if let Some(sink) = &on_progress {
                let _ = sink.add(fraction);
            }
        };
        let (sealed, out) = (Path::new(&sealed), Path::new(&out));
        Ok(crate::footage::online_pass(
            self.id, true, sealed, out, report,
        ))
    }
}

/// For the host, once the person has agreed: move the project an invite
/// names forward from the older Lumit that keeps it online to this one.
/// Asked when [`join_shared_project`] answered
/// [`BridgeShareVersions::Movable`], with the invite, name and password it
/// was given, and on [`BridgeOnlineForward::Moved`] that is asked again.
///
/// Not sync: it waits on the whole project being fetched and stored.
pub fn online_forward(
    invite: String,
    name: String,
    password: Option<String>,
) -> Result<BridgeOnlineForward, BridgeError> {
    let invite = invite.parse::<Invite>().ok().filter(Invite::is_online);
    let invite = invite.and_then(|invite| unlocked(invite, password.as_deref()));
    Ok(match invite {
        Some(invite) => forwarded(lumit_share::online::forward(invite, &name)),
        None => BridgeOnlineForward::Failed,
    })
}

/// Join the project an invite names. It opens as a new, unsaved project in
/// place of whatever was open, so saving it writes this person's own copy.
///
/// An invite to a project that is kept online joins that, with nothing
/// different to say here. The door [`share_online_door`] names has to be
/// open, and without one the answer is `Unreachable`.
///
/// `password` is the host's, when it set one. `footage` is the folder this
/// machine keeps the project's footage in. Each item is looked for there by
/// name and then by fingerprint, and what is not found shows as missing, to
/// be relinked like any other.
///
/// Not sync: it waits on the network and on the whole document arriving.
pub fn join_shared_project(
    invite: String,
    name: String,
    password: Option<String>,
    footage: Option<String>,
    on_change_stream: Option<CallbackStream>,
    events: Option<ShareEventStream>,
) -> Result<BridgeJoinOutcome, BridgeError> {
    let Ok(invite) = invite.parse::<Invite>() else {
        return Ok(BridgeJoinOutcome::BadInvite);
    };
    let Some(invite) = unlocked(invite, password.as_deref()) else {
        return Ok(BridgeJoinOutcome::PasswordNeeded);
    };
    let root = footage.filter(|f| !f.trim().is_empty()).map(PathBuf::from);
    // A project kept online has no host to dial. Lumit's server has it.
    let joined = if invite.is_online() {
        lumit_share::online::join(invite, &name, root.clone()).map(|(document, joining)| {
            let start: Start =
                Box::new(|store, events| joining.start(store, events).map(Sharing::Online));
            (document, start)
        })
    } else {
        lumit_share::join(invite, &name, root.clone()).map(|(document, joining)| {
            let start: Start =
                Box::new(|store, events| joining.start(store, events).map(Sharing::Guest));
            (document, start)
        })
    };
    let (document, start) = match joined {
        Ok(joined) => joined,
        Err(e) => {
            return Ok(match e {
                ShareError::Refused(Refusal::Version { host }) => {
                    BridgeJoinOutcome::VersionMismatch { host }
                }
                ShareError::Versions(apart) => BridgeJoinOutcome::Versions {
                    versions: versions(apart),
                },
                ShareError::Refused(Refusal::Full) => BridgeJoinOutcome::Full,
                ShareError::Unsafe => BridgeJoinOutcome::Unsafe,
                ShareError::Unreachable | ShareError::Io(_) | ShareError::Channel(_) => {
                    BridgeJoinOutcome::Unreachable
                }
                _ => BridgeJoinOutcome::Failed,
            })
        }
    };
    let media_root = root.unwrap_or_default();
    crate::packing::begin();
    let Some((project, _missing)) = adopt(document, None, &media_root, on_change_stream, None)?
    else {
        return Ok(BridgeJoinOutcome::Failed);
    };
    let store = {
        let state = project.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.clone()
    };
    let sink = Arc::new(events);
    let events = events_for(project.id, store.clone(), sink.clone());
    let Ok(sharing) = start(store.clone(), events) else {
        return Ok(BridgeJoinOutcome::Failed);
    };
    carry(project.id, store, sink, &sharing);
    SHARED
        .lock()
        .map_err(|_| BridgeError::WriteFailed)?
        .insert(project.id, sharing);
    Ok(BridgeJoinOutcome::Joined { project })
}
