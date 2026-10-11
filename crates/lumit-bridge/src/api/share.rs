//! Shared projects: one person hosts a project and others join it, all
//! editing at once. `lumit-share` does the sharing, and this is the surface
//! the frontend drives it through.

use std::{
    collections::BTreeMap,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use flutter_rust_bridge::frb;
use lumit_core::{Document, DocumentStore};
use lumit_share::{
    Conflict, Ending, Event, Invite, Person, Presence, Reach, Refusal, Relayed, ShareError, Sharing,
};
use uuid::Uuid;

use crate::{
    api::{
        composition::CompositionReference,
        layer::LayerReference,
        project::ProjectReference,
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
    /// applied and are gone.
    Back { held: u32, refused: u32 },
    /// The invite this guest is looking for its host by leads to a different
    /// project. It is still away and still looking.
    Elsewhere,
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
}

pub type ShareEventStream = StreamSink<BridgeShareEvent>;

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
            Event::Back { held, refused } => BridgeShareEvent::Back {
                held: held as u32,
                refused: refused as u32,
            },
            Event::Ended(reason) => BridgeShareEvent::Ended {
                reason: ending(reason),
            },
            Event::Reach(now) => BridgeShareEvent::Reach { reach: reach(now) },
            Event::Relayed(now) => BridgeShareEvent::Relayed {
                relayed: relayed(now),
            },
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
        None => None,
    }
}

/// `project`, whose document is `document`, was saved. A host lets go of
/// the first `mark` edits it keeps, which are in the file now. Saved while
/// nobody hosts it, whatever a host kept before goes: the file has moved on
/// from the document those edits were made on.
#[frb(ignore)]
pub(crate) fn saved(project: Uuid, document: Uuid, mark: Option<usize>) {
    let shared = SHARED.lock().ok();
    let hosted = shared.as_ref().and_then(|shared| shared.get(&project));
    match (hosted, mark) {
        (Some(Sharing::Host(host)), Some(mark)) => host.saved(mark),
        // Sharing started while the file was being written. Kept as it is.
        (Some(Sharing::Host(_)), None) => {}
        // A guest's edits stay kept, and the mark is what closing without
        // saving goes back to.
        (Some(Sharing::Guest(guest)), Some(mark)) => guest.saved(mark),
        _ => lumit_share::forget(document),
    }
}

/// Carry on as the guest `project`'s file was left as: its host away, and
/// edits in it waiting to be merged.
#[frb(ignore)]
pub(crate) fn resume(
    project: &ProjectReference,
    resuming: lumit_share::Resuming,
    events: Option<ShareEventStream>,
) -> Result<(), BridgeError> {
    let store = {
        let state = project.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.clone()
    };
    let sink = Arc::new(events);
    let events = events_for(project.id, store.clone(), sink.clone());
    if let Ok(guest) = resuming.start(store.clone(), events) {
        let sharing = Sharing::Guest(guest);
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
    /// machine is not hosting the project.
    #[frb(sync)]
    pub fn share_link(&self, address: Option<String>) -> Result<Option<String>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Host(host)) => Some(host.invite_anywhere(address.as_deref()).to_string()),
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
    /// project.
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
        let Some(Sharing::Guest(guest)) = shared.get(&self.id) else {
            return Ok(false);
        };
        guest.reinvite(invite);
        Ok(true)
    }

    /// Tell the others what this person is looking at: the composition open,
    /// the layers selected in it, the playhead's frame, and the pointer over
    /// the Viewer in composition pixels. `properties` and `keys` are the
    /// property rows and the keyframes selected in the Timeline, by whatever
    /// names the frontend matches its own rows with, which the engine passes
    /// on unread. Does nothing when the project is not shared. Latest wins,
    /// so call it as often as any of them changes.
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
        let Some(Sharing::Guest(guest)) = shared.get(&self.id) else {
            return Ok(Vec::new());
        };
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let conflicts = guest.conflicts();
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
        if let Some(Sharing::Guest(guest)) = shared.get(&self.id) {
            guest.discard();
        }
        Ok(())
    }
}

/// Join the project an invite names. It opens as a new, unsaved project in
/// place of whatever was open, so saving it writes this person's own copy.
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
    let (document, joining) = match lumit_share::join(invite, &name, root.clone()) {
        Ok(joined) => joined,
        Err(e) => {
            return Ok(match e {
                ShareError::Refused(Refusal::Version { host }) => {
                    BridgeJoinOutcome::VersionMismatch { host }
                }
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
    let (project, _missing) = adopt(document, None, &media_root, on_change_stream, None)?;
    let store = {
        let state = project.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.clone()
    };
    let sink = Arc::new(events);
    let events = events_for(project.id, store.clone(), sink.clone());
    let Ok(guest) = joining.start(store.clone(), events) else {
        return Ok(BridgeJoinOutcome::Failed);
    };
    let sharing = Sharing::Guest(guest);
    carry(project.id, store, sink, &sharing);
    SHARED
        .lock()
        .map_err(|_| BridgeError::WriteFailed)?
        .insert(project.id, sharing);
    Ok(BridgeJoinOutcome::Joined { project })
}
