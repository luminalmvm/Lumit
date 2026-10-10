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
    Conflict, Ending, Event, Invite, Person, Presence, Reach, Refusal, ShareError, Sharing,
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
}

pub type ShareEventStream = StreamSink<BridgeShareEvent>;

/// How starting to share went.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeShareStarted {
    /// Sharing, and listening on `port`. `key` is the invite's secret, to
    /// hand back to [`ProjectReference::share`] when this project is shared
    /// again, so the invite people already hold still finds it. `restored`
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
    /// Nobody answered at that address, or the invite was not accepted.
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

/// This machine's address on its network, to offer in an invite. A guess the
/// person can type over, for a VPN or a forwarded port.
#[frb(sync)]
pub fn share_local_address() -> String {
    lumit_share::local_address()
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
    } = person.presence;
    let comp = comp.and_then(|id| doc.comp(id));
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
    }
}

/// Turn a shared project's events into the frontend's and send them down
/// `sink`, if anyone is listening.
#[frb(ignore)]
fn events_for(
    project: Uuid,
    store: Arc<DocumentStore>,
    sink: Option<ShareEventStream>,
) -> lumit_share::Events {
    Arc::new(move |event| {
        let event = match event {
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
        };
        if let Some(sink) = &sink {
            _ = sink.add(event);
        }
    })
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
    let events = events_for(project.id, store.clone(), events);
    if let Ok(guest) = resuming.start(store, events) {
        SHARED
            .lock()
            .map_err(|_| BridgeError::WriteFailed)?
            .insert(project.id, Sharing::Guest(guest));
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
    /// Share this project from this machine. Others join with the text
    /// [`Self::share_invite`] gives.
    ///
    /// `name` is what the others see this person called. Port 0 takes any
    /// free one. `key` is what the last [`BridgeShareStarted::Sharing`] for
    /// this project gave, or `None` for a new invite. `outside` asks this
    /// network's router to send the port here, so people outside the network
    /// can join without a VPN. It answers later, as a
    /// [`BridgeShareEvent::Reach`]. `events` is optional the way a project's
    /// change stream is, and for the same reason: nothing about sharing
    /// depends on someone watching.
    #[frb(sync)]
    pub fn share(
        &self,
        name: String,
        port: u16,
        key: Option<String>,
        outside: bool,
        events: Option<ShareEventStream>,
    ) -> Result<BridgeShareStarted, BridgeError> {
        let (store, root) = {
            let state = self.state()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let root = state.path.as_deref().and_then(Path::parent);
            (state.store.clone(), root.map(Path::to_path_buf))
        };
        let mut shared = SHARED.lock().map_err(|_| BridgeError::WriteFailed)?;
        if shared.contains_key(&self.id) {
            return Ok(BridgeShareStarted::Failed);
        }
        let events = events_for(self.id, store.clone(), events);
        // A key that does not read as one is as good as none.
        let key = key.and_then(|key| lumit_share::key_from(&key));
        Ok(
            match lumit_share::host(store, &name, LISTEN, port, key, root, events) {
                Ok(host) => {
                    let (port, key) = (host.port(), lumit_share::key_text(&host.key()));
                    let restored = host.restored() as u32;
                    // A test never asks the machine's real router anything.
                    if outside && !cfg!(test) {
                        host.reach_out();
                    }
                    shared.insert(self.id, Sharing::Host(host));
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

    /// The invite for a host reached at `address`, to send to whoever is
    /// joining. `None` when this machine is not hosting the project.
    #[frb(sync)]
    pub fn share_invite(&self, address: String) -> Result<Option<String>, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match shared.get(&self.id) {
            Some(Sharing::Host(host)) => Some(host.invite(address.trim()).to_string()),
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
        if let Some(sharing) = sharing {
            sharing.stop();
        }
        Ok(())
    }

    /// Take a guest out of the project this machine hosts, by the id the
    /// people list gives them. Their Lumit is told and stops coming back.
    /// The invite is replaced, so the one they hold stops working. Everyone
    /// still here is sent the new one, and [`Self::share_invite`] gives it,
    /// with the key to hand to [`Self::share`] next time.
    #[frb(sync)]
    pub fn share_remove(&self, person: u32) -> Result<(), BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        if let Some(Sharing::Host(host)) = shared.get(&self.id) {
            host.remove(person);
        }
        Ok(())
    }

    /// Give a guest that has lost its host a new invite to look for it by,
    /// for a host that has moved or made a new one. False when the text is
    /// not an invite or this machine is not a guest of the project.
    #[frb(sync)]
    pub fn share_reinvite(&self, invite: String) -> Result<bool, BridgeError> {
        let shared = SHARED.lock().map_err(|_| BridgeError::ReadFailed)?;
        let Some(Sharing::Guest(guest)) = shared.get(&self.id) else {
            return Ok(false);
        };
        let Ok(invite) = invite.parse::<Invite>() else {
            return Ok(false);
        };
        guest.reinvite(invite);
        Ok(true)
    }

    /// Tell the others what this person is looking at: the composition open,
    /// the layers selected in it, the playhead's frame, and the pointer over
    /// the Viewer in composition pixels. Does nothing when the project is not
    /// shared. Latest wins, so call it as often as any of them changes.
    #[frb(sync)]
    pub fn share_presence(
        &self,
        comp: Option<CompositionReference>,
        layers: Vec<LayerReference>,
        playhead: Option<i64>,
        cursor_x: Option<f64>,
        cursor_y: Option<f64>,
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
        });
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
/// `footage` is the folder this machine keeps the project's footage in. Each
/// item is looked for there by name and then by fingerprint, and what is not
/// found shows as missing, to be relinked like any other.
///
/// Not sync: it waits on the network and on the whole document arriving.
pub fn join_shared_project(
    invite: String,
    name: String,
    footage: Option<String>,
    on_change_stream: Option<CallbackStream>,
    events: Option<ShareEventStream>,
) -> Result<BridgeJoinOutcome, BridgeError> {
    let Ok(invite) = invite.parse::<Invite>() else {
        return Ok(BridgeJoinOutcome::BadInvite);
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
    let events = events_for(project.id, store.clone(), events);
    let Ok(guest) = joining.start(store, events) else {
        return Ok(BridgeJoinOutcome::Failed);
    };
    SHARED
        .lock()
        .map_err(|_| BridgeError::WriteFailed)?
        .insert(project.id, Sharing::Guest(guest));
    Ok(BridgeJoinOutcome::Joined { project })
}
