//! The host's end of a shared project: it listens, seats guests, and sends
//! every edit to everyone in the order its store applied them.
//!
//! One thread accepts. Each guest has a reader, which is the thread that
//! greeted it, and a writer.

use crate::bulk::{self, Footage, Held, Limits, Wanted};
use crate::invite::locked_key;
use crate::kept::{HostLog, Pair};
use crate::local::{place, sane};
use crate::reach::{self, Closed};
use crate::wire::{self, decode, encode, Message, Names, Out, Receiver, Sender};
use crate::{
    dialled, local_address, room, Event, Events, Invite, Person, Presence, Reach, Refusal, Relayed,
    ShareError, MAX_PEOPLE,
};
use lumit_core::store::{Moved, RemoteTag};
use lumit_core::{Document, DocumentStore};
use parking_lot::Mutex;
use std::net::{IpAddr, Ipv6Addr, Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver as Done, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Both ends have to run this, edits being the wire format.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How many connections may be part-way through saying hello. Anyone can
/// connect before proving they hold the invite, so this is what a flood of
/// strangers is held to.
const GREETING_ROOM: usize = 8;

/// How often the accepting thread looks for a caller, tidies up, and sends on
/// what the host is looking at.
const BEAT: Duration = Duration::from_millis(50);

/// How many guests the host remembers the last edit of.
const HEARD: usize = 4 * MAX_PEOPLE;

/// The way to one seated guest.
struct Link {
    id: u32,
    /// What its guest said hello with.
    token: u64,
    out: SyncSender<Out>,
    socket: TcpStream,
}

/// Who is here. Bounded by [`MAX_PEOPLE`].
struct Seats {
    /// The invite's secret. Replaced when a guest is removed, so it is read
    /// under the lock a caller is seated under.
    key: [u8; 32],
    /// What the host's password comes to, if it set one. It outlives the
    /// invite's secret, so the same password opens the next invite too.
    lock: Option<[u8; 32]>,
    people: Vec<Person>,
    links: Vec<Link>,
    /// The last edit taken from each guest, by the token it says hello with,
    /// to tell one that comes back. The newest [`HEARD`] guests, oldest first.
    heard: Vec<(u64, u64)>,
    next: u32,
    /// Someone came or went and the others have not been told yet.
    changed: bool,
}

impl Seats {
    /// The key the channel is opened with: the invite's secret, and the
    /// password's with it when there is one. What a guest that is in holds.
    fn channel(&self) -> [u8; 32] {
        self.lock
            .map_or(self.key, |lock| locked_key(&self.key, &lock))
    }
}

struct Hub {
    store: Arc<DocumentStore>,
    port: u16,
    /// The folder the host's own footage is found under.
    root: Option<PathBuf>,
    events: Events,
    stop: AtomicBool,
    greeting: AtomicUsize,
    seats: Mutex<Seats>,
    /// What the host is looking at, if the guests have not been told yet.
    looking: Mutex<Option<Presence>>,
    names: Names,
    /// Every edit since the project was last saved, kept on disk. Taken in
    /// the tap, so nothing that holds it calls the store.
    log: Mutex<Option<HostLog>>,
    /// Whether people outside this network can get in.
    reach: Mutex<Reach>,
    /// The relay this host keeps a room at, if it was given one, and how
    /// that is going.
    relay: Mutex<Option<(String, Relayed)>>,
    /// The footage this host sends, takes and passes on, once it has been
    /// given some to carry.
    bulk: Mutex<Option<Arc<bulk::Hub>>>,
}

/// A project being shared from this machine.
///
/// [`Self::stop`] ends it for everyone. A drop only lets go: the guests are
/// not told it is over, so they keep what they do next and bring it back
/// when the same project is shared again with the same key.
pub struct Host {
    hub: Arc<Hub>,
    accepting: Mutex<Option<JoinHandle<()>>>,
    /// Hears when the thread that keeps the router's port open has closed
    /// it and gone.
    reaching: Mutex<Option<Done<()>>>,
    restored: usize,
}

/// How long stopping waits for the router to be told to close the port. A
/// router that takes longer is left to let the port lapse by itself.
const CLOSING: Duration = Duration::from_millis(1500);

/// Put back the edits a host made or was sent after its last save, which the
/// document it has opened again does not hold. Its guests were working on a
/// document with them in. Each is landed, so one the document already has
/// changes nothing. Answers how many changed something.
fn restore(store: &DocumentStore, lost: &[Pair]) -> usize {
    if lost.is_empty() {
        return 0;
    }
    // Shared for the length of this, which is what lets the store take an
    // edit that is nobody's undo step.
    store.share(false, Arc::new(|_| {}));
    let tag = RemoteTag { peer: 0, id: 0 };
    let changed = |(op, was): &&Pair| {
        let before = store.snapshot();
        store.commit_remote(op, Some(was), tag).is_ok() && *before != *store.snapshot()
    };
    lost.iter().filter(changed).count()
}

/// Listen at `address`. Every address, `0.0.0.0`, means IPv6 as well where
/// the system keeps the two apart, as Windows does. Where one listener takes
/// both, the second is refused and not needed.
fn listen(address: IpAddr, port: u16) -> std::io::Result<(Vec<TcpListener>, u16)> {
    let first = TcpListener::bind((address, port))?;
    let port = first.local_addr()?.port();
    let second = address
        .is_unspecified()
        .then(|| TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)).ok());
    let listeners: Vec<_> = [Some(first), second.flatten()]
        .into_iter()
        .flatten()
        .collect();
    for listener in &listeners {
        listener.set_nonblocking(true)?;
    }
    Ok((listeners, port))
}

/// Start sharing `store`'s project as its host, listening at `address` on
/// `port`, or on any free port for 0. The address is `0.0.0.0` to be reached
/// from other machines.
///
/// `key` is the secret of an invite handed out before, for a host picking up
/// where it left off: guests who were here still hold it, and find their way
/// back by it. `None` makes a new one. `lock` is what [`crate::lock_of`]
/// made of a password every guest then has to give, or `None` for none.
#[allow(clippy::too_many_arguments)]
pub fn host(
    store: Arc<DocumentStore>,
    name: &str,
    address: IpAddr,
    port: u16,
    key: Option<[u8; 32]>,
    lock: Option<[u8; 32]>,
    root: Option<PathBuf>,
    events: Events,
) -> Result<Host, ShareError> {
    let (listeners, port) = listen(address, port)?;
    // A new invite is one nobody is coming back by.
    let fresh = key.is_none();
    let key = match key {
        Some(key) => key,
        None => {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).map_err(|e| ShareError::NoRandomness(e.to_string()))?;
            key
        }
    };
    let (log, lost) = HostLog::open(store.snapshot().id, fresh).unzip();
    let restored = restore(&store, &lost.unwrap_or_default());
    let hub = Arc::new(Hub {
        store,
        port,
        root,
        events,
        stop: AtomicBool::new(false),
        greeting: AtomicUsize::new(0),
        seats: Mutex::new(Seats {
            key,
            lock,
            people: vec![Person {
                id: 0,
                name: name.chars().take(64).collect(),
                colour: 1,
                presence: Presence::default(),
            }],
            links: Vec::new(),
            heard: Vec::new(),
            next: 1,
            changed: false,
        }),
        looking: Mutex::new(None),
        names: Names::default(),
        log: Mutex::new(log),
        reach: Mutex::new(Reach::Off),
        relay: Mutex::new(None),
        bulk: Mutex::new(None),
    });
    // Weak, or the store would hold the hub that holds the store.
    let tapped = Arc::downgrade(&hub);
    hub.store.share(
        false,
        Arc::new(move |moved| {
            if let Some(hub) = tapped.upgrade() {
                hub.moved(moved);
            }
        }),
    );
    let accepting = hub.clone();
    let spawned = thread::Builder::new()
        .name("lumit-share-accept".into())
        .spawn(move || accepting.accept(&listeners));
    match spawned {
        Ok(thread) => Ok(Host {
            hub,
            accepting: Mutex::new(Some(thread)),
            reaching: Mutex::new(None),
            restored,
        }),
        Err(e) => {
            hub.store.unshare();
            Err(e.into())
        }
    }
}

impl Host {
    /// `address` with the port this host listens on.
    fn at(&self, address: &str) -> String {
        let address = address.trim();
        // An IPv6 address has colons of its own, so it goes in brackets.
        let bare = address.contains(':') && !address.starts_with('[');
        let (open, close) = if bare { ("[", "]") } else { ("", "") };
        format!("{open}{address}{close}:{}", self.hub.port)
    }

    /// The invite to hand out, for a host reached at `address` and nowhere
    /// else.
    #[must_use]
    pub fn invite(&self, address: &str) -> Invite {
        Invite {
            addresses: vec![self.at(address)],
            relays: Vec::new(),
            key: self.key(),
            locked: self.lock().is_some(),
        }
    }

    /// The invite to hand out when nobody has said where the guest is: every
    /// way to this host it knows of, for the guest to try all at once.
    /// `typed` goes first, for an address only the person knows, such as a
    /// VPN's. Then the router's address on the internet once it has opened
    /// the port, this machine's own there over IPv6, and its address on its
    /// own network.
    #[must_use]
    pub fn invite_anywhere(&self, typed: Option<&str>) -> Invite {
        let outside = match self.reach() {
            Reach::Open { address } => Some(address),
            _ => None,
        };
        let known = [outside, crate::global_address(), Some(local_address())];
        let typed = typed.map(str::trim).filter(|typed| !typed.is_empty());
        let mut addresses: Vec<String> = Vec::new();
        for address in typed
            .into_iter()
            .chain(known.iter().flatten().map(String::as_str))
        {
            let address = self.at(address);
            if !addresses.contains(&address) {
                addresses.push(address);
            }
        }
        // A relay that is not answering just now may be by the time the
        // guest tries it.
        let relay = self.hub.relay.lock().clone();
        Invite {
            addresses,
            relays: relay.into_iter().map(|(relay, _)| relay).collect(),
            key: self.key(),
            locked: self.lock().is_some(),
        }
    }

    /// The port it is listening on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.hub.port
    }

    /// The invite's secret, to share the same project by again later.
    #[must_use]
    pub fn key(&self) -> [u8; 32] {
        self.hub.seats.lock().key
    }

    /// What this host's password comes to, if it set one, to share the same
    /// project by again later without asking for the password again.
    #[must_use]
    pub fn lock(&self) -> Option<[u8; 32]> {
        self.hub.seats.lock().lock
    }

    /// How many edits made since the project was last saved were put back
    /// when sharing started, for a host that had closed without saving.
    #[must_use]
    pub fn restored(&self) -> usize {
        self.restored
    }

    /// What a save writes: the document, the store's revision at it, and how
    /// many kept edits are in it, to hand to [`Self::saved`]. Taken in one
    /// moment, so an edit arriving as the project is saved is either in the
    /// file or still kept, and never neither.
    #[must_use]
    pub fn saving(&self) -> (Arc<Document>, u64, usize) {
        let store = &self.hub.store;
        store.frozen(|document| {
            let mark = self.hub.log.lock().as_ref().map_or(0, HostLog::mark);
            (document, store.revision(), mark)
        })
    }

    /// The project was saved with the first `mark` kept edits in it, so they
    /// need keeping no longer.
    pub fn saved(&self, mark: usize) {
        if let Some(log) = self.hub.log.lock().as_mut() {
            log.saved(mark);
        }
    }

    #[must_use]
    pub fn people(&self) -> Vec<Person> {
        self.hub.seats.lock().people.clone()
    }

    /// Ask this network's router to send the port here, so people outside
    /// the network can join without a VPN. Answers at once. What comes of it
    /// is an [`Event::Reach`], and [`Self::reach`] from then on. The port is
    /// closed again when sharing stops.
    pub fn reach_out(&self) {
        let mut reaching = self.reaching.lock();
        if reaching.is_some() {
            return;
        }
        *self.hub.reach.lock() = Reach::Asking;
        let (done, gone) = sync_channel(1);
        let hub = self.hub.clone();
        let spawned = thread::Builder::new()
            .name("lumit-share-reach".into())
            .spawn(move || {
                hub.reach();
                let _ = done.try_send(());
            });
        match spawned {
            Ok(_) => *reaching = Some(gone),
            Err(_) => *self.hub.reach.lock() = Reach::Refused,
        }
    }

    /// Whether people outside this network can get in.
    #[must_use]
    pub fn reach(&self) -> Reach {
        self.hub.reach.lock().clone()
    }

    /// Keep a room at the relay at `relay`, a `host:port`, so a guest that
    /// no address of this machine lets in can still join. Answers at once.
    /// What comes of it is an [`Event::Relayed`], and [`Self::relayed`] from
    /// then on. An invite made after this names the relay. The room goes
    /// when sharing stops.
    pub fn relay_through(&self, relay: &str) {
        let relay = relay.trim().to_owned();
        {
            let mut asked = self.hub.relay.lock();
            if relay.is_empty() || asked.is_some() {
                return;
            }
            *asked = Some((relay.clone(), Relayed::Asking));
        }
        let hub = self.hub.clone();
        let spawned = thread::Builder::new()
            .name("lumit-share-relay".into())
            .spawn(move || hub.relayed(&relay));
        if spawned.is_err() {
            *self.hub.relay.lock() = None;
        }
    }

    /// Whether this host has a room at a relay.
    #[must_use]
    pub fn relayed(&self) -> Relayed {
        let relay = self.hub.relay.lock();
        relay.as_ref().map_or(Relayed::Off, |(_, relayed)| *relayed)
    }

    /// Send and take footage. See [`crate::Sharing::carry_footage`].
    pub fn carry_footage(&self, footage: Arc<dyn Footage>, limits: Arc<Limits>) {
        let mut bulk = self.hub.bulk.lock();
        if bulk.is_none() && !self.hub.stop.load(Ordering::Relaxed) {
            *bulk = Some(bulk::Hub::new(footage, limits, true));
        }
    }

    /// Which footage this machine has the original of, told to everyone.
    pub fn set_holds(&self, items: Vec<Held>) {
        self.hub.hold(0, items);
    }

    /// Who has the original of `item`.
    #[must_use]
    pub fn holders(&self, item: Uuid) -> Vec<(u32, u64)> {
        let bulk = self.hub.bulk.lock().clone();
        bulk.map_or_else(Vec::new, |bulk| bulk.holders(item))
    }

    /// Ask whichever guest has it for `wanted`.
    pub fn want(&self, wanted: Wanted) {
        let bulk = self.hub.bulk.lock().clone();
        if let Some(bulk) = bulk {
            bulk.want(wanted);
        }
    }

    /// Say `body` to the guest numbered `to`.
    pub fn note(&self, to: u32, body: serde_json::Value) {
        self.hub.pass(0, to, body);
    }

    /// Latest wins: the accepting thread sends it on its next beat, so a
    /// playhead that moves every frame is not a message every frame.
    pub fn set_presence(&self, presence: Presence) {
        *self.hub.looking.lock() = Some(presence);
    }

    /// Take one guest out of the project. They are told, and their Lumit
    /// stops trying to come back. The invite is replaced, so the one they
    /// hold lets nobody in, and everyone still here is sent the new one. A
    /// guest who has lost the host just then is not, and needs it given.
    pub fn remove(&self, id: u32) {
        let mut seats = self.hub.seats.lock();
        let Some(at) = seats.links.iter().position(|link| link.id == id) else {
            return;
        };
        let gone = seats.links.remove(at);
        self.hub.dismiss(&gone, &Message::Removed);
        seats.people.retain(|p| p.id != id);
        seats.changed = true;
        // With no randomness for a new secret the old invite stands.
        let mut key = [0u8; 32];
        if getrandom::fill(&mut key).is_err() {
            return;
        }
        seats.key = key;
        // The guests still here hold the key the channel opens with, which
        // has the password in it already.
        let key = seats.channel();
        if let Ok(bytes) = encode(&Message::Invite { key }, &self.hub.names) {
            Hub::send_each(&mut seats, |_| bytes.clone());
        }
    }

    /// Stop sharing: tell every guest it is over, and let go of the port.
    /// Nobody is coming back, so the edits kept for that go too.
    pub fn stop(&self) {
        self.end(Some(&Message::Closed));
        if let Some(log) = self.hub.log.lock().take() {
            log.remove();
        }
    }

    /// Let go of the store, the guests and the port, saying `last` to each
    /// guest first if there is something to say.
    fn end(&self, last: Option<&Message>) {
        if self.hub.stop.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Some(bulk) = self.hub.bulk.lock().take() {
            bulk.stop();
        }
        self.hub.store.unshare();
        {
            let mut seats = self.hub.seats.lock();
            for link in seats.links.drain(..) {
                match last {
                    Some(last) => self.hub.dismiss(&link, last),
                    None => {
                        let _ = link.socket.shutdown(Shutdown::Both);
                    }
                }
            }
            seats.people.truncate(1);
        }
        // The port is free once the accepting thread has gone, which is
        // within a beat. Waited for, so sharing again straight away finds it.
        if let Some(thread) = self.accepting.lock().take() {
            let _ = thread.join();
        }
        // And the router is told to close its port, which is waited for a
        // moment so that Lumit closing does not leave it open.
        if let Some(gone) = self.reaching.lock().take() {
            let _ = gone.recv_timeout(CLOSING);
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.end(None);
    }
}

impl Hub {
    /// The reach thread: have the router open the port, keep it open for as
    /// long as the project is shared, and close it after.
    fn reach(&self) {
        let said = |reach: Reach| {
            self.reach.lock().clone_from(&reach);
            (self.events)(Event::Reach(reach));
        };
        let mapping = match reach::open(self.port, &self.stop) {
            Ok((mapping, address)) => {
                said(Reach::Open {
                    address: address.to_string(),
                });
                mapping
            }
            Err(Closed::Refused) => return said(Reach::Refused),
            Err(Closed::Behind) => return said(Reach::Behind),
        };
        let mut asked = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            thread::sleep(BEAT);
            if asked.elapsed() < reach::RENEW {
                continue;
            }
            asked = Instant::now();
            if !mapping.renew() {
                said(Reach::Refused);
            }
        }
        mapping.close();
    }

    /// The relay thread: keep a room at `relay` for as long as the project
    /// is shared, and take each guest that comes to it. The room is named
    /// after the invite's secret, so when that is replaced the room is too.
    fn relayed(self: &Arc<Self>, relay: &str) {
        let said = |relayed: Relayed| {
            let mut now = self.relay.lock();
            let changed = now.as_ref().is_some_and(|(_, was)| *was != relayed);
            if let Some((_, was)) = now.as_mut() {
                *was = relayed;
            }
            drop(now);
            if changed {
                (self.events)(Event::Relayed(relayed));
            }
        };
        while !self.stop.load(Ordering::Relaxed) {
            let key = self.seats.lock().channel();
            let name = room(&key);
            let current =
                || !self.stop.load(Ordering::Relaxed) && self.seats.lock().channel() == key;
            // Lumit's own relay with no door to it open is as good as one
            // that does not answer.
            let at = dialled(relay);
            match at
                .as_deref()
                .map(|at| (at, lumit_relay::Room::open(at, &name)))
            {
                Some((at, Ok(mut kept))) => {
                    said(Relayed::Open);
                    while current() {
                        match kept.guest(BEAT) {
                            Ok(Some(guest)) => self.take(at, &name, guest),
                            Ok(None) => {}
                            Err(_) => break,
                        }
                    }
                }
                _ => said(Relayed::Unreachable),
            }
            // Asked again before long, and at once for a new secret.
            let asked = Instant::now();
            while current() && asked.elapsed() < lumit_relay::PING {
                thread::sleep(BEAT);
            }
        }
    }

    /// Take a guest waiting at the relay and seat it like any other caller.
    fn take(self: &Arc<Self>, relay: &str, name: &str, guest: u64) {
        if self.greeting.fetch_add(1, Ordering::Relaxed) >= GREETING_ROOM {
            self.greeting.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        let (hub, relay, name) = (self.clone(), relay.to_owned(), name.to_owned());
        let spawned = thread::Builder::new()
            .name("lumit-share-guest".into())
            .spawn(
                move || match lumit_relay::take(relay.as_str(), &name, guest) {
                    Ok(socket) => {
                        let _ = hub.seat(socket);
                    }
                    Err(_) => {
                        hub.greeting.fetch_sub(1, Ordering::Relaxed);
                    }
                },
            );
        if spawned.is_err() {
            self.greeting.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Note which footage `peer` has the original of, and tell everyone.
    fn hold(&self, peer: u32, items: Vec<Held>) {
        let Some(bulk) = self.bulk.lock().clone() else {
            return;
        };
        bulk.holds(peer, items.clone());
        if let Ok(bytes) = encode(&Message::Holds { peer, items }, &self.names) {
            Self::send_each(&mut self.seats.lock(), |_| bytes.clone());
        }
    }

    /// Pass a note from `from` on to `to`, which is this machine for 0.
    fn pass(&self, from: u32, to: u32, body: serde_json::Value) {
        if to == 0 {
            return (self.events)(Event::Note { from, body });
        }
        let Ok(bytes) = encode(&Message::Note { to, from, body }, &self.names) else {
            return;
        };
        let seats = self.seats.lock();
        if let Some(link) = seats.links.iter().find(|link| link.id == to) {
            let _ = link.out.try_send(Out::Bytes(bytes));
        }
    }

    /// A guest's second connection, for footage: run it until it drops. Only
    /// for a guest that is seated, which `token` is how it is known by.
    fn carry(&self, token: u64, socket: TcpStream, sender: Sender, receiver: Receiver) {
        let peer = {
            let seats = self.seats.lock();
            let seated = seats.links.iter().find(|link| link.token == token);
            seated.filter(|_| token != 0).map(|link| link.id)
        };
        let bulk = self.bulk.lock().clone();
        if let (Some(peer), Some(bulk)) = (peer, bulk) {
            bulk.link(peer, socket, sender, receiver);
        }
    }

    /// Say one last thing to a guest and close its connection behind it.
    fn dismiss(&self, link: &Link, last: &Message) {
        if let Ok(bytes) = encode(last, &self.names) {
            let _ = link.out.try_send(Out::Bytes(bytes));
        }
        if link.out.try_send(Out::Close).is_err() {
            let _ = link.socket.shutdown(Shutdown::Both);
        }
    }

    /// Queue for every guest what `bytes` gives for it. A guest whose queue
    /// is full has fallen too far behind to catch up and is dropped. It
    /// reconnects and is sent the document afresh.
    fn send_each(seats: &mut Seats, bytes: impl Fn(u32) -> Arc<[u8]>) {
        let before = seats.links.len();
        seats.links.retain(|link| {
            if link.out.try_send(Out::Bytes(bytes(link.id))).is_ok() {
                return true;
            }
            let _ = link.socket.shutdown(Shutdown::Both);
            false
        });
        if seats.links.len() != before {
            let Seats { people, links, .. } = seats;
            people.retain(|p| p.id == 0 || links.iter().any(|l| l.id == p.id));
            seats.changed = true;
        }
    }

    /// The store's tap: an edit has been applied here, so everyone is sent it.
    /// Its author is only told it went in, unless it went in differently from
    /// how they sent it. Runs under the store's journal lock, which is what
    /// keeps the order.
    fn moved(&self, moved: Moved<'_>) {
        let (peer, id, op, was, as_sent) = match moved {
            Moved::Local { op, was, .. } => (0, 0, op, was, false),
            Moved::Remote {
                tag,
                op,
                was,
                as_sent,
            } => (tag.peer, tag.id, op, was, as_sent),
        };
        if let Some(log) = self.log.lock().as_mut() {
            log.append(op, was);
        }
        let op = op.clone();
        let Ok(applied) = encode(&Message::Applied { peer, id, op }, &self.names) else {
            return;
        };
        let accepted = as_sent
            .then(|| encode(&Message::Accepted { id }, &self.names).ok())
            .flatten();
        Self::send_each(&mut self.seats.lock(), |link| match &accepted {
            Some(accepted) if link == peer => accepted.clone(),
            _ => applied.clone(),
        });
    }

    fn accept(self: &Arc<Self>, listeners: &[TcpListener]) {
        while !self.stop.load(Ordering::Relaxed) {
            let callers = listeners.iter().filter_map(|l| l.accept().ok());
            let mut quiet = true;
            for (socket, _) in callers {
                quiet = false;
                if self.greeting.fetch_add(1, Ordering::Relaxed) >= GREETING_ROOM {
                    self.greeting.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let hub = self.clone();
                let spawned = thread::Builder::new()
                    .name("lumit-share-guest".into())
                    .spawn(move || {
                        let _ = hub.seat(socket);
                    });
                if spawned.is_err() {
                    self.greeting.fetch_sub(1, Ordering::Relaxed);
                }
            }
            if quiet {
                let looking = self.looking.lock().take();
                if let Some(presence) = looking {
                    self.presence(0, presence);
                }
                self.tidy();
                thread::sleep(BEAT);
            }
        }
    }

    /// Greet one caller and, if it holds the invite and runs this version,
    /// seat it and read from it until it goes.
    fn seat(self: &Arc<Self>, socket: TcpStream) -> Result<(), ShareError> {
        // The invite as it stands. A caller still saying hello when it is
        // replaced is not seated.
        let (key, channel) = {
            let seats = self.seats.lock();
            (seats.key, seats.channel())
        };
        let greeted = (|| {
            // An accepted socket takes after its listener on Windows.
            socket.set_nonblocking(false)?;
            let (sender, mut receiver) = wire::open(socket.try_clone()?, &channel, false)?;
            let hello = decode(&receiver.recv(1 << 16)?, &self.names, None)?;
            Ok::<_, ShareError>((sender, receiver, hello))
        })();
        self.greeting.fetch_sub(1, Ordering::Relaxed);
        let (mut sender, mut receiver, hello) = greeted?;
        let (protocol, version, schema, name, token) = match hello {
            Message::Hello {
                protocol,
                version,
                schema,
                name,
                token,
            } => (protocol, version, schema, name, token),
            Message::Bulk { protocol, token } if protocol == wire::PROTOCOL => {
                self.carry(token, socket, sender, receiver);
                return Ok(());
            }
            _ => return Err(ShareError::OutOfTurn),
        };
        if protocol != wire::PROTOCOL
            || version != VERSION
            || schema != lumit_project::SCHEMA_VERSION
        {
            let host = VERSION.to_owned();
            self.refuse(&mut sender, Refusal::Version { host });
            return Ok(());
        }

        let (out, outbox) = sync_channel(wire::OUTBOX);
        let seat_out = out.clone();
        let seated = self.store.frozen(|document| {
            let mut seats = self.seats.lock();
            if seats.key != key {
                return Err(ShareError::OutOfTurn);
            }
            // The connection this guest had before is over, noticed here yet
            // or not. Nothing more is taken from it, bar an edit already on
            // its way in, which is counted below and reaches this guest like
            // anyone else's.
            let old = seats.links.iter().position(|link| link.token == token);
            if let Some(old) = old.filter(|_| token != 0) {
                let old = seats.links.remove(old);
                let _ = old.socket.shutdown(Shutdown::Both);
                seats.people.retain(|p| p.id != old.id);
            }
            if seats.people.len() >= MAX_PEOPLE {
                return Ok(None);
            }
            let id = seats.next;
            seats.next += 1;
            let colour = (1..=u8::MAX)
                .find(|c| seats.people.iter().all(|p| p.colour != *c))
                .unwrap_or(1);
            seats.people.push(Person {
                id,
                name: name.chars().take(64).collect(),
                colour,
                presence: Presence::default(),
            });
            seats.links.push(Link {
                id,
                token,
                out: seat_out,
                socket,
            });
            seats.changed = true;
            let heard = seats.heard.iter().find(|(t, _)| *t == token);
            let heard = heard.map_or(0, |(_, id)| *id);
            Ok(Some((id, heard, document, seats.people.clone())))
        })?;
        let Some((id, heard, document, people)) = seated else {
            self.refuse(&mut sender, Refusal::Full);
            return Ok(());
        };

        receiver.patience(wire::QUIET);
        // The document first, then whatever has queued behind it. Encoded
        // here rather than while the store was held still.
        let welcome = encode(
            &Message::Welcome {
                you: id,
                heard,
                document: Box::new(Document::clone(&document)),
                people,
            },
            &self.names,
        );
        let hub = self.clone();
        let spawned = thread::Builder::new()
            .name("lumit-share-send".into())
            .spawn(move || match welcome {
                Ok(bytes) if sender.send(&bytes).is_ok() => {
                    wire::write_loop(sender, &outbox, &hub.names, || None);
                }
                _ => sender.close(),
            });
        if spawned.is_ok() {
            self.listen((id, token), &mut receiver, &out);
        }
        self.unseat(id);
        spawned?;
        Ok(())
    }

    fn refuse(&self, sender: &mut Sender, refusal: Refusal) {
        if let Ok(bytes) = encode(&Message::Refused(refusal), &self.names) {
            let _ = sender.send(&bytes);
        }
        sender.close();
    }

    /// Note that the guest `token` names sent edit `id` on connection `peer`,
    /// before it is taken. False when that connection is no longer the
    /// guest's seat: it has come back on another, or been taken out, and the
    /// edit is not wanted. Under the lock a guest is seated under, so one
    /// that comes back is told of every edit that was let past here.
    fn hear(&self, peer: u32, token: u64, id: u64) -> bool {
        let mut seats = self.seats.lock();
        if !seats.links.iter().any(|link| link.id == peer) {
            return false;
        }
        if token != 0 {
            seats.heard.retain(|(t, _)| *t != token);
            seats.heard.push((token, id));
            if seats.heard.len() > HEARD {
                seats.heard.remove(0);
            }
        }
        true
    }

    /// Read one guest's edits until it goes or says something out of turn.
    fn listen(&self, (peer, token): (u32, u64), receiver: &mut Receiver, out: &SyncSender<Out>) {
        while !self.stop.load(Ordering::Relaxed) {
            let Ok(bytes) = receiver.recv(wire::EDIT_LIMIT) else {
                break;
            };
            match decode(&bytes, &self.names, self.root.as_deref()) {
                Ok(Message::Submit { id, mut op, was }) => {
                    place(&mut op, &self.store.snapshot(), self.root.as_deref());
                    let tag = RemoteTag { peer, id };
                    // Applied, the tap has already told everyone, this guest
                    // included. Refused, only this guest needs to hear, and
                    // in its place among the edits that were applied.
                    //
                    // `was` is only ever compared with, so nothing in it
                    // reaches the document and it needs no checking.
                    if !self.hear(peer, token, id) {
                        break;
                    }
                    if sane(&op) && self.store.commit_remote(&op, Some(&was), tag).is_ok() {
                        continue;
                    }
                    let Ok(bytes) = encode(&Message::Rejected { id }, &self.names) else {
                        break;
                    };
                    if out.try_send(Out::Bytes(bytes)).is_err() {
                        break;
                    }
                }
                Ok(Message::Presence { presence, .. }) => self.presence(peer, presence),
                Ok(Message::Holds { items, .. }) => self.hold(peer, items),
                Ok(Message::Note { to, body, .. }) => self.pass(peer, to, body),
                Ok(Message::Ping) => {}
                _ => break,
            }
        }
    }

    /// Someone is looking at something else: note it and tell the others.
    fn presence(&self, peer: u32, presence: Presence) {
        let presence = presence.tidied();
        let people = {
            let mut seats = self.seats.lock();
            let Some(person) = seats.people.iter_mut().find(|p| p.id == peer) else {
                return;
            };
            person.presence = presence.clone();
            if let Ok(bytes) = encode(&Message::Presence { peer, presence }, &self.names) {
                // Latest wins, so a guest with a full queue misses this one
                // rather than being dropped for it.
                for link in seats.links.iter().filter(|l| l.id != peer) {
                    let _ = link.out.try_send(Out::Bytes(bytes.clone()));
                }
            }
            seats.people.clone()
        };
        (self.events)(Event::People { me: 0, people });
    }

    fn unseat(&self, id: u32) {
        let mut seats = self.seats.lock();
        seats.links.retain(|link| {
            if link.id == id {
                let _ = link.socket.shutdown(Shutdown::Both);
            }
            link.id != id
        });
        seats.people.retain(|p| p.id != id);
        seats.changed = true;
        drop(seats);
        if let Some(bulk) = self.bulk.lock().clone() {
            bulk.forget(id);
        }
    }

    /// Tell everyone who is here, if that has changed. From the accepting
    /// thread, never from the tap: events cross into the frontend, and the
    /// tap runs under the store's lock.
    fn tidy(&self) {
        let people = {
            let mut seats = self.seats.lock();
            if !std::mem::take(&mut seats.changed) {
                return;
            }
            let people = seats.people.clone();
            if let Ok(bytes) = encode(&Message::People(people.clone()), &self.names) {
                Self::send_each(&mut seats, |_| bytes.clone());
            }
            // And who has which footage, whole, so whoever has just come
            // knows it and nobody still counts on someone who has gone.
            let bulk = self.bulk.lock().clone();
            for (peer, items) in bulk.map_or_else(Vec::new, |bulk| bulk.all_holds()) {
                let here = people.iter().any(|p| p.id == peer);
                let items = if here { items } else { Vec::new() };
                if let Ok(bytes) = encode(&Message::Holds { peer, items }, &self.names) {
                    Self::send_each(&mut seats, |_| bytes.clone());
                }
            }
            people
        };
        (self.events)(Event::People { me: 0, people });
    }
}
