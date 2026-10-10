//! A relay for shared projects, for when nobody's router will let the other
//! in: the host and its guests each connect out to it, and it passes what
//! one says on to the other.
//!
//! It is somebody's own server. Lumit's makers run none, and Lumit uses one
//! only when it is given its address. What crosses it is already encrypted
//! between the host and each guest with the invite's secret, which the relay
//! is never told, so the person who runs one sees who connected and how much
//! they said and nothing of the project.
//!
//! How it goes. A host keeps one connection open and names a room on it. A
//! guest connects and names the same room, and the host is told someone is
//! waiting. The host makes a second connection to take that guest, and from
//! then on the relay copies bytes between those two until either goes. Each
//! connection starts with one line of text saying which of the three it is,
//! and is answered with one.
//!
//! This is both ends of that: [`serve`] is the relay, and [`Room`], [`take`]
//! and [`join`] are what a host and a guest say to one. Threads: the relay
//! has one that accepts, one for each connection while it says which it is,
//! one for each host, and two for each guest being passed on.

use std::collections::HashMap;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The port a relay listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 47857;

/// Raised whenever a line changes shape.
const PROTOCOL: u32 = 1;

/// What every first line starts with.
const CALL: &str = "LUMIT-RELAY";

/// The longest line either end reads.
const LINE_LIMIT: usize = 128;

/// How long a connection has to say which it is, and how long a host or a
/// guest waits for its answer.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// How often a host says it is still there.
pub const PING: Duration = Duration::from_secs(5);

/// How long a host that has said nothing is believed. Three missed pings.
const QUIET: Duration = Duration::from_secs(20);

/// How long a guest waits to be taken before it is let go.
const WAITING: Duration = Duration::from_secs(15);

/// How long two ends being passed on may both say nothing. A shared project
/// pings every few seconds, so this is a connection that has died.
const IDLE: Duration = Duration::from_secs(60);

/// How often the accepting thread looks for a caller.
const BEAT: Duration = Duration::from_millis(50);

/// Whether `room` is a name a room may have. What a host and its guests
/// agree on without the relay being able to work the invite out from it.
#[must_use]
pub fn is_room(room: &str) -> bool {
    (16..=64).contains(&room.len()) && room.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read one line, a byte at a time so nothing after it is taken with it:
/// what follows the last line on a connection belongs to somebody else.
/// `line` holds what has come of it so far, which is how a wait that ran out
/// part-way through one is picked up again.
fn read_line(mut socket: &TcpStream, line: &mut Vec<u8>) -> io::Result<String> {
    let mut byte = [0u8; 1];
    loop {
        if socket.read(&mut byte)? == 0 {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        if byte[0] == b'\n' {
            let text = String::from_utf8_lossy(line).trim().to_owned();
            line.clear();
            return Ok(text);
        }
        if line.len() >= LINE_LIMIT {
            return Err(ErrorKind::InvalidData.into());
        }
        line.push(byte[0]);
    }
}

fn say(mut socket: &TcpStream, line: &str) -> io::Result<()> {
    socket.write_all(format!("{line}\n").as_bytes())
}

/// Connect to the first address `relay` names that answers.
fn dial(relay: impl ToSocketAddrs) -> io::Result<TcpStream> {
    let mut last = io::Error::from(ErrorKind::NotFound);
    for at in relay.to_socket_addrs()? {
        match TcpStream::connect_timeout(&at, PATIENCE) {
            Ok(socket) => {
                socket.set_nodelay(true)?;
                socket.set_read_timeout(Some(PATIENCE))?;
                socket.set_write_timeout(Some(PATIENCE))?;
                return Ok(socket);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Connect, say `line`, and hear the relay agree.
fn call(relay: impl ToSocketAddrs, line: &str) -> io::Result<TcpStream> {
    let socket = dial(relay)?;
    say(&socket, &format!("{CALL} {PROTOCOL} {line}"))?;
    match read_line(&socket, &mut Vec::new())?.as_str() {
        "OK" => Ok(socket),
        _ => Err(ErrorKind::ConnectionRefused.into()),
    }
}

/// A host's room at a relay, held open for as long as this is.
pub struct Room {
    control: TcpStream,
    line: Vec<u8>,
    said: Instant,
}

impl Room {
    /// Open `room` at `relay`. Refused when the relay is full, or the room
    /// is already somebody's.
    pub fn open(relay: impl ToSocketAddrs, room: &str) -> io::Result<Room> {
        let control = call(relay, &format!("HOST {room}"))?;
        Ok(Room {
            control,
            line: Vec::new(),
            said: Instant::now(),
        })
    }

    /// Wait up to `wait` for a guest, and answer the number to [`take`] it
    /// by, or `None` when nobody came. Also what tells the relay this host
    /// is still here, so it is called again and again. An error is the relay
    /// gone, and the room with it.
    pub fn guest(&mut self, wait: Duration) -> io::Result<Option<u64>> {
        if self.said.elapsed() >= PING {
            say(&self.control, "PING")?;
            self.said = Instant::now();
        }
        self.control.set_read_timeout(Some(wait))?;
        match read_line(&self.control, &mut self.line) {
            Ok(line) => Ok(line.strip_prefix("GUEST ").and_then(|n| n.parse().ok())),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// As the host of `room`, take the guest the relay numbered `guest`. What
/// comes back is a connection to that guest.
pub fn take(relay: impl ToSocketAddrs, room: &str, guest: u64) -> io::Result<TcpStream> {
    call(relay, &format!("TAKE {room} {guest}"))
}

/// As a guest, ask for the host of `room`. What comes back is a connection
/// to that host, once it has taken this guest.
pub fn join(relay: impl ToSocketAddrs, room: &str) -> io::Result<TcpStream> {
    call(relay, &format!("JOIN {room}"))
}

/// How much a relay takes on. What keeps a flood of strangers to a known
/// cost.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The most hosts at once.
    pub rooms: usize,
    /// The most guests waiting on one host to take them.
    pub waiting: usize,
    /// The most connections being greeted or passed on at once.
    pub connections: usize,
    /// How many bytes a second a host and a guest may pass each way once
    /// they have used up [`BURST`], or 0 for as many as the line carries.
    /// Edits are far under any limit worth setting. Footage is not, which
    /// is what this is for: a relay that should carry one and not the other.
    pub rate: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            rooms: 256,
            waiting: 8,
            connections: 1024,
            rate: 0,
        }
    }
}

struct Waiting {
    number: u64,
    socket: TcpStream,
    since: Instant,
}

struct Hosted {
    /// Which connection opened the room, so that one closing does not take
    /// away a room opened again since.
    opening: u64,
    control: TcpStream,
    waiting: Vec<Waiting>,
    next: u64,
}

struct Relay {
    limits: Limits,
    /// Every room, by name. Bounded by [`Limits::rooms`], and a room goes
    /// when its host does.
    rooms: Mutex<HashMap<String, Hosted>>,
    openings: AtomicUsize,
    connections: AtomicUsize,
}

impl Relay {
    /// The rooms. A thread that died holding them left them whole: nothing
    /// is half-done under this lock.
    fn rooms(&self) -> MutexGuard<'_, HashMap<String, Hosted>> {
        self.rooms.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A connection has said its first line. Do what it asked.
    fn greet(&self, socket: TcpStream) -> io::Result<()> {
        socket.set_nodelay(true)?;
        socket.set_read_timeout(Some(PATIENCE))?;
        socket.set_write_timeout(Some(PATIENCE))?;
        let line = read_line(&socket, &mut Vec::new())?;
        let mut words = line.split(' ');
        let called = words.next() == Some(CALL);
        if !called || words.next() != Some(PROTOCOL.to_string().as_str()) {
            return say(&socket, "NO");
        }
        let (what, room) = (words.next(), words.next().unwrap_or_default());
        if !is_room(room) {
            return say(&socket, "NO");
        }
        match what {
            Some("HOST") => self.host(socket, room),
            Some("JOIN") => self.park(socket, room),
            Some("TAKE") => match words.next().and_then(|n| n.parse().ok()) {
                Some(guest) => self.pass(socket, room, guest),
                None => say(&socket, "NO"),
            },
            _ => say(&socket, "NO"),
        }
    }

    /// Keep a room for a host until it goes quiet or goes.
    fn host(&self, socket: TcpStream, room: &str) -> io::Result<()> {
        let opening = self.openings.fetch_add(1, Ordering::Relaxed) as u64;
        {
            let mut rooms = self.rooms();
            if rooms.len() >= self.limits.rooms || rooms.contains_key(room) {
                drop(rooms);
                return say(&socket, "BUSY");
            }
            let hosted = Hosted {
                opening,
                control: socket.try_clone()?,
                waiting: Vec::new(),
                next: 1,
            };
            rooms.insert(room.to_owned(), hosted);
        }
        let kept = (|| {
            say(&socket, "OK")?;
            socket.set_read_timeout(Some(QUIET))?;
            let mut line = Vec::new();
            while read_line(&socket, &mut line)? == "PING" {
                // A guest nobody took has been waiting on a host that is
                // not going to.
                if let Some(hosted) = self.rooms().get_mut(room) {
                    hosted.waiting.retain(|w| w.since.elapsed() < WAITING);
                }
            }
            Ok(())
        })();
        let mut rooms = self.rooms();
        if rooms.get(room).is_some_and(|h| h.opening == opening) {
            rooms.remove(room);
        }
        kept
    }

    /// Hold a guest until the host of its room takes it, and tell that host.
    fn park(&self, socket: TcpStream, room: &str) -> io::Result<()> {
        let told = {
            let mut rooms = self.rooms();
            match rooms.get_mut(room) {
                Some(hosted) if hosted.waiting.len() < self.limits.waiting => {
                    let number = hosted.next;
                    hosted.next += 1;
                    hosted.waiting.push(Waiting {
                        number,
                        socket: socket.try_clone()?,
                        since: Instant::now(),
                    });
                    Some((hosted.control.try_clone()?, number))
                }
                _ => None,
            }
        };
        match told {
            // Said outside the lock. A host that cannot be told has gone,
            // which its own thread finds out.
            Some((control, number)) => {
                let _ = say(&control, &format!("GUEST {number}"));
                Ok(())
            }
            None => say(&socket, "NONE"),
        }
    }

    /// Join a host's second connection to the guest it asked for, and copy
    /// between them until either goes.
    fn pass(&self, host: TcpStream, room: &str, guest: u64) -> io::Result<()> {
        let waiting = {
            let mut rooms = self.rooms();
            let hosted = rooms.get_mut(room);
            hosted.and_then(|hosted| {
                let at = hosted.waiting.iter().position(|w| w.number == guest)?;
                Some(hosted.waiting.remove(at))
            })
        };
        let Some(Waiting { socket: guest, .. }) = waiting else {
            return say(&host, "NONE");
        };
        say(&guest, "OK")?;
        say(&host, "OK")?;
        host.set_read_timeout(Some(IDLE))?;
        guest.set_read_timeout(Some(IDLE))?;
        let (from_guest, to_host) = (guest.try_clone()?, host.try_clone()?);
        let rate = self.limits.rate;
        let back = thread::Builder::new()
            .name("lumit-relay-pass".into())
            .spawn(move || copy(&from_guest, &to_host, rate))?;
        copy(&host, &guest, rate);
        let _ = back.join();
        Ok(())
    }
}

/// How much passes each way at full speed before a [`Limits::rate`] holds:
/// room for a whole project to be sent to someone joining.
pub const BURST: u64 = 16 << 20;

/// Copy what `from` says to `to` until either goes, then close both, which
/// is what ends the copy going the other way. No faster than `rate` bytes a
/// second once [`BURST`] has gone, with nought for no limit.
fn copy(mut from: &TcpStream, mut to: &TcpStream, rate: u64) {
    let mut bytes = [0u8; 16 << 10];
    // What may still pass at once, topped up at the rate as time goes by.
    let (mut allowance, mut topped) = (BURST as f64, Instant::now());
    while let Ok(n @ 1..) = from.read(&mut bytes) {
        if to.write_all(&bytes[..n]).is_err() {
            break;
        }
        if rate == 0 {
            continue;
        }
        let now = Instant::now();
        let earned = now.duration_since(topped).as_secs_f64() * rate as f64;
        allowance = (allowance + earned).min(BURST as f64) - n as f64;
        topped = now;
        if allowance < 0.0 {
            thread::sleep(Duration::from_secs_f64(-allowance / rate as f64));
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

/// Be a relay on `listeners` until `stop` is set. Does not return before.
/// More than one listener is for a system that keeps IPv4 and IPv6 apart.
pub fn serve(listeners: &[TcpListener], limits: Limits, stop: &AtomicBool) -> io::Result<()> {
    for listener in listeners {
        listener.set_nonblocking(true)?;
    }
    let relay = Arc::new(Relay {
        limits,
        rooms: Mutex::new(HashMap::new()),
        openings: AtomicUsize::new(0),
        connections: AtomicUsize::new(0),
    });
    while !stop.load(Ordering::Relaxed) {
        let Some((socket, _)) = listeners.iter().find_map(|l| l.accept().ok()) else {
            thread::sleep(BEAT);
            continue;
        };
        if relay.connections.fetch_add(1, Ordering::Relaxed) >= limits.connections {
            relay.connections.fetch_sub(1, Ordering::Relaxed);
            continue;
        }
        let greeting = relay.clone();
        let spawned = thread::Builder::new()
            .name("lumit-relay-greet".into())
            .spawn(move || {
                // An accepted socket takes after its listener on Windows.
                if socket.set_nonblocking(false).is_ok() {
                    let _ = greeting.greet(socket);
                }
                greeting.connections.fetch_sub(1, Ordering::Relaxed);
            });
        if spawned.is_err() {
            relay.connections.fetch_sub(1, Ordering::Relaxed);
        }
    }
    // Every host is let go of, and the guests waiting on one with it. Two
    // ends being passed on carry on until either goes.
    for (_, hosted) in relay.rooms().drain() {
        let _ = hosted.control.shutdown(Shutdown::Both);
    }
    Ok(())
}
