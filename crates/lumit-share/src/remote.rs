//! The link between the two ends of an export done on another computer: the
//! sender, which has the project and asks, and the worker, which does the
//! export. Lumit's server tells each about the other. This is the way the
//! files go: the project, the footage the worker lacks, and the finished
//! file coming back.
//!
//! Files cross with bulk, the worker as its host and the sender as its one
//! guest, so the sender can want the export and the worker fetches what the
//! sender holds. Either machine may sleep or lose its network at any time,
//! so the sender keeps reaching the worker until it is stopped, and a file
//! carries on from where it stopped once the link is back.
//!
//! Both ends hold the job's key. It opens the channel, and names the room
//! the worker keeps at a relay.
//!
//! Threads: the worker has one that accepts and one that keeps its room at a
//! relay, and greets each caller on a thread of its own, which goes on to
//! read its link. The sender has one that keeps reaching the worker and runs
//! the link when it has one, and for a moment one for each place it tries.
//! Bulk has its own for each link. None is the UI thread.

use crate::bulk::{Footage, Held, Hub, Limits, Wanted};
use crate::guest::HEAD_START;
use crate::host::{self, GREETING_EACH, GREETING_ROOM, GUESS_PAUSE};
use crate::invite::MAX_ADDRESSES;
use crate::wire::{self, Receiver, Sender};
use crate::{dialled, global_address, local_address, ShareError};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;
use web_time::{Duration, Instant};

/// Raised whenever what crosses on a job's link changes shape, bulk's own
/// frames included.
const VERSION: u32 = 1;

/// The longest hello read, which is a long list of footage: about 70 bytes
/// a file, so a million files, as many as the biggest manifest a job can
/// have lists. Only read from a caller that has shown it holds the key.
const HELLO_LIMIT: u32 = 64 << 20;

/// How often a thread that waits looks to see whether it has been stopped.
const BEAT: Duration = Duration::from_millis(50);

/// How long the sender waits after a try that failed, doubled each time
/// after, up to the longest.
const FIRST_WAIT: Duration = Duration::from_millis(500);
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// How long a link has to last for the sender to try again at once when it
/// drops. One that drops sooner is no better than not reaching the worker.
const STEADY: Duration = Duration::from_secs(1);

/// Where a worker listens: every address. Tests keep to this machine, so
/// running them never asks the firewall.
const EVERY: IpAddr = if cfg!(test) {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
} else {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
};

/// What a job's link tells whoever runs it. Called from the link's threads
/// with no lock of this module's held but the one that keeps these in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent {
    /// The other end is here. `relayed` is through the relay, which is
    /// slower than a way of its own.
    Linked { relayed: bool },
    /// It has gone. The sender is already reaching it again.
    Unlinked,
}

/// The sender's first words on each connection. The worker answers with
/// the same, holding nothing, which is how the sender knows it was let in.
///
/// What the sender holds goes only here. When it changes, the link is cut
/// and made again with a new hello: once bulk runs it has the connection to
/// itself, and a frame of its own for this would change bulk for something
/// said only when the project's footage changes. A cut costs a handshake,
/// and what was crossing carries on from where it stopped.
#[derive(Serialize, Deserialize)]
struct Hello {
    version: u32,
    job: Uuid,
    holds: Vec<Held>,
}

/// The name of a job's room at a relay. Worked out from the key, which the
/// relay cannot work back from it, with words of its own so it is never a
/// shared project's room.
fn room(key: &[u8; 32]) -> String {
    let name = blake3::derive_key("lumit-share 2026 remote export room", key);
    hex::encode(&name[..16])
}

/// A connection that has proved both ends hold the key, and whether it is
/// through the relay.
type Opened = (TcpStream, Sender, Receiver, bool);

/// One way the sender tries to the worker.
type Dial = Box<dyn FnOnce() -> std::io::Result<TcpStream> + Send>;

/// A caller to the worker still saying hello.
struct Greeter {
    id: u64,
    /// Where it came from, for one on this machine's own port.
    from: Option<IpAddr>,
    /// Its connection, to close when it takes longer than a hello does.
    socket: TcpStream,
    since: Instant,
}

/// One end of a job's link, shared by its threads.
struct End {
    job: Uuid,
    key: [u8; 32],
    hub: Arc<Hub>,
    events: Box<dyn Fn(LinkEvent) + Send + Sync>,
    stop: AtomicBool,
    next: AtomicU64,
    /// The connection the link runs on now, and its number, which tells it
    /// from one that has replaced it.
    current: Mutex<Option<(u64, TcpStream)>>,
    /// Held while a link starts or ends and says so, so that what is said
    /// comes in the order it happened.
    telling: Mutex<()>,
    /// What the sender holds, said on each connection. `None` on the worker.
    holds: Mutex<Option<Vec<Held>>>,
    /// Bounded by [`GREETING_ROOM`].
    greeters: Mutex<Vec<Greeter>>,
}

/// One end of a job's link. Dropping it stops it.
pub struct Link {
    end: Arc<End>,
    /// The port the worker listens on, when it does.
    port: Option<u16>,
}

fn spawn(name: &str, run: impl FnOnce() + Send + 'static) -> Result<(), ShareError> {
    thread::Builder::new()
        .name(name.into())
        .spawn(run)
        .map(drop)
        .map_err(ShareError::from)
}

impl Link {
    fn new(
        job: Uuid,
        key: [u8; 32],
        hub: Arc<Hub>,
        holds: Option<Vec<Held>>,
        events: Box<dyn Fn(LinkEvent) + Send + Sync>,
    ) -> Link {
        let end = Arc::new(End {
            job,
            key,
            hub,
            events,
            stop: AtomicBool::new(false),
            next: AtomicU64::new(1),
            current: Mutex::new(None),
            telling: Mutex::new(()),
            holds: Mutex::new(holds),
            greeters: Mutex::new(Vec::new()),
        });
        Link { end, port: None }
    }

    /// Where the worker listens, for the sender: this machine's address on
    /// its network and its own on the internet over IPv6, with the port.
    /// Empty on the sender, or when not listening.
    #[must_use]
    pub fn addresses(&self) -> Vec<SocketAddr> {
        let Some(port) = self.port else {
            return Vec::new();
        };
        // ponytail: the address the default route leaves from, as a host's
        // invite has. Every interface's, when std can list them.
        let here: Vec<IpAddr> = if cfg!(test) {
            vec![EVERY]
        } else {
            [Some(local_address()), global_address()]
                .into_iter()
                .flatten()
                .filter_map(|address| address.parse().ok())
                .collect()
        };
        here.into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect()
    }

    /// Ask the other end for `wanted`. Asked when there is a link, and
    /// again on the next one for as long as it has not arrived.
    pub fn want(&self, wanted: Wanted) {
        self.end.hub.want(wanted);
    }

    /// Stop taking `wanted`, at both ends. What has come of it is kept, to
    /// carry on from if it is wanted again.
    pub fn unwant(&self, wanted: &Wanted) {
        self.end.hub.unwant(wanted);
    }

    /// What the other end can give, as it last said. Empty on the sender.
    #[must_use]
    pub fn holders(&self, item: Uuid) -> Vec<(u32, u64)> {
        self.end.hub.holders(item)
    }

    /// Whether the other end is here now.
    #[must_use]
    pub fn linked(&self) -> bool {
        self.end.current.lock().is_some()
    }

    /// Change what this end holds, on the sender. Told to the worker now if
    /// it is here ([`Hello`] says how), and otherwise when it is next.
    pub fn set_holds(&self, holds: Vec<Held>) {
        let sender = self.end.holds.lock().as_mut().map(|held| *held = holds);
        if sender.is_some() {
            self.end.cut();
        }
    }

    /// Stop for good: every transfer, the link, and the threads that keep it.
    pub fn stop(&self) {
        let end = &self.end;
        end.stop.store(true, Ordering::Relaxed);
        end.hub.stop();
        end.cut();
        for greeter in end.greeters.lock().iter() {
            let _ = greeter.socket.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The worker's end of job `job`: listen on this machine's addresses when
/// `listen` is set, keep a room at `relay` when one is given, and take the
/// sender from either. Lumit's own relay is used only while its door is
/// open, and tried again when it opens later.
pub fn work(
    job: Uuid,
    key: [u8; 32],
    listen: bool,
    relay: Option<&str>,
    footage: Arc<dyn Footage>,
    limits: Arc<Limits>,
    events: Box<dyn Fn(LinkEvent) + Send + Sync>,
) -> Result<Link, ShareError> {
    // Made first, so that a thread that cannot be started stops the rest.
    let mut link = Link::new(job, key, Hub::patient(footage, limits), None, events);
    if listen {
        let (listeners, port) = host::listen(EVERY, 0)?;
        link.port = Some(port);
        let end = link.end.clone();
        spawn("lumit-share-remote-accept", move || end.accept(&listeners))?;
    }
    if let Some(relay) = relay {
        let (end, relay) = (link.end.clone(), relay.to_owned());
        spawn("lumit-share-remote-relay", move || end.relayed(&relay))?;
    }
    Ok(link)
}

/// The sender's end of job `job`: reach the worker at `addresses` and
/// through `relay` at once, and again whenever the link is down, until
/// stopped. `holds` are the files this end can give, told to the worker on
/// each link.
#[allow(clippy::too_many_arguments)]
pub fn send(
    job: Uuid,
    key: [u8; 32],
    addresses: Vec<SocketAddr>,
    relay: Option<&str>,
    holds: Vec<Held>,
    footage: Arc<dyn Footage>,
    limits: Arc<Limits>,
    events: Box<dyn Fn(LinkEvent) + Send + Sync>,
) -> Result<Link, ShareError> {
    let hub = Hub::new(footage, limits, false);
    let link = Link::new(job, key, hub, Some(holds), events);
    let (end, relay) = (link.end.clone(), relay.map(str::to_owned));
    spawn("lumit-share-remote-reach", move || {
        end.keep(&addresses, relay.as_deref());
    })?;
    Ok(link)
}

impl End {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Cut the link this end has now, if it has one.
    fn cut(&self) {
        if let Some((_, socket)) = self.current.lock().as_ref() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    /// Run bulk on a connection that has said hello, until it drops. `peer`
    /// is the other end's number to bulk.
    fn run(&self, peer: u32, socket: TcpStream, sender: Sender, receiver: Receiver, relayed: bool) {
        let Ok(kept) = socket.try_clone() else {
            return;
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        {
            let _telling = self.telling.lock();
            if self.stopped() {
                return;
            }
            *self.current.lock() = Some((id, kept));
            (self.events)(LinkEvent::Linked { relayed });
        }
        self.hub.link(peer, socket, sender, receiver);
        let _telling = self.telling.lock();
        let ended = {
            let mut current = self.current.lock();
            let mine = current.as_ref().is_some_and(|(now, _)| *now == id);
            if mine {
                *current = None;
            }
            mine
        };
        // A link that was replaced has nothing to say: the other end is here.
        if ended {
            (self.events)(LinkEvent::Unlinked);
        }
    }

    /// The worker's accepting thread.
    fn accept(self: &Arc<Self>, listeners: &[TcpListener]) {
        while !self.stopped() {
            let mut quiet = true;
            for (socket, from) in listeners.iter().filter_map(|l| l.accept().ok()) {
                quiet = false;
                let Some(id) = self.greet(&socket, Some(from.ip())) else {
                    continue;
                };
                let end = self.clone();
                let greeting = move || end.hello(socket, id, false);
                if spawn("lumit-share-remote-greet", greeting).is_err() {
                    self.greeters.lock().retain(|greeter| greeter.id != id);
                }
            }
            self.hurry();
            if quiet {
                thread::sleep(BEAT);
            }
        }
    }

    /// The worker's relay thread: keep a room at `relay` until stopped, and
    /// take whoever comes to it. One that does not answer, or Lumit's own
    /// with no door to it open, is asked again every few seconds.
    fn relayed(self: &Arc<Self>, relay: &str) {
        let name = room(&self.key);
        while !self.stopped() {
            if let Some(at) = dialled(relay) {
                if let Ok(mut kept) = lumit_relay::Room::open(at.as_str(), &name) {
                    while !self.stopped() {
                        match kept.guest(BEAT) {
                            Ok(Some(guest)) => self.take(&at, &name, kept.token(), guest),
                            Ok(None) => {}
                            Err(_) => break,
                        }
                        self.hurry();
                    }
                }
            }
            let asked = Instant::now();
            while !self.stopped() && asked.elapsed() < lumit_relay::PING {
                thread::sleep(BEAT);
            }
        }
    }

    /// Take the sender from the relay and greet it, on a thread of its own.
    fn take(self: &Arc<Self>, relay: &str, name: &str, token: &str, guest: u64) {
        let end = self.clone();
        let (relay, name, token) = (relay.to_owned(), name.to_owned(), token.to_owned());
        let _ = spawn("lumit-share-remote-greet", move || {
            let Ok(socket) = lumit_relay::take(relay.as_str(), &name, &token, guest) else {
                return;
            };
            if let Some(id) = end.greet(&socket, None) {
                end.hello(socket, id, true);
            }
        });
    }

    /// Give a caller a place among those saying hello. `None` when there is
    /// no room: too many are at it already, or too many from its address.
    fn greet(&self, socket: &TcpStream, from: Option<IpAddr>) -> Option<u64> {
        let socket = socket.try_clone().ok()?;
        let mut greeters = self.greeters.lock();
        let same = greeters
            .iter()
            .filter(|greeter| from.is_some() && greeter.from == from)
            .count();
        if greeters.len() >= GREETING_ROOM || same >= GREETING_EACH {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        greeters.push(Greeter {
            id,
            from,
            socket,
            since: Instant::now(),
        });
        Some(id)
    }

    /// Close every caller that has taken longer over its hello than a hello
    /// takes, which one sending a byte at a time would otherwise never do.
    fn hurry(&self) {
        let greeters = self.greeters.lock();
        for greeter in greeters.iter() {
            if greeter.since.elapsed() > wire::GREETING {
                let _ = greeter.socket.shutdown(Shutdown::Both);
            }
        }
    }

    /// On the worker: hear a caller's hello and, if it holds the key and is
    /// for this job, run the link with it until it drops. `greeter` is its
    /// place among those saying hello.
    fn hello(&self, socket: TcpStream, greeter: u64, relayed: bool) {
        let greeted = (|| {
            // An accepted socket takes after its listener on Windows.
            socket.set_nonblocking(false)?;
            let (mut sender, mut receiver) = wire::open(socket.try_clone()?, &self.key, false)?;
            // It holds the key, so it is no guesser to hurry: a long hello
            // takes as long as it takes to cross.
            self.greeters.lock().retain(|g| g.id != greeter);
            let hello: Hello = serde_json::from_slice(&receiver.recv(HELLO_LIMIT)?)?;
            if hello.version != VERSION || hello.job != self.job {
                return Err(ShareError::OutOfTurn);
            }
            let answer = Hello {
                version: VERSION,
                job: self.job,
                holds: Vec::new(),
            };
            sender.send(&serde_json::to_vec(&answer)?)?;
            Ok::<_, ShareError>((sender, receiver, hello.holds))
        })();
        // A caller that did not hold the key keeps its place a moment
        // longer, which is what makes guessing slow.
        if greeted.is_err() {
            thread::sleep(GUESS_PAUSE);
        }
        self.greeters.lock().retain(|g| g.id != greeter);
        let Ok((sender, receiver, holds)) = greeted else {
            return;
        };
        self.hub.holds(1, holds);
        self.run(1, socket, sender, receiver, relayed);
    }

    /// The sender's reaching thread: reach the worker whenever the link is
    /// down, until stopped.
    fn keep(&self, addresses: &[SocketAddr], relay: Option<&str>) {
        let mut wait = Duration::ZERO;
        loop {
            let until = Instant::now() + wait;
            while !self.stopped() && Instant::now() < until {
                thread::sleep(BEAT);
            }
            if self.stopped() {
                return;
            }
            let reached = self.reach(addresses, relay).and_then(|o| self.say_hello(o));
            let held = reached.is_some_and(|(socket, sender, receiver, relayed)| {
                let began = Instant::now();
                self.run(0, socket, sender, receiver, relayed);
                began.elapsed() >= STEADY
            });
            // A link that held is reached again at once. Anything else, and
            // the worker is waited for longer each time.
            wait = if held {
                Duration::ZERO
            } else {
                (wait * 2).clamp(FIRST_WAIT, LONGEST_WAIT)
            };
        }
    }

    /// Find the worker: every address at once, and the relay a moment later,
    /// a worker that can be reached itself being the better way to it. The
    /// first to prove it holds the key wins, and an address that leads to
    /// some other machine fails that and is said nothing more to. `None`
    /// when every try failed, or once stopped.
    fn reach(&self, addresses: &[SocketAddr], relay: Option<&str>) -> Option<Opened> {
        let mut places: Vec<(bool, Dial)> = addresses
            .iter()
            .take(MAX_ADDRESSES)
            .map(|at| {
                let at = *at;
                let dial: Dial = Box::new(move || TcpStream::connect_timeout(&at, wire::GREETING));
                (false, dial)
            })
            .collect();
        // Looked for each time, so a door to Lumit's own that opens later
        // is found.
        if let Some(relay) = relay.and_then(dialled) {
            let name = room(&self.key);
            places.push((
                true,
                Box::new(move || lumit_relay::join(relay.as_str(), &name)),
            ));
        }
        // Room for every answer, so a try that loses has nowhere to wait and
        // goes, closing its connection as it does.
        let (found, answers) = sync_channel(places.len());
        for (relayed, dial) in places {
            let (found, key) = (found.clone(), self.key);
            let _ = spawn("lumit-share-remote-try", move || {
                if relayed {
                    thread::sleep(HEAD_START);
                }
                let opened = dial().map_err(ShareError::from).and_then(|socket| {
                    let (sender, receiver) = wire::open(socket.try_clone()?, &key, true)?;
                    Ok((socket, sender, receiver, relayed))
                });
                if let Ok(opened) = opened {
                    let _ = found.try_send(opened);
                }
            });
        }
        drop(found);
        loop {
            match answers.recv_timeout(BEAT) {
                Ok(opened) => return Some(opened),
                Err(RecvTimeoutError::Timeout) if !self.stopped() => {}
                Err(_) => return None,
            }
        }
    }

    /// On the sender: say hello on a connection to the worker, and hear it
    /// let this end in.
    fn say_hello(&self, opened: Opened) -> Option<Opened> {
        let (socket, mut sender, mut receiver, relayed) = opened;
        let hello = Hello {
            version: VERSION,
            job: self.job,
            holds: self.holds.lock().clone().unwrap_or_default(),
        };
        sender.send(&serde_json::to_vec(&hello).ok()?).ok()?;
        let answer: Hello = serde_json::from_slice(&receiver.recv(HELLO_LIMIT).ok()?).ok()?;
        let ours = answer.version == VERSION && answer.job == self.job;
        ours.then_some((socket, sender, receiver, relayed))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
