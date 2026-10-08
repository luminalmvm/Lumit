//! A guest's end of a shared project: it sends the edits made here, applies
//! the ones the host sends, and when the host is lost keeps working and
//! merges on the way back.
//!
//! One thread reads and reconnects. Each connection has a writer.

use crate::host::VERSION;
use crate::kept::{Finding, Kept, Pair};
use crate::local::{carry, place, sane, sane_document, settle};
use crate::wire::{self, decode, encode, Message, Names, Out, Receiver, Sender};
use crate::{Conflict, Ending, Event, Events, Invite, Person, Presence, ShareError};
use lumit_core::shared::land;
use lumit_core::store::{Moved, RemoteTag, Tap};
use lumit_core::{Document, DocumentStore};
use lumit_project::resolve_all_media;
use parking_lot::Mutex;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

/// The longest a guest waits between tries at finding its host again.
const LONGEST_WAIT: u64 = 15;

/// A connection that has been welcomed.
struct Connected {
    sender: Sender,
    receiver: Receiver,
    socket: TcpStream,
    you: u32,
    people: Vec<Person>,
}

/// A connection the store's edits are being queued for and sent down.
struct Live {
    receiver: Receiver,
    people: Vec<Person>,
}

/// What a guest reconnects with.
struct Seat {
    /// Where the host is and the secret that lets this guest in. Replaced
    /// when the person is given a new invite for a host that has moved.
    invite: Mutex<Invite>,
    name: String,
    /// The folder this machine keeps the project's footage under.
    root: Option<PathBuf>,
    names: Names,
}

/// A guest that has the host's document and has not started editing it.
pub struct Joining {
    connected: Connected,
    seat: Seat,
    /// The host's own id for the project.
    project: Uuid,
}

/// A guest's own copy, opened again with edits in it that were made while
/// the host was away and never merged.
pub struct Resuming {
    seat: Seat,
    project: Uuid,
    base: Arc<Document>,
    since: Vec<Pair>,
    kept: Kept,
}

/// Pick up a guest's copy where it was left, if it was left with its host
/// away. `file` is the copy as its file in `folder` holds it. Answers the
/// copy as it stood when it was closed, every edit made while away included
/// whether or not it was saved, and what goes on looking for the host.
#[must_use]
pub fn resume(file: &mut Document, folder: &Path) -> Option<(Document, Resuming)> {
    let (finding, mut base, mut since) = Kept::read(file.id)?;
    let root = finding.root.as_deref();
    // What was kept has no paths in it: where footage is on this machine is
    // never written down. The copy's own file is what knows.
    resolve_all_media(file, folder, &[]);
    carry(&mut base, file);
    if let Some(root) = root {
        resolve_all_media(&mut base, root, &[]);
    }
    let mut document = base.clone();
    // Landed, as each was on the copy when it was closed. One that does not
    // land follows a line the file lost, and is left out of the merge too.
    since.retain_mut(|(op, was)| {
        place(op, file, root);
        land(&mut document, op, Some(was)).is_ok()
    });
    let base = Arc::new(base);
    // Written again, so the file and the store count the same edits.
    let kept = Kept::begin(&finding, &base, &since)?;
    let seat = Seat {
        invite: Mutex::new(finding.invite.parse().ok()?),
        name: finding.name,
        root: finding.root,
        names: Names::default(),
    };
    let resuming = Resuming {
        seat,
        project: finding.project,
        base,
        since,
        kept,
    };
    Some((document, resuming))
}

/// Reach the host an invite names, and say hello.
fn connect(seat: &Seat) -> Result<(Connected, Document), ShareError> {
    let invite = seat.invite.lock().clone();
    let socket = invite
        .address
        .to_socket_addrs()
        .map_err(|_| ShareError::Unreachable)?
        .find_map(|addr| TcpStream::connect_timeout(&addr, wire::GREETING).ok())
        .ok_or(ShareError::Unreachable)?;
    let (mut sender, mut receiver) = wire::open(socket.try_clone()?, &invite.key, true)?;
    let hello = Message::Hello {
        protocol: wire::PROTOCOL,
        version: VERSION.to_owned(),
        schema: lumit_project::SCHEMA_VERSION.to_owned(),
        name: seat.name.clone(),
    };
    sender.send(&encode(&hello, &seat.names)?)?;
    match decode(&receiver.recv(wire::DOCUMENT_LIMIT)?, &seat.names)? {
        Message::Welcome {
            you,
            document,
            people,
        } => {
            if !sane_document(&document) {
                return Err(ShareError::Unsafe);
            }
            receiver.patience(wire::QUIET);
            let connected = Connected {
                sender,
                receiver,
                socket,
                you,
                people,
            };
            Ok((connected, *document))
        }
        Message::Refused(refusal) => Err(ShareError::Refused(refusal)),
        _ => Err(ShareError::OutOfTurn),
    }
}

/// Join the project an invite names. Answers the host's document, with its
/// footage pointed at this machine's copies under `root` where they were
/// found, and the connection to start editing on.
///
/// Blocks while it connects and while the document arrives, so not for the
/// UI thread.
pub fn join(
    invite: Invite,
    name: &str,
    root: Option<PathBuf>,
) -> Result<(Document, Joining), ShareError> {
    let seat = Seat {
        invite: Mutex::new(invite),
        name: name.chars().take(64).collect(),
        root,
        names: Names::default(),
    };
    let (connected, mut document) = connect(&seat)?;
    let project = document.id;
    settle(&mut document, None, seat.root.as_deref());
    let joining = Joining {
        connected,
        seat,
        project,
    };
    Ok((document, joining))
}

struct Inner {
    store: Arc<DocumentStore>,
    events: Events,
    seat: Seat,
    /// The host's own id for the project this guest joined. A host found
    /// again has to be sharing the same one, or there is nothing to merge
    /// with.
    project: Uuid,
    stop: AtomicBool,
    /// A new invite has come, so the wait before the next try is cut short.
    hurry: AtomicBool,
    me: AtomicU32,
    /// The way to the host while there is one.
    link: Mutex<Option<(SyncSender<Out>, TcpStream)>>,
    /// What this end is looking at, if the host has not been told yet.
    presence: Mutex<Option<Presence>>,
    people: Mutex<Vec<Person>>,
    /// Edits held back by a merge, until the person chooses. Each is removed
    /// by [`Guest::resolve`], and all go when sharing stops.
    conflicts: Mutex<Vec<Conflict>>,
    /// The edits made since the host was lost, on disk. There while the host
    /// is away. Taken before the store's lock, never under it.
    kept: Mutex<Option<Kept>>,
    /// The person has left, so nothing more is kept.
    left: AtomicBool,
}

/// A project shared from someone else's machine. Leaving it is [`Self::stop`]
/// or a drop.
pub struct Guest {
    inner: Arc<Inner>,
}

impl Inner {
    fn new(store: Arc<DocumentStore>, events: Events, seat: Seat, project: Uuid) -> Arc<Self> {
        Arc::new(Inner {
            store,
            events,
            seat,
            project,
            stop: AtomicBool::new(false),
            hurry: AtomicBool::new(false),
            me: AtomicU32::new(0),
            link: Mutex::new(None),
            presence: Mutex::new(None),
            people: Mutex::new(Vec::new()),
            conflicts: Mutex::new(Vec::new()),
            kept: Mutex::new(None),
            left: AtomicBool::new(false),
        })
    }

    /// The store's tap. Weak, or the store would hold the guest that holds
    /// the store.
    fn tap(self: &Arc<Self>) -> Tap {
        let tapped = Arc::downgrade(self);
        Arc::new(move |moved| {
            if let Some(inner) = tapped.upgrade() {
                inner.moved(moved);
            }
        })
    }

    /// Start the guest's thread, with a connection or looking for one.
    fn begin(self: &Arc<Self>, live: Option<Live>) -> Result<Guest, ShareError> {
        let running = self.clone();
        let spawned = thread::Builder::new()
            .name("lumit-share-host".into())
            .spawn(move || running.run(live));
        if let Err(e) = spawned {
            self.end();
            return Err(e.into());
        }
        Ok(Guest {
            inner: self.clone(),
        })
    }
}

impl Joining {
    /// Start editing. `store` holds exactly the document [`join`] answered.
    pub fn start(self, store: Arc<DocumentStore>, events: Events) -> Result<Guest, ShareError> {
        let inner = Inner::new(store, events, self.seat, self.project);
        // The way out before the store is shared, or an edit made in between
        // would wait for an answer to a message that was never sent.
        let live = inner.link_up(self.connected, 0);
        inner.store.share(true, inner.tap());
        inner.begin(live)
    }
}

impl Resuming {
    /// Carry on editing, and looking for the host. `store` holds the
    /// document [`resume`] answered.
    pub fn start(self, store: Arc<DocumentStore>, events: Events) -> Result<Guest, ShareError> {
        let inner = Inner::new(store, events, self.seat, self.project);
        *inner.kept.lock() = Some(self.kept);
        let tap = inner.tap();
        inner.store.share_apart(tap, self.base, self.since);
        inner.begin(None)
    }
}

impl Guest {
    #[must_use]
    pub fn people(&self) -> Vec<Person> {
        self.inner.people.lock().clone()
    }

    #[must_use]
    pub fn me(&self) -> u32 {
        self.inner.me.load(Ordering::Relaxed)
    }

    pub fn set_presence(&self, presence: Presence) {
        *self.inner.presence.lock() = Some(presence.tidied());
    }

    /// The edits a merge held back, each waiting on [`Self::resolve`].
    #[must_use]
    pub fn conflicts(&self) -> Vec<Conflict> {
        self.inner.conflicts.lock().clone()
    }

    /// Look for the host by a new invite from now on, for a host that has
    /// moved or made a new one. Only matters while the host is away: the
    /// next try uses it.
    pub fn reinvite(&self, invite: Invite) {
        *self.inner.seat.invite.lock() = invite;
        // What is kept names the old invite, and is written again with this.
        if let Some(kept) = self.inner.kept.lock().as_mut() {
            kept.stale = true;
        }
        self.inner.hurry.store(true, Ordering::Relaxed);
    }

    /// Settle the conflict at `index`: keep mine, which applies the held
    /// edits as one undo step, or keep theirs, which drops them. Keeping mine
    /// writes only what was changed here, so whatever else has changed around
    /// it since stays. Answers how many of the edits no longer applied.
    pub fn resolve(&self, index: usize, mine: bool) -> usize {
        let conflict = {
            let mut conflicts = self.inner.conflicts.lock();
            if index >= conflicts.len() {
                return 0;
            }
            conflicts.remove(index)
        };
        if !mine {
            return 0;
        }
        let store = &self.inner.store;
        store.begin_undo_group();
        let failed = conflict
            .ops
            .into_iter()
            .filter(|(op, was)| store.commit_over(op.clone(), was).is_err())
            .count();
        store.end_undo_group();
        failed
    }

    /// Lose the connection, as a network would.
    #[cfg(test)]
    pub(crate) fn cut(&self) {
        self.inner.hang_up();
    }

    /// Whether this guest is without its host just now.
    #[must_use]
    pub fn away(&self) -> bool {
        self.inner.link.lock().is_none()
    }

    /// Leave. The document stays as it is, and what was being kept to merge
    /// with the host is let go of.
    pub fn stop(&self) {
        self.inner.left.store(true, Ordering::Relaxed);
        if let Some(kept) = self.inner.kept.lock().take() {
            kept.remove();
        }
        self.inner.end();
    }
}

/// A drop is the project closing, not the person leaving. Edits made with
/// the host away stay on disk, and the copy opened again carries on. So do
/// edits the host had not answered yet, which closing would otherwise lose.
impl Drop for Guest {
    fn drop(&mut self) {
        // Said first, so a merge finishing this moment leaves what is kept.
        self.inner.stop.store(true, Ordering::Relaxed);
        let away = self.away();
        self.inner.store.cast_adrift();
        if away || self.inner.store.unanswered() > 0 {
            self.inner.keep();
        }
        self.inner.end();
    }
}

impl Inner {
    /// The store's tap: an edit was made here, so the host is sent it. If it
    /// cannot be queued the connection is dropped, and the edit goes with the
    /// merge when the host is found again. Runs under the store's lock.
    fn moved(&self, moved: Moved<'_>) {
        let Moved::Local { id, op, was } = moved else {
            return;
        };
        let (op, was) = (op.clone(), Box::new(was.clone()));
        let submit = Message::Submit { id, op, was };
        let mut link = self.link.lock();
        let sent = encode(&submit, &self.seat.names)
            .ok()
            .zip(link.as_ref())
            .is_some_and(|(bytes, (out, _))| out.try_send(Out::Bytes(bytes)).is_ok());
        if !sent {
            if let Some((_, socket)) = link.take() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }

    fn tell_people(&self, people: Vec<Person>) {
        let me = self.me.load(Ordering::Relaxed);
        (self.events)(Event::People { me, people });
    }

    /// Without the host nobody else can be seen, so the list is this person
    /// alone until it is back. Where the others were last is not where they
    /// are.
    fn alone(&self) {
        let me = self.me.load(Ordering::Relaxed);
        let people = {
            let mut people = self.people.lock();
            people.retain(|p| p.id == me);
            if people.is_empty() {
                // A copy opened again has not been given a seat yet.
                people.push(Person {
                    id: me,
                    name: self.seat.name.clone(),
                    colour: 1,
                    presence: Presence::default(),
                });
            }
            people.clone()
        };
        self.tell_people(people);
    }

    fn hang_up(&self) {
        if let Some((_, socket)) = self.link.lock().take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    /// Stop for good. The store is let go of here and not on the guest's
    /// thread, which can be some seconds noticing: by then the store may be
    /// shared again, as a host's or another guest's.
    fn end(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.hang_up();
        self.store.unshare();
    }

    /// Sharing is over and this thread is the one that found out.
    fn finish(&self) {
        if !self.stop.load(Ordering::Relaxed) {
            self.store.unshare();
        }
    }

    /// Write down what a guest without its host would lose if Lumit closed:
    /// the document both last had, the first time, and each edit made since.
    /// Does nothing for a guest that has its host.
    fn keep(&self) {
        let mut kept = self.kept.lock();
        // Under the lock, so a guest that has left is not kept for after all.
        if self.left.load(Ordering::Relaxed) {
            return;
        }
        let from = kept.as_ref().map_or(0, |kept| kept.written);
        let Some((base, since)) = self.store.apart(from) else {
            return;
        };
        match kept.as_mut() {
            Some(kept) if !kept.stale && Arc::ptr_eq(&kept.base, &base) => kept.append(&since),
            // Nothing kept yet, or what is kept follows a document the store
            // has merged past, or names an invite that has been replaced.
            _ => {
                let Some((base, since)) = self.store.apart(0) else {
                    return;
                };
                let invite = self.seat.invite.lock().to_string();
                let (name, root) = (self.seat.name.clone(), self.seat.root.clone());
                let finding = Finding::new(self.project, invite, name, root);
                if let Some(again) = Kept::begin(&finding, &base, &since) {
                    *kept = Some(again);
                }
            }
        }
    }

    /// Make a fresh connection the way out for the store's edits, and start
    /// its writer. `room` is how many edits are about to be queued at once on
    /// top of the usual: a merge queues every edit it keeps before any of
    /// them is answered. The writer runs from here so the host hears from
    /// this guest all through a long merge. `None` when it would not start.
    fn link_up(self: &Arc<Self>, connected: Connected, room: usize) -> Option<Live> {
        let (out, outbox) = sync_channel(wire::OUTBOX + room);
        *self.link.lock() = Some((out, connected.socket));
        self.me.store(connected.you, Ordering::Relaxed);
        self.people.lock().clone_from(&connected.people);
        let writing = self.clone();
        let sender = connected.sender;
        let spawned = thread::Builder::new()
            .name("lumit-share-send".into())
            .spawn(move || {
                wire::write_loop(sender, &outbox, &writing.seat.names, || {
                    let presence = writing.presence.lock().take()?;
                    Some(Message::Presence { peer: 0, presence })
                });
            });
        if spawned.is_err() {
            self.hang_up();
            return None;
        }
        Some(Live {
            receiver: connected.receiver,
            people: connected.people,
        })
    }

    /// The guest's thread: read from the host until it is lost, then look for
    /// it again and merge, for as long as the project is shared.
    fn run(self: Arc<Self>, mut live: Option<Live>) {
        loop {
            if let Some(mut live) = live.take() {
                self.tell_people(live.people);
                let ended = self.listen(&mut live.receiver);
                self.hang_up();
                if ended.is_some() || self.stop.load(Ordering::Relaxed) {
                    self.finish();
                    if let Some(ending) = ended {
                        (self.events)(Event::Ended(ending));
                    }
                    return;
                }
            }
            self.store.cast_adrift();
            self.alone();
            (self.events)(Event::Away);
            self.keep();

            let mut wait = 1;
            // The invite the person has been told leads somewhere else.
            let mut elsewhere = None;
            live = loop {
                if self.stop.load(Ordering::Relaxed) {
                    return;
                }
                let ending = match connect(&self.seat) {
                    // The project closed while the host was being asked. What
                    // was kept stays kept, for the copy opened again.
                    Ok(_) if self.stop.load(Ordering::Relaxed) => continue,
                    // Another project altogether. There is nothing here to
                    // merge with, so it is said once and the looking goes on.
                    Ok((_, document)) if document.id != self.project => {
                        let invite = Some(self.seat.invite.lock().clone());
                        if elsewhere != invite {
                            elsewhere = invite;
                            (self.events)(Event::Elsewhere);
                        }
                        wait = self.wait(wait);
                        continue;
                    }
                    Ok((connected, mut document)) => {
                        // The way out before the merge, which sends through it.
                        let room = self.store.unanswered();
                        let Some(live) = self.link_up(connected, room) else {
                            wait = self.wait(wait);
                            continue;
                        };
                        let have = self.store.snapshot();
                        settle(&mut document, Some(&have), self.seat.root.as_deref());
                        self.keep();
                        let (conflicts, refused) = self.store.rejoin(document);
                        {
                            // Merged, so there is nothing left to keep. Not
                            // if the project closed meanwhile: then nothing
                            // of the merge was sent, and what is kept is
                            // what the copy opened again merges from.
                            let mut kept = self.kept.lock();
                            if self.stop.load(Ordering::Relaxed) {
                                return;
                            }
                            if let Some(kept) = kept.take() {
                                kept.remove();
                            }
                        }
                        let held = {
                            let mut all = self.conflicts.lock();
                            all.extend(conflicts);
                            all.len()
                        };
                        (self.events)(Event::Back { held, refused });
                        break Some(live);
                    }
                    Err(ShareError::Refused(refusal)) => Ending::Refused(refusal),
                    Err(ShareError::Unsafe) => Ending::Unsafe,
                    Err(_) => {
                        wait = self.wait(wait);
                        continue;
                    }
                };
                if self.stop.load(Ordering::Relaxed) {
                    return;
                }
                self.finish();
                // Not kept past here. The copy is edited on with nothing
                // writing those edits down, so what was kept would be an
                // older copy by the time it was opened again.
                if let Some(kept) = self.kept.lock().take() {
                    kept.remove();
                }
                (self.events)(Event::Ended(ending));
                return;
            };
        }
    }

    /// Wait `seconds` before looking for the host again, and answer how long
    /// to wait next time. In short steps, so leaving does not wait on it, a
    /// new invite is tried at once, and an edit made meanwhile is on disk
    /// within one of them.
    fn wait(&self, seconds: u64) -> u64 {
        for _ in 0..seconds * 10 {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            self.keep();
            if self.hurry.swap(false, Ordering::Relaxed) {
                return 1;
            }
            thread::sleep(Duration::from_millis(100));
        }
        (seconds * 2).min(LONGEST_WAIT)
    }

    /// Read from the host until the connection goes or the two documents are
    /// found to have drifted apart, either of which is put right by finding
    /// the host again. An ending when the host said this is over.
    fn listen(&self, receiver: &mut Receiver) -> Option<Ending> {
        let me = self.me.load(Ordering::Relaxed);
        while !self.stop.load(Ordering::Relaxed) {
            let Ok(bytes) = receiver.recv(wire::DOCUMENT_LIMIT) else {
                break;
            };
            match decode(&bytes, &self.seat.names) {
                Ok(Message::Applied { peer, id, mut op }) => {
                    if !sane(&op) {
                        break;
                    }
                    place(&mut op, &self.store.snapshot(), self.seat.root.as_deref());
                    // One of this guest's own comes back when the host
                    // applied it differently from how it was sent.
                    let applied = if peer == me {
                        self.store.commit_answer(id, &op)
                    } else {
                        self.store.commit_remote(&op, None, RemoteTag { peer, id })
                    };
                    if applied.is_err() {
                        break;
                    }
                }
                Ok(Message::Accepted { id }) => {
                    if !self.store.acknowledge(id) {
                        break;
                    }
                }
                Ok(Message::Rejected { id }) => {
                    if self.store.reject(id).is_err() {
                        break;
                    }
                }
                Ok(Message::People(people)) => {
                    self.people.lock().clone_from(&people);
                    self.tell_people(people);
                }
                Ok(Message::Presence { peer, presence }) => {
                    let people = {
                        let mut people = self.people.lock();
                        if let Some(person) = people.iter_mut().find(|p| p.id == peer) {
                            person.presence = presence;
                        }
                        people.clone()
                    };
                    self.tell_people(people);
                }
                Ok(Message::Ping) => {}
                Ok(Message::Closed) => return Some(Ending::Closed),
                Ok(Message::Removed) => return Some(Ending::Removed),
                _ => break,
            }
        }
        None
    }
}
