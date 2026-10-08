//! The host's end of a shared project: it listens, seats guests, and sends
//! every edit to everyone in the order its store applied them.
//!
//! One thread accepts. Each guest has a reader, which is the thread that
//! greeted it, and a writer.

use crate::kept::{HostLog, Pair};
use crate::local::{place, sane};
use crate::wire::{self, decode, encode, Message, Names, Out, Receiver, Sender};
use crate::{Event, Events, Invite, Person, Presence, Refusal, ShareError, MAX_PEOPLE};
use lumit_core::store::{Moved, RemoteTag};
use lumit_core::{Document, DocumentStore};
use parking_lot::Mutex;
use std::net::{IpAddr, Ipv6Addr, Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Both ends have to run this, edits being the wire format.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How many connections may be part-way through saying hello. Anyone can
/// connect before proving they hold the invite, so this is what a flood of
/// strangers is held to.
const GREETING_ROOM: usize = 8;

/// How often the accepting thread looks for a caller, tidies up, and sends on
/// what the host is looking at.
const BEAT: Duration = Duration::from_millis(50);

/// The way to one seated guest.
struct Link {
    id: u32,
    out: SyncSender<Out>,
    socket: TcpStream,
}

/// Who is here. Bounded by [`MAX_PEOPLE`].
struct Seats {
    people: Vec<Person>,
    links: Vec<Link>,
    next: u32,
    /// Someone came or went and the others have not been told yet.
    changed: bool,
}

struct Hub {
    store: Arc<DocumentStore>,
    key: [u8; 32],
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
}

/// A project being shared from this machine.
///
/// [`Self::stop`] ends it for everyone. A drop only lets go: the guests are
/// not told it is over, so they keep what they do next and bring it back
/// when the same project is shared again with the same key.
pub struct Host {
    hub: Arc<Hub>,
    accepting: Mutex<Option<JoinHandle<()>>>,
    restored: usize,
}

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
/// back by it. `None` makes a new one.
pub fn host(
    store: Arc<DocumentStore>,
    name: &str,
    address: IpAddr,
    port: u16,
    key: Option<[u8; 32]>,
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
        key,
        port,
        root,
        events,
        stop: AtomicBool::new(false),
        greeting: AtomicUsize::new(0),
        seats: Mutex::new(Seats {
            people: vec![Person {
                id: 0,
                name: name.chars().take(64).collect(),
                colour: 1,
                presence: Presence::default(),
            }],
            links: Vec::new(),
            next: 1,
            changed: false,
        }),
        looking: Mutex::new(None),
        names: Names::default(),
        log: Mutex::new(log),
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
            restored,
        }),
        Err(e) => {
            hub.store.unshare();
            Err(e.into())
        }
    }
}

impl Host {
    /// The invite to hand out, for a host reached at `address`.
    #[must_use]
    pub fn invite(&self, address: &str) -> Invite {
        let address = address.trim();
        // An IPv6 address has colons of its own, so it goes in brackets.
        let bare = address.contains(':') && !address.starts_with('[');
        let (open, close) = if bare { ("[", "]") } else { ("", "") };
        Invite {
            address: format!("{open}{address}{close}:{}", self.hub.port),
            key: self.hub.key,
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
        self.hub.key
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

    /// Latest wins: the accepting thread sends it on its next beat, so a
    /// playhead that moves every frame is not a message every frame.
    pub fn set_presence(&self, presence: Presence) {
        *self.hub.looking.lock() = Some(presence);
    }

    /// Take one guest out of the project. They are told, and their Lumit
    /// stops trying to come back. Anyone who holds the invite can still join
    /// with it, so keeping someone out for good takes a new invite.
    pub fn remove(&self, id: u32) {
        let seats = self.hub.seats.lock();
        if let Some(link) = seats.links.iter().find(|link| link.id == id) {
            self.hub.dismiss(link, &Message::Removed);
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
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.end(None);
    }
}

impl Hub {
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
        let greeted = (|| {
            // An accepted socket takes after its listener on Windows.
            socket.set_nonblocking(false)?;
            let (sender, mut receiver) = wire::open(socket.try_clone()?, &self.key, false)?;
            let hello = decode(&receiver.recv(1 << 16)?, &self.names)?;
            Ok::<_, ShareError>((sender, receiver, hello))
        })();
        self.greeting.fetch_sub(1, Ordering::Relaxed);
        let (mut sender, mut receiver, hello) = greeted?;
        let Message::Hello {
            protocol,
            version,
            schema,
            name,
        } = hello
        else {
            return Err(ShareError::OutOfTurn);
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
            if seats.people.len() >= MAX_PEOPLE {
                return None;
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
                out: seat_out,
                socket,
            });
            seats.changed = true;
            Some((id, document, seats.people.clone()))
        });
        let Some((id, document, people)) = seated else {
            self.refuse(&mut sender, Refusal::Full);
            return Ok(());
        };

        receiver.patience(wire::QUIET);
        // The document first, then whatever has queued behind it. Encoded
        // here rather than while the store was held still.
        let welcome = encode(
            &Message::Welcome {
                you: id,
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
            self.listen(id, &mut receiver, &out);
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

    /// Read one guest's edits until it goes or says something out of turn.
    fn listen(&self, peer: u32, receiver: &mut Receiver, out: &SyncSender<Out>) {
        while !self.stop.load(Ordering::Relaxed) {
            let Ok(bytes) = receiver.recv(wire::EDIT_LIMIT) else {
                break;
            };
            match decode(&bytes, &self.names) {
                Ok(Message::Submit { id, mut op, was }) => {
                    place(&mut op, &self.store.snapshot(), self.root.as_deref());
                    let tag = RemoteTag { peer, id };
                    // Applied, the tap has already told everyone, this guest
                    // included. Refused, only this guest needs to hear, and
                    // in its place among the edits that were applied.
                    //
                    // `was` is only ever compared with, so nothing in it
                    // reaches the document and it needs no checking.
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
            people
        };
        (self.events)(Event::People { me: 0, people });
    }
}
